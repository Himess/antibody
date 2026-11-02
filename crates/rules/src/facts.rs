//! Facts v2: what a verifier run returns (spec §5.2), and the summary the rules keep.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::types::*;

/// Facts are at most this many bytes of canonical JSON.
pub const MAX_FACTS_BYTES: usize = 1024;
/// At most this many root-cause keys per verdict.
pub const MAX_KEYS: usize = 4;
/// At most this many tags per disclosure run.
pub const MAX_DISCLOSE: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct FactKey {
    pub slot: u8,
    pub group: u16,
    pub tag: H32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundCheck {
    pub ok: bool,
    pub canaries_ok: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disclosed {
    pub tag: H32,
    pub key: H32,
}

/// The JSON a verifier run returns. Fields not used by a mode are absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Facts {
    pub v: u8,
    pub mode: Mode,
    pub request_sha256: H32,
    pub manifest_sha256: H32,
    pub program_key: String,
    pub round: u32,
    /// Hex of the program's key epoch; empty for an `activate` run of a program without one yet.
    pub key_epoch: String,
    pub commit_id: Option<u64>,
    pub run_id: u64,
    pub kind: Kind,
    #[serde(default)]
    pub keys: Vec<FactKey>,
    #[serde(default)]
    pub impact_usd_micro: String,
    #[serde(default)]
    pub protected_harm_usd_micro: String,
    pub receipt_sha256: H32,
    #[serde(default)]
    pub poc_sha256: Option<H32>,
    #[serde(default)]
    pub reason: Option<String>,
    /// `activate`: the program's X25519 sealing public key.
    #[serde(default)]
    pub sealing_pk: Option<H32>,
    /// `check_round`.
    #[serde(default)]
    pub round_check: Option<RoundCheck>,
    /// `release_wrap`: the content key wrapped to the project's key.
    #[serde(default)]
    pub wrap: Option<String>,
    /// `verify`: the content key under the disclosure key of the primary tag.
    #[serde(default)]
    pub wrap_d: Option<String>,
    /// `disclose`.
    #[serde(default)]
    pub disclosed: Vec<Disclosed>,
    /// `distinct`: the patch stops the earlier key's PoC and not this one.
    #[serde(default)]
    pub distinct: Option<bool>,
}

/// What the rules keep of a run's facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct FactsSummary {
    pub kind: Kind,
    pub keys: Vec<FactKey>,
    pub impact_usd_micro: Amount,
    pub protected_harm_usd_micro: Amount,
    pub receipt_sha256: H32,
    pub reason: Option<String>,
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum Extra {
    None,
    SealingKey { pk: H32, epoch: String },
    RoundCheck { ok: bool, canaries_ok: bool },
    Wrap { wrap: String },
    WrapD { wrap_d: String },
    Disclosed { keys: Vec<(H32, H32)> },
    Distinct { distinct: bool },
}

/// What a run's facts must match: the request near-antibody sent.
pub struct Expect<'a> {
    pub mode: Mode,
    pub request_sha256: H32,
    pub manifest_sha256: H32,
    pub program_key: &'a str,
    pub round: u32,
    pub key_epoch: &'a str,
    pub commit_id: Option<u64>,
    pub run_id: u64,
    /// Slots that exist for this run's round.
    pub slots: &'a [SlotRule],
}

fn amount(s: &str) -> Result<Amount, String> {
    if s.is_empty() {
        return Ok(0);
    }
    if s.len() > 39 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err("an amount is not a decimal integer".into());
    }
    s.parse::<Amount>().map_err(|_| "an amount does not fit".into())
}

