//! Sealed-blob locations backed by NEAR transactions (FastFS, docs/crypto-spec.md §4.1).
//!
//! A blob uploaded with FastFS lives in the arguments of a `__fastdata_fastfs` function call. The
//! FastFS gateway serves it over https; when the gateway is unavailable the same bytes can be read
//! from any NEAR RPC node with `near-tx:<tx hash>:<signer account>`. Either way the reader checks
//! the blob's sha256 against the commitment, so the gateway and the RPC node are untrusted.

use base64::Engine as _;

pub const SCHEME: &str = "near-tx:";

/// Parses `near-tx:<base58 tx hash>:<signer account>`.
pub fn parse_location(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix(SCHEME)?;
    let (hash, signer) = rest.split_once(':')?;
    (!hash.is_empty() && !signer.is_empty()).then_some((hash, signer))
}

/// JSON-RPC body that fetches the transaction (archival nodes keep old ones).
pub fn tx_request(hash: &str, signer: &str) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": "antibody", "method": "tx",
                       "params": {"tx_hash": hash, "sender_account_id": signer, "wait_until": "INCLUDED_FINAL"}})
}

/// Archival RPCs of two operators for a NEAR network id, tried in order (docs/spec.md §4.4).
pub fn archival_rpcs(network: &str) -> [&'static str; 2] {
    if network == "mainnet" {
        ["https://archival-rpc.mainnet.fastnear.com", "https://archival-rpc.mainnet.near.org"]
    } else {
        ["https://archival-rpc.testnet.fastnear.com", "https://archival-rpc.testnet.near.org"]
    }
}

/// Default archival RPC for a NEAR network id (`NEAR_NETWORK_ID` inside OutLayer).
pub fn archival_rpc(network: &str) -> &'static str {
    if network == "mainnet" {
        "https://archival-rpc.mainnet.fastnear.com"
    } else {
        "https://archival-rpc.testnet.fastnear.com"
    }
}

/// Extracts the uploaded file from a `tx` RPC response: the first `__fastdata_fastfs` call's
/// borsh `FastfsData::Simple { relative_path, content: Some { mime_type, content } }`.
pub fn content_from_tx_response(resp: &serde_json::Value) -> Result<Vec<u8>, String> {
    let actions = resp["result"]["transaction"]["actions"].as_array().ok_or("no transaction in RPC response")?;
    for a in actions {
        let fc = &a["FunctionCall"];
        if fc["method_name"].as_str() == Some("__fastdata_fastfs") {
            let args = base64::engine::general_purpose::STANDARD
                .decode(fc["args"].as_str().ok_or("no args")?)
                .map_err(|e| format!("args base64: {e}"))?;
            return simple_content(&args);
        }
    }
    Err("no __fastdata_fastfs call in the transaction".into())
}

/// Borsh `FastfsData::Simple` → file content.
pub fn simple_content(args: &[u8]) -> Result<Vec<u8>, String> {
    let mut i = 0usize;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let s = args.get(i..i + n).ok_or("truncated FastFS data")?;
        i += n;
        Ok(s)
    };
    if take(1)?[0] != 0 {
        return Err("only FastfsData::Simple uploads are supported (<= 1 MiB)".into());
    }
    let pl = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
    take(pl)?;
    if take(1)?[0] != 1 {
        return Err("FastFS upload has no content".into());
    }
    let ml = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
    take(ml)?;
    let cl = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
    Ok(take(cl)?.to_vec())
}

/// Borsh `FastfsData::Simple` for an upload (used by the CLI and the relayer).
pub fn simple_args(relative_path: &str, mime_type: &str, content: &[u8]) -> Vec<u8> {
    let mut o = vec![0u8];
    o.extend((relative_path.len() as u32).to_le_bytes());
    o.extend(relative_path.as_bytes());
    o.push(1);
    o.extend((mime_type.len() as u32).to_le_bytes());
    o.extend(mime_type.as_bytes());
    o.extend((content.len() as u32).to_le_bytes());
    o.extend(content);
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_location() {
        let args = simple_args("antibody/blob.bin", "application/octet-stream", b"hello");
        assert_eq!(simple_content(&args).unwrap(), b"hello");
        let resp = serde_json::json!({"result": {"transaction": {"actions": [
            {"FunctionCall": {"method_name": "__fastdata_fastfs", "args": base64::engine::general_purpose::STANDARD.encode(&args)}}]}}});
        assert_eq!(content_from_tx_response(&resp).unwrap(), b"hello");
        assert_eq!(parse_location("near-tx:9xYz:alice.testnet"), Some(("9xYz", "alice.testnet")));
        assert_eq!(parse_location("https://x"), None);
    }
}
