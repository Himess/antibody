//! Account ids, amounts and small encodings shared by the engine.

use sha2::{Digest, Sha256};

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// NEAR's account id rules: 2 to 64 characters of `a-z`, `0-9` and the separators `.`, `-`, `_`, with no
/// separator first, last, or next to another.
pub fn is_valid_account_id(id: &str) -> bool {
    let b = id.as_bytes();
    if b.len() < 2 || b.len() > 64 {
        return false;
    }
    let mut last_was_separator = true;
    for &c in b {
        if c == b'.' || c == b'-' || c == b'_' {
            if last_was_separator {
                return false;
            }
            last_was_separator = true;
        } else if c.is_ascii_lowercase() || c.is_ascii_digit() {
            last_was_separator = false;
        } else {
            return false;
        }
    }
    !last_was_separator
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountType {
    Named,
    /// 64 lowercase hex characters: an ed25519 public key.
    NearImplicit,
    /// `0x` and 40 lowercase hex characters.
    EthImplicit,
}

pub fn account_type(id: &str) -> AccountType {
    let hex = |s: &str| s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c));
    if id.len() == 64 && hex(id) {
        AccountType::NearImplicit
    } else if id.len() == 42 && id.starts_with("0x") && hex(&id[2..]) {
        AccountType::EthImplicit
    } else {
        AccountType::Named
    }
}

pub fn is_top_level(id: &str) -> bool {
    !id.contains('.')
}

/// `id` is a direct sub-account of `parent` (`x.parent`, with no dot in `x`).
pub fn is_sub_account_of(id: &str, parent: &str) -> bool {
    id.strip_suffix(parent).and_then(|s| s.strip_suffix('.')).is_some_and(|s| !s.is_empty() && !s.contains('.'))
}

/// A decimal amount in yoctoNEAR (or raw token units).
pub fn parse_u128(s: &str, what: &str) -> Result<u128, String> {
    if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
        return Err(format!("bad {what} {s:?}"));
    }
    s.parse::<u128>().map_err(|_| format!("bad {what} {s:?}"))
}

/// A non-negative decimal USD string ("2000", "1.5") in micro-USD, as v1 parses committed prices.
pub fn usd_to_micro(s: &str) -> Result<u128, String> {
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if int.is_empty() || frac.len() > 6 || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return Err(format!("bad USD amount {s:?}"));
    }
    let int: u128 = int.parse().map_err(|_| format!("bad USD amount {s:?}"))?;
    let frac: u128 = format!("{frac:0<6}").parse().map_err(|_| format!("bad USD amount {s:?}"))?;
    int.checked_mul(1_000_000).and_then(|v| v.checked_add(frac)).ok_or_else(|| format!("bad USD amount {s:?}"))
}

/// `floor(a * b / d)` without overflow in the product (256-bit intermediate); saturates at `u128::MAX`.
pub fn mul_div(a: u128, b: u128, d: u128) -> u128 {
    assert!(d != 0, "mul_div by zero");
    // 128 x 128 -> 256 bits, in 64-bit limbs.
    let (a1, a0) = (a >> 64, a & u64::MAX as u128);
    let (b1, b0) = (b >> 64, b & u64::MAX as u128);
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 64) + (p01 & u64::MAX as u128) + (p10 & u64::MAX as u128);
    let lo = (p00 & u64::MAX as u128) | (mid << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    if hi >= d {
        return u128::MAX;
    }
    // Long division of (hi, lo) by d, one bit at a time (hi < d keeps the quotient in 128 bits).
    let (mut rem, mut q) = (hi, 0u128);
    for i in (0..128).rev() {
        let carry = rem >> 127;
        rem = (rem << 1) | ((lo >> i) & 1);
        if carry == 1 || rem >= d {
            rem = rem.wrapping_sub(d);
            q |= 1u128 << i;
        }
    }
    q
}

/// A signed amount as a string: `-123` or `123`.
pub fn signed(neg: bool, mag: u128) -> String {
    if neg && mag != 0 {
        format!("-{mag}")
    } else {
        mag.to_string()
    }
}

/// Micro-dollars as a decimal string with 2 decimals (display only), as v1 formats them.
pub fn fmt_usd(neg: bool, micro: u128) -> String {
    let cents = micro / 10_000;
    let s = format!("{}.{:02}", cents / 100, cents % 100);
    if neg && micro != 0 {
        format!("-{s}")
    } else {
        s
    }
}

pub fn b64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

pub fn unb64(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).map_err(|_| "not base64".to_string())
}

pub fn hex32(s: &str) -> Option<[u8; 32]> {
    let b = hex::decode(s.trim_start_matches("0x")).ok()?;
    b.try_into().ok()
}

/// Lowercase hex of exactly 32 bytes (a committed sha256).
pub fn is_hex32_lower(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_ids() {
        for ok in ["ab", "alice.near", "a-b_c.d", "0x1234567890123456789012345678901234567890", &"a".repeat(64)] {
            assert!(is_valid_account_id(ok), "{ok}");
        }
        for bad in ["a", "A.near", ".a", "a.", "a..b", "a-.b", "a b", &"a".repeat(65)] {
            assert!(!is_valid_account_id(bad), "{bad}");
        }
        assert!(is_sub_account_of("x.alice.near", "alice.near"));
        assert!(!is_sub_account_of("y.x.alice.near", "alice.near"));
        assert!(!is_sub_account_of("alice.near", "alice.near"));
        assert_eq!(account_type(&"ab".repeat(32)), AccountType::NearImplicit);
        assert_eq!(account_type("0x1234567890abcdef1234567890abcdef12345678"), AccountType::EthImplicit);
        assert_eq!(account_type("alice.near"), AccountType::Named);
    }

    #[test]
    fn mul_div_is_exact() {
        assert_eq!(mul_div(10, 20, 3), 66);
        assert_eq!(mul_div(u128::MAX, 2, 4), u128::MAX / 2);
        assert_eq!(mul_div(10u128.pow(30), 3_000_000, 10u128.pow(24)), 3_000_000_000_000);
        assert_eq!(mul_div(u128::MAX, u128::MAX, 1), u128::MAX, "saturates");
        assert_eq!(usd_to_micro("1.5").unwrap(), 1_500_000);
        assert_eq!(fmt_usd(true, 1_234_567), "-1.23");
    }
}
