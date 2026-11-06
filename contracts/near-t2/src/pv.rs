//! The public values a proof commits: layout v1 of `antibody_zk_core::Rerun::public_values` (integers
//! big-endian). `u8 version | request_sha256 | receipt_sha256 | blob_sha256 | base_sha256 |
//! manifest_sha256 | anchor block hash (32 bytes each) | u32 round | u64 commit | u64 run | u8 kind |
//! u16 len, program key | u16 len, verifier | u8 n, n × (u8 slot, u8 severity)`.

pub const V1: u8 = 1;
/// A known issue is verified like a submission and names no commit.
pub const KNOWN_ISSUE_COMMIT: u64 = u64::MAX;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicValues {
    pub request_sha256: [u8; 32],
    pub receipt_sha256: [u8; 32],
    pub blob_sha256: [u8; 32],
    pub base_sha256: [u8; 32],
    pub manifest_sha256: [u8; 32],
    pub anchor: [u8; 32],
    pub round: u32,
    pub commit_id: u64,
    pub run_id: u64,
    pub kind: u8,
    pub program_key: String,
    pub verifier: String,
    /// (slot, severity 0 low … 3 critical) of each broken invariant, in the receipt's order.
    pub broken: Vec<(u8, u8)>,
}

pub fn decode(b: &[u8]) -> Result<PublicValues, &'static str> {
    let mut i = 0usize;
    let mut take = |n: usize| -> Result<&[u8], &'static str> {
        let s = b.get(i..i + n).ok_or("public values: truncated")?;
        i += n;
        Ok(s)
    };
    if take(1)?[0] != V1 {
        return Err("public values: not layout v1");
    }
    let mut h = || -> Result<[u8; 32], &'static str> { Ok(take(32)?.try_into().unwrap()) };
    let (request_sha256, receipt_sha256, blob_sha256, base_sha256, manifest_sha256, anchor) = (h()?, h()?, h()?, h()?, h()?, h()?);
    let round = u32::from_be_bytes(take(4)?.try_into().unwrap());
    let commit_id = u64::from_be_bytes(take(8)?.try_into().unwrap());
    let run_id = u64::from_be_bytes(take(8)?.try_into().unwrap());
    let kind = take(1)?[0];
    let mut s = || -> Result<String, &'static str> {
        let n = u16::from_be_bytes(take(2)?.try_into().unwrap()) as usize;
        String::from_utf8(take(n)?.to_vec()).map_err(|_| "public values: not utf-8")
    };
    let (program_key, verifier) = (s()?, s()?);
    let n = take(1)?[0] as usize;
    let mut broken = Vec::with_capacity(n);
    for _ in 0..n {
        let p = take(2)?;
        broken.push((p[0], p[1]));
    }
    if i != b.len() {
        return Err("public values: trailing bytes");
    }
    Ok(PublicValues { request_sha256, receipt_sha256, blob_sha256, base_sha256, manifest_sha256, anchor, round, commit_id, run_id, kind, program_key, verifier, broken })
}

/// The verdict kind's name, as near-antibody's facts write it (`antibody_zk_core::kind_byte`).
pub fn kind_name(k: u8) -> &'static str {
    match k {
        0 => "Violation",
        1 => "NoViolation",
        2 => "InvalidPoC",
        3 => "ProgramMisconfigured",
        4 => "Unevaluable",
        5 => "InfraError",
        _ => "?",
    }
}

/// A severity's name, as near-antibody's program parameters write it.
pub fn severity_name(s: u8) -> &'static str {
    match s {
        0 => "low",
        1 => "medium",
        2 => "high",
        3 => "critical",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixtures_public_values_decode() {
        let pv = decode(include_bytes!("../../../fixtures/zk/a/public_values.bin")).unwrap();
        assert_eq!((pv.round, pv.commit_id, pv.run_id, kind_name(pv.kind)), (0, 13, 33, "Violation"));
        assert_eq!(pv.program_key, "bounty.antibody.testnet/5");
        assert_eq!(pv.broken, vec![(0, 3)], "slot 0, critical");
        let c = decode(include_bytes!("../../../fixtures/zk/c/public_values.bin")).unwrap();
        assert_eq!((c.run_id, kind_name(c.kind), c.broken.len()), (35, "NoViolation", 0));
        let mut long = include_bytes!("../../../fixtures/zk/a/public_values.bin").to_vec();
        long.push(0);
        assert!(decode(&long).is_err(), "trailing bytes");
        assert!(decode(&long[..100]).is_err(), "truncated");
    }
}
