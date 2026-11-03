//! Program keys (docs/spec.md §11.1–§11.2).
//!
//! In the TEE one OutLayer encryption key backs every program key. The verifier's `outlayer.manifest`
//! section declares `{"path": "antibody", "bind": "wasm", "caller": "predecessor"}`, so OutLayer's
//! keystore derives it from this build's hash and the account that called OutLayer: only this build,
//! called by near-antibody, ever holds it. The guest never sees it; it asks the host for
//! `mac(path, data)`. Natively, and in the local wasm runner, a TEST root key stands in for the host.
//!
//! Every derivation is domain-separated: `label ‖ 0x00 ‖ u32_be(len(program_key)) ‖ program_key ‖ …`.

use x25519_dalek::{PublicKey, StaticSecret};

use crate::facts::{hmac_sha256, sha256};

/// The encryption key's path in the manifest section.
pub const KEY_PATH: &str = "antibody";

pub trait Mac {
    fn mac(&self, data: &[u8]) -> Result<[u8; 32], String>;
}

/// TEST-ONLY stand-in for OutLayer's host key: HMAC-SHA256 under a root key.
pub struct TestMac(pub [u8; 32]);

impl Mac for TestMac {
    fn mac(&self, data: &[u8]) -> Result<[u8; 32], String> {
        Ok(hmac_sha256(&self.0, data))
    }
}

/// OutLayer's `outlayer:encryption-keys/api.mac`, inside the TEE.
#[cfg(target_os = "wasi")]
pub struct HostMac;

#[cfg(target_os = "wasi")]
mod bindings {
    wit_bindgen::generate!({
        world: "antibody-verifier",
        path: "wit",
        with: { "outlayer:encryption-keys/api@0.1.0": generate },
    });
}

#[cfg(target_os = "wasi")]
impl Mac for HostMac {
    fn mac(&self, data: &[u8]) -> Result<[u8; 32], String> {
        let out = bindings::outlayer::encryption_keys::api::mac(KEY_PATH, None, data)?;
        <[u8; 32]>::try_from(out.as_slice()).map_err(|_| "the host's mac is not 32 bytes".to_string())
    }
}

fn label(l: &str, program_key: &str, rest: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(l.len() + 5 + program_key.len() + rest.len());
    m.extend_from_slice(l.as_bytes());
    m.push(0);
    m.extend_from_slice(&(program_key.len() as u32).to_be_bytes());
    m.extend_from_slice(program_key.as_bytes());
    m.extend_from_slice(rest);
    m
}

/// The keys of one program.
pub struct ProgramKeys<'a> {
    mac: &'a dyn Mac,
    program_key: &'a str,
}

impl<'a> ProgramKeys<'a> {
    pub fn new(mac: &'a dyn Mac, program_key: &'a str) -> Self {
        ProgramKeys { mac, program_key }
    }

    /// The program's sealing secret (X25519, clamped by `StaticSecret`).
    pub fn seal_secret(&self) -> Result<StaticSecret, String> {
        Ok(StaticSecret::from(self.mac.mac(&label("antibody/v2/seal", self.program_key, &[]))?))
    }

    pub fn seal_public(&self) -> Result<[u8; 32], String> {
        Ok(PublicKey::from(&self.seal_secret()?).to_bytes())
    }

    /// `hex(sha256(seal_pk)[0..8])`: changes when the key changes.
    pub fn key_epoch(&self) -> Result<String, String> {
        Ok(hex::encode(&sha256(&self.seal_public()?)[..8]))
    }

    /// A root-cause key's tag: opaque until disclosure.
    pub fn tag(&self, slot: u8, group: &[u8]) -> Result<[u8; 32], String> {
        let mut rest = vec![slot];
        rest.extend_from_slice(group);
        self.mac.mac(&label("antibody/v2/tag", self.program_key, &rest))
    }

    /// The disclosure key of a tag in a round: opens every PoC committed in that round whose primary tag
    /// it is. A later round's proof of the same tag needs its own key.
    pub fn disclose(&self, tag: &[u8; 32], round: u32) -> Result<[u8; 32], String> {
        let mut rest = tag.to_vec();
        rest.extend_from_slice(&round.to_be_bytes());
        self.mac.mac(&label("antibody/v2/disclose", self.program_key, &rest))
    }
}

/// The group bytes of a root cause (spec §7.2): a committed group's id, or the target and selector.
pub fn group_bytes(custom: Option<u16>, target: &[u8; 20], selector: &[u8; 4]) -> Vec<u8> {
    match custom {
        Some(id) => {
            let mut v = vec![b'g'];
            v.extend_from_slice(&id.to_be_bytes());
            v
        }
        None => {
            let mut v = vec![b'f'];
            v.extend_from_slice(target);
            v.extend_from_slice(selector);
            v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_separated_and_stable() {
        let m = TestMac([7; 32]);
        let a = ProgramKeys::new(&m, "antibody.testnet/0");
        let b = ProgramKeys::new(&m, "antibody.testnet/1");
        assert_ne!(a.seal_public().unwrap(), b.seal_public().unwrap());
        assert_eq!(a.key_epoch().unwrap().len(), 16);
        let t = a.tag(0, &group_bytes(None, &[1; 20], &[2; 4])).unwrap();
        assert_ne!(t, a.disclose(&t, 0).unwrap());
        assert_ne!(a.disclose(&t, 0).unwrap(), a.disclose(&t, 1).unwrap(), "one disclosure key per round");
        assert_ne!(t, a.tag(1, &group_bytes(None, &[1; 20], &[2; 4])).unwrap());
        assert_eq!(t, ProgramKeys::new(&m, "antibody.testnet/0").tag(0, &group_bytes(None, &[1; 20], &[2; 4])).unwrap());
    }
}
