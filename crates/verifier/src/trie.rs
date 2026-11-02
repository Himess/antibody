//! Canonical tries (docs/spec.md §5.7) and merging a pinned base witness with a PoC's extra witness
//! (§4.4).
//!
//! rsp's partial trie stores each node's kind as given, and its hash covers only the node's RLP. A leaf
//! whose value is 32 bytes and an extension pointing at a hashed branch encode to the same bytes, and
//! lookups never check the hex-prefix flag that tells them apart. A witness could therefore relabel an
//! extension as a leaf, keep the state root, and make every key below it read as absent. `validate`
//! refuses any node whose flag does not match its kind, and any non-canonical shape.
//!
//! The node types below mirror `rsp_mpt::MptNode` field for field and variant for variant, so a trie
//! transcodes between the two through bincode. rsp keeps its node enum private.

use alloy_primitives::{map::HashMap, B256};
use rsp_mpt::{EthereumState, MptNode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    data: Data,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Data {
    Null,
    Branch([Option<Box<Node>>; 16]),
    Leaf(Vec<u8>, Vec<u8>),
    Extension(Vec<u8>, Box<Node>),
    Digest(B256),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct State {
    state_trie: Node,
    storage_tries: HashMap<B256, Node>,
}

fn to_mirror<T: Serialize, U: for<'de> Deserialize<'de>>(v: &T) -> Result<U, String> {
    let bytes = bincode::serialize(v).map_err(|e| format!("trie encode: {e}"))?;
    bincode::deserialize(&bytes).map_err(|e| format!("trie transcode: {e}"))
}

/// Depth bound: an account path is 64 nibbles, so no canonical trie is deeper.
const MAX_DEPTH: usize = 70;

fn check_prefix(p: &[u8], leaf: bool) -> Result<(), String> {
    let first = *p.first().ok_or("empty path in a trie node")?;
    let flag = first >> 4;
    let ok_kind = if leaf { flag == 2 || flag == 3 } else { flag == 0 || flag == 1 };
    if !ok_kind {
        return Err(format!("a {} carries the path flag {flag}", if leaf { "leaf" } else { "extension" }));
    }
    if flag % 2 == 0 && first & 0x0f != 0 {
        return Err("an even path has a non-zero padding nibble".into());
    }
    if !leaf && flag == 0 && p.len() == 1 {
        return Err("an extension with an empty path".into());
    }
    Ok(())
}

fn validate_node(n: &Node, depth: usize, root: bool) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("trie too deep".into());
    }
    match &n.data {
        Data::Null => {
            if root {
                Ok(())
            } else {
                Err("an empty node inside a trie".into())
            }
        }
        Data::Digest(_) => Ok(()),
        Data::Leaf(p, v) => {
            check_prefix(p, true)?;
            if v.is_empty() {
                return Err("a leaf without a value".into());
            }
            Ok(())
        }
        Data::Extension(p, child) => {
            check_prefix(p, false)?;
            match &child.data {
                Data::Branch(_) | Data::Digest(_) => validate_node(child, depth + 1, false),
                _ => Err("an extension must point at a branch".into()),
            }
        }
        Data::Branch(children) => {
            let present = children.iter().filter(|c| c.is_some()).count();
            if present < 2 {
                return Err("a branch with fewer than two children".into());
            }
            for c in children.iter().flatten() {
                validate_node(c, depth + 1, false)?;
            }
            Ok(())
        }
    }
}

/// Refuses any non-canonical node in the state trie or a storage trie.
pub fn validate(state: &EthereumState) -> Result<(), String> {
    let s: State = to_mirror(state)?;
    validate_node(&s.state_trie, 0, true).map_err(|e| format!("state trie: {e}"))?;
    for (k, t) in &s.storage_tries {
        validate_node(t, 0, true).map_err(|e| format!("storage trie {k}: {e}"))?;
    }
    Ok(())
}

fn hash_of(n: &Node) -> Result<B256, String> {
    let m: MptNode = to_mirror(n)?;
    Ok(m.hash())
}

fn merge_node(a: &Node, b: &Node, depth: usize) -> Result<Node, String> {
    if depth > MAX_DEPTH {
        return Err("trie too deep".into());
    }
    match (&a.data, &b.data) {
        (Data::Digest(x), Data::Digest(y)) => {
            if x == y {
                Ok(a.clone())
            } else {
                Err("the witnesses disagree on a subtree".into())
            }
        }
        (Data::Digest(h), _) => {
            if hash_of(b)? == *h {
                Ok(b.clone())
            } else {
                Err("an expanded subtree does not match its hash".into())
            }
        }
        (_, Data::Digest(h)) => {
            if hash_of(a)? == *h {
                Ok(a.clone())
            } else {
                Err("an expanded subtree does not match its hash".into())
            }
        }
        (Data::Null, Data::Null) => Ok(a.clone()),
        (Data::Leaf(p1, v1), Data::Leaf(p2, v2)) => {
            if p1 == p2 && v1 == v2 {
                Ok(a.clone())
            } else {
                Err("the witnesses disagree on a leaf".into())
            }
        }
        (Data::Extension(p1, c1), Data::Extension(p2, c2)) => {
            if p1 != p2 {
                return Err("the witnesses disagree on an extension".into());
            }
            Ok(Node { data: Data::Extension(p1.clone(), Box::new(merge_node(c1, c2, depth + 1)?)) })
        }
        (Data::Branch(x), Data::Branch(y)) => {
            let mut out: [Option<Box<Node>>; 16] = Default::default();
            for i in 0..16 {
                out[i] = match (&x[i], &y[i]) {
                    (None, None) => None,
                    (Some(c1), Some(c2)) => Some(Box::new(merge_node(c1, c2, depth + 1)?)),
                    _ => return Err("the witnesses disagree on a branch".into()),
                };
            }
            Ok(Node { data: Data::Branch(out) })
        }
        _ => Err("the witnesses disagree on a node kind".into()),
    }
}

