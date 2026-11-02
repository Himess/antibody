//! Lint of a program's invariants code (docs/spec.md §5.4): no opcode whose result depends on the
//! environment rather than on the protocol's state, and none that creates or destroys code.
//! `GAS` is allowed: Solidity forwards it on every external call.

pub const FORBIDDEN: &[(u8, &str)] = &[
    (0x32, "ORIGIN"),
    (0x3a, "GASPRICE"),
    (0x40, "BLOCKHASH"),
    (0x41, "COINBASE"),
    (0x44, "PREVRANDAO"),
    (0x49, "BLOBHASH"),
    (0x4a, "BLOBBASEFEE"),
    (0xf0, "CREATE"),
    (0xf2, "CALLCODE"),
    (0xf5, "CREATE2"),
    (0xff, "SELFDESTRUCT"),
];

/// Solidity appends CBOR metadata (a map) and its length in the last two bytes. Its bytes are data,
/// not code.
fn strip_metadata(code: &[u8]) -> &[u8] {
    if code.len() < 3 {
        return code;
    }
    let n = u16::from_be_bytes([code[code.len() - 2], code[code.len() - 1]]) as usize;
    if n + 2 <= code.len() && n >= 1 {
        let start = code.len() - 2 - n;
        if matches!(code[start], 0xa1..=0xa6) {
            return &code[..start];
        }
    }
    code
}

pub fn lint(code: &[u8]) -> Result<(), String> {
    let code = strip_metadata(code);
    let mut i = 0usize;
    while i < code.len() {
        let op = code[i];
        if let Some((_, name)) = FORBIDDEN.iter().find(|(o, _)| *o == op) {
            return Err(format!("the invariants use {name} at byte {i}"));
        }
        if (0x60..=0x7f).contains(&op) {
            i += (op - 0x5f) as usize;
        }
        i += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_data_is_skipped_and_forbidden_opcodes_found() {
        // PUSH1 0x40 (the byte 0x40 is data, not BLOCKHASH), then STOP.
        assert!(lint(&[0x60, 0x40, 0x00]).is_ok());
        assert!(lint(&[0x60, 0x01, 0x40]).is_err());
        assert!(lint(&[0x5a, 0x00]).is_ok(), "GAS is allowed");
        assert!(lint(&[0xf0]).is_err());
    }
}