/// Parses and checks facts. Any mismatch with the request is the run's fault, never the hunter's: the
/// caller turns an error into `InfraError`.
pub fn parse(bytes: &[u8], ex: &Expect) -> Result<FactsSummary, String> {
    if bytes.len() > MAX_FACTS_BYTES {
        return Err(format!("facts are {} bytes, over {}", bytes.len(), MAX_FACTS_BYTES));
    }
    let f: Facts = serde_json::from_slice(bytes).map_err(|e| format!("facts do not parse: {e}"))?;
    if f.v != 2 {
        return Err("facts version is not 2".into());
    }
    if f.mode != ex.mode {
        return Err("facts are for another mode".into());
    }
    if f.request_sha256 != ex.request_sha256 {
        return Err("facts bind another request".into());
    }
    if f.manifest_sha256 != ex.manifest_sha256 || f.program_key != ex.program_key || f.round != ex.round {
        return Err("facts are for another program, manifest or round".into());
    }
    if f.commit_id != ex.commit_id || f.run_id != ex.run_id {
        return Err("facts are for another commit or run".into());
    }
    if ex.mode != Mode::Activate && f.key_epoch != ex.key_epoch {
        return Err("facts use another key epoch".into());
    }
    if f.keys.len() > MAX_KEYS {
        return Err("too many keys".into());
    }
    for (i, k) in f.keys.iter().enumerate() {
        if !ex.slots.iter().any(|s| s.slot == k.slot) {
            return Err("a key names an unknown slot".into());
        }
        if f.keys[..i].iter().any(|o| o.tag == k.tag) {
            return Err("duplicate key".into());
        }
    }
    let violation_like = matches!(ex.mode, Mode::Verify | Mode::Second | Mode::Rerun);
    if violation_like && (f.kind == Kind::Violation) != !f.keys.is_empty() {
        return Err("keys must be present exactly for a Violation".into());
    }
    if let Some(r) = &f.reason {
        if r.len() > 64 {
            return Err("reason too long".into());
        }
    }
    let extra = match ex.mode {
        Mode::Activate => match (&f.sealing_pk, f.kind) {
            (Some(pk), Kind::NoViolation) => {
                if f.key_epoch.len() != 16 || !f.key_epoch.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("activation key epoch is not 8 bytes of hex".into());
                }
                Extra::SealingKey { pk: *pk, epoch: f.key_epoch.clone() }
            }
            (_, Kind::InfraError) | (_, Kind::ProgramMisconfigured) => Extra::None,
            _ => return Err("an activation must return a sealing key".into()),
        },
        Mode::CheckRound => match &f.round_check {
            Some(rc) => Extra::RoundCheck { ok: rc.ok, canaries_ok: rc.canaries_ok },
            None if f.kind == Kind::InfraError => Extra::None,
            None => return Err("a round check must say whether the round is valid".into()),
        },
        Mode::ReleaseWrap => match &f.wrap {
            Some(w) if w.len() <= 256 => Extra::Wrap { wrap: w.clone() },
            None if f.kind == Kind::InfraError => Extra::None,
            _ => return Err("a release wrap must carry a wrap".into()),
        },
        Mode::Disclose => {
            if f.disclosed.len() > MAX_DISCLOSE {
                return Err("too many disclosed keys".into());
            }
            Extra::Disclosed { keys: f.disclosed.iter().map(|d| (d.tag, d.key)).collect() }
        }
        Mode::Distinct => match f.distinct {
            Some(d) => Extra::Distinct { distinct: d },
            None if f.kind == Kind::InfraError => Extra::None,
            None => return Err("a distinctness run must say whether the bugs are distinct".into()),
        },
        Mode::Verify | Mode::Second => match &f.wrap_d {
            Some(w) if w.len() <= 256 => Extra::WrapD { wrap_d: w.clone() },
            Some(_) => return Err("wrap_d too long".into()),
            None => Extra::None,
        },
        Mode::Rerun => Extra::None,
    };
    Ok(FactsSummary {
        kind: f.kind,
        keys: f.keys,
        impact_usd_micro: amount(&f.impact_usd_micro)?,
        protected_harm_usd_micro: amount(&f.protected_harm_usd_micro)?,
        receipt_sha256: f.receipt_sha256,
        reason: f.reason,
        extra,
    })
}

/// Two runs agree when they report the same verdict: kind, keys and impact (spec §6.2, §6.4).
pub fn agree(a: &FactsSummary, b: &FactsSummary) -> bool {
    a.kind == b.kind
        && a.keys == b.keys
        && a.impact_usd_micro == b.impact_usd_micro
        && a.protected_harm_usd_micro == b.protected_harm_usd_micro
        && a.receipt_sha256 == b.receipt_sha256
}
