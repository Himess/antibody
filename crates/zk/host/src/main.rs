//! `antibody-zk <guest elf> <case dir>...`: runs the SP1 program on each disclosed run in SP1's executor
//! (no proof) and prints one JSON line per case: cycles, time, the public values, and whether the
//! receipt equals the one the TEE produced (`expected.json`: near-antibody's record of the run).
//!
//! A case dir holds `request.json`, `content_key.hex`, `expected.json`, `blob.bin` (unless the request
//! carries the blob inline) and finds the base witness in `../base/<sha256>.bin` (fixtures/zk/).

use std::{path::Path, sync::Arc, time::Instant};

use serde_json::{json, Value};
use sp1_core_executor::{MinimalExecutorEnum, Program};

/// The public values (layout v1 of `antibody_zk_core::Rerun::public_values`).
struct PublicValues {
    request_sha256: [u8; 32],
    receipt_sha256: [u8; 32],
    blob_sha256: [u8; 32],
    anchor: [u8; 32],
    commit_id: u64,
    run_id: u64,
    kind: u8,
    program_key: String,
    verifier: String,
    broken: Vec<(u8, u8)>,
}

fn decode_public_values(b: &[u8]) -> Result<PublicValues, String> {
    let mut i = 0usize;
    let mut take = |n: usize| -> Result<Vec<u8>, String> {
        let s = b.get(i..i + n).ok_or("public values: truncated")?.to_vec();
        i += n;
        Ok(s)
    };
    if take(1)?[0] != 1 {
        return Err("public values: not layout v1".into());
    }
    let mut h = Vec::new();
    for _ in 0..6 {
        h.push(<[u8; 32]>::try_from(take(32)?).unwrap());
    }
    let _round = u32::from_be_bytes(take(4)?.try_into().unwrap());
    let commit_id = u64::from_be_bytes(take(8)?.try_into().unwrap());
    let run_id = u64::from_be_bytes(take(8)?.try_into().unwrap());
    let kind = take(1)?[0];
    let mut strings = Vec::new();
    for _ in 0..2 {
        let n = u16::from_be_bytes(take(2)?.try_into().unwrap()) as usize;
        strings.push(String::from_utf8(take(n)?).map_err(|_| "public values: not utf-8")?);
    }
    let n = take(1)?[0] as usize;
    let mut broken = Vec::new();
    for _ in 0..n {
        let p = take(2)?;
        broken.push((p[0], p[1]));
    }
    if i != b.len() {
        return Err("public values: trailing bytes".into());
    }
    let verifier = strings.pop().unwrap();
    let program_key = strings.pop().unwrap();
    Ok(PublicValues { request_sha256: h[0], receipt_sha256: h[1], blob_sha256: h[2], anchor: h[5], commit_id, run_id, kind, program_key, verifier, broken })
}

fn read(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (elf_path, cases) = args.split_first().expect("usage: antibody-zk <guest elf> <case dir>...");
    let elf = read(Path::new(elf_path));
    let program = Arc::new(Program::from(&elf).expect("an SP1 program"));
    let mut all_ok = true;
    for case in cases {
        let d = Path::new(case);
        let expected: Value = serde_json::from_slice(&read(&d.join("expected.json"))).expect("expected.json");
        let base = read(&d.join("../base").join(format!("{}.bin", expected["base_sha256"].as_str().expect("base_sha256"))));
        let blob = std::fs::read(d.join("blob.bin")).unwrap_or_default();
        let key = hex::decode(String::from_utf8(read(&d.join("content_key.hex"))).unwrap().trim()).expect("content key hex");
        let mut ex = MinimalExecutorEnum::new(program.clone(), false, None);
        for input in [read(&d.join("request.json")), base, Vec::new(), blob, key] {
            ex.with_input(&input);
        }
        let t = Instant::now();
        while ex.execute_chunk().is_some() {}
        let secs = t.elapsed().as_secs_f64();
        let cycles = ex.global_clk();
        let exit_code = ex.exit_code();
        let pv_bytes = ex.public_values_stream().clone();
        let row = match decode_public_values(&pv_bytes) {
            Ok(pv) => {
                let ok = exit_code == 0
                    && hex::encode(pv.receipt_sha256) == expected["tee"]["receipt_sha256"].as_str().unwrap_or_default()
                    && hex::encode(pv.request_sha256) == expected["request_sha256"].as_str().unwrap_or_default()
                    && hex::encode(pv.blob_sha256) == expected["blob_sha256"].as_str().unwrap_or_default()
                    && pv.run_id == expected["run_id"].as_u64().unwrap_or(u64::MAX);
                all_ok &= ok;
                json!({"case": case, "cycles": cycles, "execute_seconds": secs, "mhz": cycles as f64 / secs / 1e6, "exit_code": exit_code,
                       "receipt_equals_the_tees": ok, "receipt_sha256": hex::encode(pv.receipt_sha256), "tee_receipt_sha256": expected["tee"]["receipt_sha256"],
                       "public_values_bytes": pv_bytes.len(), "public_values_sha256": hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&pv_bytes)),
                       "run_id": pv.run_id, "commit_id": pv.commit_id, "kind": pv.kind, "broken": pv.broken, "program_key": pv.program_key,
                       "anchor": hex::encode(pv.anchor), "verifier": pv.verifier})
            }
            Err(e) => {
                all_ok = false;
                json!({"case": case, "cycles": cycles, "exit_code": exit_code, "error": e})
            }
        };
        println!("{row}");
    }
    if !all_ok {
        std::process::exit(1);
    }
}
