//! A bounty window's manifest, run with the verifier's own code and no network (docs/immune.md, "Bounty windows"):
//! the fixture's window config (fixtures/window) becomes a fork-mode manifest whose deployment the verifier runs on
//! an empty anchor, whose activation checks pass (the rules' lint, the targets' code hashes, `snapshot()` and
//! `check(snapshot()) == 0`), and against which a proof of the planted bug breaks the rule. `window open` does the
//! same with the deployment run at a real anchor (`cmd::deployed_code`).

use std::path::{Path, PathBuf};

use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use antibody_cli::{cmd, window};
use antibody_verifier::{
    bundle::{sender_address, Bundle, BundleCall},
    db::{MissingTracker, MissingWitness},
    manifest::{deployer, Anchor, Manifest},
};
use eyre::{eyre, Result};
use revm::{
    database_interface::DBErrorMarker,
    state::{AccountInfo, Bytecode},
    DatabaseRef,
};

const NOW: u64 = 1_800_000_000;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/window")
}

/// An anchor with nothing on it: the fixture's deployment reads nothing but the addresses it creates at.
#[derive(Debug)]
struct Anchorless;

#[derive(Debug)]
struct Never;
impl std::fmt::Display for Never {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("never")
    }
}
impl std::error::Error for Never {}
impl DBErrorMarker for Never {}

impl DatabaseRef for Anchorless {
    type Error = Never;
    fn basic_ref(&self, _: Address) -> Result<Option<AccountInfo>, Never> {
        Ok(None)
    }
    fn code_by_hash_ref(&self, _: B256) -> Result<Bytecode, Never> {
        Ok(Bytecode::default())
    }
    fn storage_ref(&self, _: Address, _: U256) -> Result<U256, Never> {
        Ok(U256::ZERO)
    }
    fn block_hash_ref(&self, _: u64) -> Result<B256, Never> {
        Ok(B256::ZERO)
    }
}

impl MissingTracker for Anchorless {
    fn first_missing(&self) -> Option<MissingWitness> {
        None
    }
}

fn header() -> alloy_consensus::Header {
    alloy_consensus::Header {
        gas_limit: 30_000_000,
        number: 1,
        timestamp: NOW,
        base_fee_per_gas: Some(0),
        excess_blob_gas: Some(0),
        blob_gas_used: Some(0),
        parent_beacon_block_root: Some(B256::ZERO),
        ..Default::default()
    }
}

fn anchor() -> Anchor {
    Anchor { chain_id: 31337, block_number: 1, block_hash: header().hash_slow(), evm_spec: "osaka".into(), trust: "project-attested".into() }
}

/// `cmd::deployed_code` with no RPC: the deployment run by the verifier's own code on [`Anchorless`].
fn offline_code(m: &Manifest, addrs: &[Address]) -> Result<Vec<(B256, Bytes)>> {
    let spec = antibody_verifier::run::program_chain_spec(m.anchor.chain_id).map_err(|e| eyre!("{e}"))?;
    let db = antibody_verifier::program::deployed(&header(), m, spec, &Anchorless).map_err(|e| eyre!("{:?}", antibody_verifier::program::classify(&e)))?;
    addrs
        .iter()
        .map(|a| {
            let info = db.basic_ref(*a).map_err(|e| eyre!("{e}"))?;
            Ok(match info {
                Some(i) if i.code_hash != keccak256([]) => {
                    let code = match i.code {
                        Some(c) => c,
                        None => db.code_by_hash_ref(i.code_hash).map_err(|e| eyre!("{e}"))?,
                    };
                    (i.code_hash, code.original_bytes())
                }
                _ => (keccak256([]), Bytes::new()),
            })
        })
        .collect()
}

