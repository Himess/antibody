//! `antibody-verify`: reads a witness envelope (bincode) from a file argument or stdin, prints the
//! receipt JSON on stdout (or a failure JSON) and resource metrics as one JSON line on stderr.
//! Exit code: 0 = verified, 1 = verification failed, 2 = I/O error.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    io::{Read, Write},
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

use antibody_verifier::{receipt::Failure, run, Envelope, RunOptions};
use sha2::{Digest, Sha256};

/// Counting allocator: reports peak heap bytes (works natively and in wasm).
struct Counting;
static CUR: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            let c = CUR.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(c, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        CUR.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = System.realloc(p, l, new);
        if !q.is_null() {
            if new > l.size() {
                let c = CUR.fetch_add(new - l.size(), Ordering::Relaxed) + (new - l.size());
                PEAK.fetch_max(c, Ordering::Relaxed);
            } else {
                CUR.fetch_sub(l.size() - new, Ordering::Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn main() {
    let t0 = Instant::now();
    let mut input = Vec::new();
    let res = match std::env::args().nth(1) {
        Some(path) => std::fs::File::open(&path).and_then(|mut f| f.read_to_end(&mut input)),
        None => std::io::stdin().read_to_end(&mut input),
    };
    if let Err(e) = res {
        eprintln!("io error: {e}");
        std::process::exit(2);
    }
    let t_read = t0.elapsed();
    let digest: [u8; 32] = Sha256::digest(&input).into();
    let opts = RunOptions { full_diff: std::env::var("ANTIBODY_FULL_DIFF").map(|v| v == "1").unwrap_or(false) };

    let t1 = Instant::now();
    let envelope = Envelope::decode(&input);
    drop(input);
    let t_decode = t1.elapsed();

    // Optional phase stop (cost attribution): ANTIBODY_STOP=decode|verify
    let stop = std::env::var("ANTIBODY_STOP").unwrap_or_default();
    if stop == "decode" || stop == "verify" {
        let r = envelope.map_err(antibody_verifier::VerifyError::Decode).and_then(|env| {
            if stop == "verify" {
                antibody_verifier::verify_only(&env).map(|_| ())
            } else {
                Ok(())
            }
        });
        println!("{{\"phase\":\"{stop}\",\"ok\":{}}}", r.is_ok());
        return;
    }
    let t2 = Instant::now();
    if let Ok(env @ Envelope { payload: antibody_verifier::Payload::Program(w), .. }) = &envelope {
        let out = program_output(env, w, digest);
        let t_run = t2.elapsed();
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&out).expect("write output");
        writeln!(stdout).ok();
        stdout.flush().ok();
        eprintln!(
            "{{\"metrics\":{{\"read_ms\":{:.1},\"decode_ms\":{:.1},\"run_ms\":{:.1},\"total_ms\":{:.1},\"peak_heap_bytes\":{}}}}}",
            t_read.as_secs_f64() * 1e3,
            t_decode.as_secs_f64() * 1e3,
            t_run.as_secs_f64() * 1e3,
            t0.elapsed().as_secs_f64() * 1e3,
            PEAK.load(Ordering::Relaxed)
        );
        return;
    }
    let outcome = envelope.map_err(antibody_verifier::VerifyError::Decode).and_then(|env| run(&env, digest, &opts));
    let t_run = t2.elapsed();

    let mut out = std::io::stdout().lock();
    let code = match outcome {
        Ok(receipt) => {
            serde_json::to_writer_pretty(&mut out, &receipt).expect("write receipt");
            0
        }
        Err(e) => {
            // Typed verdict on stdout with exit code 0: OutLayer drops stdout on a non-zero exit, so a
            // failure must still be a successful run that *reports* the failure class.
            let f = Failure { schema: "antibody.failure.v1", ok: false, class: e.class(), error: e.to_string() };
            serde_json::to_writer_pretty(&mut out, &f).expect("write failure");
            if std::env::var("ANTIBODY_STRICT_EXIT").map(|v| v == "1").unwrap_or(false) {
                1
            } else {
                0
            }
        }
    };
    writeln!(out).ok();
    out.flush().ok();
    eprintln!(
        "{{\"metrics\":{{\"read_ms\":{:.1},\"decode_ms\":{:.1},\"run_ms\":{:.1},\"total_ms\":{:.1},\"peak_heap_bytes\":{}}}}}",
        t_read.as_secs_f64() * 1e3,
        t_decode.as_secs_f64() * 1e3,
        t_run.as_secs_f64() * 1e3,
        t0.elapsed().as_secs_f64() * 1e3,
        PEAK.load(Ordering::Relaxed)
    );
    // Return normally on success: some WASI hosts report `proc_exit(0)` as an error.
    if code != 0 {
        std::process::exit(code);
    }
}

fn env_hex32(name: &str) -> Option<[u8; 32]> {
    let v = std::env::var(name).ok()?;
    let b = hex::decode(v.trim().trim_start_matches("0x")).ok()?;
    <[u8; 32]>::try_from(b.as_slice()).ok()
}

/// Program mode (v0.4). Context comes from the caller (in the TEE: the contract's request):
/// ANTIBODY_PROGRAM_ID, ANTIBODY_COMMIT_ID, ANTIBODY_EXPECTED_MANIFEST (hex),
/// ANTIBODY_K_PROGRAM (hex; TEST key for local runs — in the TEE it is derived from the protected
/// secret), ANTIBODY_WASM_HASH. ANTIBODY_OUTPUT=facts prints only the facts (≤ 1 KB, compact);
/// otherwise `{"receipt":…,"facts":…}`.
fn program_output(env: &Envelope, w: &antibody_verifier::ProgramWitness, digest: [u8; 32]) -> Vec<u8> {
    let ctx = antibody_verifier::ProgramContext {
        program_id: std::env::var("ANTIBODY_PROGRAM_ID").unwrap_or_else(|_| "local".into()),
        commit_id: std::env::var("ANTIBODY_COMMIT_ID").ok().and_then(|v| v.parse().ok()).unwrap_or(0),
        expected_manifest_hash: env_hex32("ANTIBODY_EXPECTED_MANIFEST").map(Into::into),
    };
    let receipt = antibody_verifier::run_program(env, w, digest, &ctx);
    let k = env_hex32("ANTIBODY_K_PROGRAM");
    let facts = receipt.facts(k.as_ref(), None, std::env::var("ANTIBODY_WASM_HASH").ok());
    if std::env::var("ANTIBODY_OUTPUT").map(|v| v == "facts").unwrap_or(false) {
        serde_json::to_vec(&facts).expect("facts serialize")
    } else {
        #[derive(serde::Serialize)]
        struct Full<'a> {
            receipt: &'a antibody_verifier::facts::ProgramReceipt,
            facts: &'a antibody_verifier::facts::Facts,
        }
        serde_json::to_vec_pretty(&Full { receipt: &receipt, facts: &facts }).expect("output serialize")
    }
}