/// The partial trie of `root` that proves nothing: the root's digest alone. Merged with a base it adds
/// nothing, so it is the state of a submission whose proof reads only what the base proves.
pub fn digest_only(root: B256) -> Result<EthereumState, String> {
    to_mirror(&State { state_trie: Node { data: Data::Digest(root) }, storage_tries: HashMap::default() })
}

/// Merges two partial tries of the same state: every node is the expanded one where either side has
/// it, and every expansion is checked against the hash it replaces.
pub fn merge(base: &EthereumState, extra: &EthereumState) -> Result<EthereumState, String> {
    let a: State = to_mirror(base)?;
    let b: State = to_mirror(extra)?;
    let state_trie = merge_node(&a.state_trie, &b.state_trie, 0)?;
    let mut storage_tries = a.storage_tries.clone();
    for (k, t) in b.storage_tries {
        let merged = match a.storage_tries.get(&k) {
            Some(x) => merge_node(x, &t, 0)?,
            None => t,
        };
        storage_tries.insert(k, merged);
    }
    to_mirror(&State { state_trie, storage_tries })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(path: &[u8], v: &[u8]) -> Node {
        Node { data: Data::Leaf(path.to_vec(), v.to_vec()) }
    }

    #[test]
    fn a_digest_only_trie_merges_to_the_base() {
        let base = to_mirror::<State, EthereumState>(&State { state_trie: leaf(&[0x20, 0xab], &[1]), storage_tries: HashMap::default() }).unwrap();
        let root = base.state_root();
        let merged = merge(&base, &digest_only(root).unwrap()).unwrap();
        assert_eq!(merged.state_root(), root);
        assert_eq!(bincode::serialize(&merged).unwrap(), bincode::serialize(&base).unwrap());
    }

    #[test]
    fn flags_must_match_kinds() {
        assert!(check_prefix(&[0x20, 0xab], true).is_ok());
        assert!(check_prefix(&[0x3a, 0xbc], true).is_ok());
        assert!(check_prefix(&[0x00, 0xab], true).is_err(), "an extension flag on a leaf");
        assert!(check_prefix(&[0x1a], false).is_ok());
        assert!(check_prefix(&[0x20, 0xab], false).is_err(), "a leaf flag on an extension");
        assert!(check_prefix(&[0x2f, 0xab], true).is_err(), "even path with padding");
    }

    #[test]
    fn relabelled_extension_is_refused() {
        // An extension to a hashed branch and a leaf with that hash as value encode alike.
        let ext_as_leaf = leaf(&[0x00, 0x12], &[0xaa; 32]);
        assert!(validate_node(&ext_as_leaf, 0, true).is_err());
    }

    #[test]
    fn merge_expands_digests_and_checks_them() {
        // Long enough (encoding >= 32 bytes) to be referenced by its hash, as in a real trie.
        let l = leaf(&[0x20, 0x01], &[0x5a; 40]);
        let h = hash_of(&l).unwrap();
        let d = Node { data: Data::Digest(h) };
        let mut kids: [Option<Box<Node>>; 16] = Default::default();
        kids[1] = Some(Box::new(d.clone()));
        kids[2] = Some(Box::new(Node { data: Data::Digest(B256::repeat_byte(7)) }));
        let a = Node { data: Data::Branch(kids.clone()) };
        let mut kids2 = kids;
        kids2[1] = Some(Box::new(l.clone()));
        let b = Node { data: Data::Branch(kids2) };
        let m = merge_node(&a, &b, 0).unwrap();
        assert_eq!(hash_of(&m).unwrap(), hash_of(&a).unwrap());
        let bad = leaf(&[0x20, 0x02], &[0x5b; 40]);
        let mut kids3: [Option<Box<Node>>; 16] = Default::default();
        kids3[1] = Some(Box::new(bad));
        kids3[2] = Some(Box::new(Node { data: Data::Digest(B256::repeat_byte(7)) }));
        assert!(merge_node(&a, &Node { data: Data::Branch(kids3) }, 0).is_err());
    }
}