fn args(cfg: &window::Config) -> window::PlanArgs {
    window::PlanArgs {
        name: window::window_name(7, "window-fixture", Some("1a2b3c4")),
        project: "window-bot.testnet".into(),
        description: window::describe(cfg.description.as_deref(), window::parse_window_name("PR #7 window (window-fixture @ 1a2b3c4)").as_ref(), None, "0.01", "wNEAR", NOW + 7_200),
        deadline: NOW + 7_200,
        escrow: cmd::parse_tokens(cfg.escrow.as_deref().unwrap_or("0.01"), 24).unwrap(),
        decimals: 24,
        root: fixture(),
        artifacts: cfg.artifacts.clone(),
        rules_artifacts: cfg.artifacts.clone(),
        now: NOW,
    }
}

#[test]
fn the_fixture_window_is_a_program_the_verifier_activates_and_a_proof_can_break() {
    let cfg = window::Config::load(&fixture().join("window.toml")).unwrap();
    let p = window::plan(&cfg, &args(&cfg), anchor(), offline_code).unwrap();
    let m = &p.manifest;
    let d = deployer();
    let (coin, vault) = (d.create(0), d.create(1));
    assert_eq!(p.rules.address, d.create(4));
    // The rules' runtime code is the instance's: its immutables name the vault and the coin.
    let rules = &m.invariants.runtime_code;
    assert_eq!(m.invariants.runtime_code_hash, keccak256(rules));
    assert!(rules.windows(20).any(|w| w == vault.as_slice()) && rules.windows(20).any(|w| w == coin.as_slice()));
    assert!(p.deployed.iter().all(|x| x.code_hash != B256::ZERO && x.code_hash != keccak256([])), "every created contract has code");

    // The verifier's activation checks pass on it: the rules pass the lint, every target has its code hash,
    // snapshot() and check(snapshot()) == 0 at the anchor.
    let spec = antibody_verifier::run::program_chain_spec(31337).unwrap();
    let reserved = Some(antibody_verifier::run::reserved_address("bounty.test.near/7", header().hash_slow()));
    let inv = Bytecode::new_raw(m.invariants.runtime_code.clone());
    let db = antibody_verifier::program::deployed(&header(), m, spec.clone(), &Anchorless).unwrap();
    antibody_verifier::program::anchor_checks(&header(), m, &inv, spec.clone(), &db, reserved).expect("the activation checks pass");

    // A proof that sweeps the vault (the planted bug) breaks the rule: hunters can win this window.
    let salt = B256::repeat_byte(7);
    let hunter = sender_address(&salt, 0);
    let sweep = [&keccak256(b"sweep(address)")[..4], hunter.into_word().as_slice()].concat();
    let bundle = Bundle {
        sender_salt: salt,
        senders: 1,
        endowment_wei: 10u128.pow(16),
        calls: vec![BundleCall { sender: 0, to: Some(vault), value: U256::ZERO, data: sweep.into(), gas_limit: 1_000_000 }],
    };
    let run = antibody_verifier::program::execute_program(&header(), m, &inv, &bundle, spec, &Anchorless, reserved).unwrap();
    assert_eq!(run.broken_mask, U256::from(1u8), "vault_keeps_its_coins breaks");
    assert!(run.target_code_hashes_match);
}

#[test]
fn a_deployment_that_reverts_names_its_transactions() {
    // The vault's sweep takes no ether: sending it some reverts the deployment's fourth transaction.
    let text = std::fs::read_to_string(fixture().join("window.toml")).unwrap();
    let text = text
        .replace("[[tx]]\ncall = \"coin\"", "[[tx]]\ncall = \"vault\"")
        .replace("mint(address, uint256)", "sweep(address)")
        .replace("args = [\"vault\", \"1000000000000000000000\"]", "args = [\"deployer\"]\nvalue = 5");
    let cfg = window::Config::parse(&text).unwrap();
    let e = format!("{:#}", window::plan(&cfg, &args(&cfg), anchor(), offline_code).unwrap_err());
    assert!(e.contains("deploy transaction 3 reverted") && e.contains("3: call vault.sweep(address)"), "{e}");
}
