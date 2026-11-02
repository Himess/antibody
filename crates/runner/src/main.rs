//! antibody-run: runs the verifier component exactly the way OutLayer's worker does
//! (wasmtime 28, component model, async, fuel metering, epoch interruption, stdin/stdout pipes,
//! StoreLimits), and reports instructions (fuel), wall time, peak linear memory and output.
//!
//!   antibody-run <verifier.wasm> <witness.bin> [--max-instructions N] [--max-memory-mb M]
//!                [--timeout-s S] [--no-fuel] [--env K=V ...] [--out receipt.json]

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use wasmtime::{
    component::{Component, Linker, ResourceTable},
    Config, Engine, ResourceLimiter, Store, StoreLimits, StoreLimitsBuilder,
};
use wasmtime_wasi::{
    bindings::Command,
    pipe::{MemoryInputPipe, MemoryOutputPipe},
    WasiCtx, WasiCtxBuilder, WasiView,
};

/// Wraps OutLayer's StoreLimits and records the peak linear-memory size requested.
struct Tracking {
    inner: StoreLimits,
    peak: Arc<AtomicUsize>,
}

impl ResourceLimiter for Tracking {
    fn memory_growing(&mut self, current: usize, desired: usize, maximum: Option<usize>) -> anyhow::Result<bool> {
        let ok = self.inner.memory_growing(current, desired, maximum)?;
        if ok {
            self.peak.fetch_max(desired, Ordering::Relaxed);
        }
        Ok(ok)
    }
    fn table_growing(&mut self, current: usize, desired: usize, maximum: Option<usize>) -> anyhow::Result<bool> {
        self.inner.table_growing(current, desired, maximum)
    }
}

struct Host {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: Tracking,
}

impl WasiView for Host {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let wasm_path = args.get(1).context("usage: antibody-run <wasm> <witness>")?;
    let input_path = args.get(2).context("usage: antibody-run <wasm> <witness>")?;
    // OutLayer hard caps: 500B instructions, 4096 MB, 180 s.
    let max_instr: u64 = arg(&args, "--max-instructions").map(|s| s.parse().unwrap()).unwrap_or(500_000_000_000);
    let max_mem_mb: usize = arg(&args, "--max-memory-mb").map(|s| s.parse().unwrap()).unwrap_or(4096);
    let timeout_s: u64 = arg(&args, "--timeout-s").map(|s| s.parse().unwrap()).unwrap_or(180);
    let fuel = !args.iter().any(|a| a == "--no-fuel");

    let wasm = std::fs::read(wasm_path)?;
    let input = std::fs::read(input_path)?;
    let wasm_sha256 = hex::encode(Sha256::digest(&wasm));

    let mut config = Config::new();
    config.wasm_component_model(true);
    config.async_support(true);
    config.consume_fuel(fuel);
    config.epoch_interruption(true);
    let engine = Engine::new(&config)?;

    let t_compile = Instant::now();
    let component = Component::from_binary(&engine, &wasm)?;
    let compile_ms = t_compile.elapsed().as_secs_f64() * 1e3;

    let mut linker: Linker<Host> = Linker::new(&engine);
    wasmtime_wasi::add_to_linker_async(&mut linker)?;

    let stdout = MemoryOutputPipe::new(64 * 1024 * 1024);
    let stderr = MemoryOutputPipe::new(1024 * 1024);
    let mut b = WasiCtxBuilder::new();
    b.stdin(MemoryInputPipe::new(input.clone()));
    b.stdout(stdout.clone());
    b.stderr(stderr.clone());
    b.allow_tcp(false);
    b.allow_udp(false);
    b.allow_ip_name_lookup(false);
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--env" {
            if let Some((k, v)) = args.get(i + 1).and_then(|s| s.split_once('=')) {
                b.env(k, v);
            }
            i += 1;
        }
        i += 1;
    }
    let peak = Arc::new(AtomicUsize::new(0));
    let host = Host {
        wasi: b.build(),
        table: ResourceTable::new(),
        limits: Tracking {
            inner: StoreLimitsBuilder::new().memory_size(max_mem_mb * 1024 * 1024).table_elements(1_000_000).build(),
            peak: peak.clone(),
        },
    };
    let mut store = Store::new(&engine, host);
    store.limiter(|h| &mut h.limits);
    if fuel {
        store.set_fuel(max_instr)?;
    }
    store.set_epoch_deadline(timeout_s);
    store.epoch_deadline_trap();
    let eng = engine.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            eng.increment_epoch();
        }
    });

    let t_run = Instant::now();
    let command = Command::instantiate_async(&mut store, &component, &linker).await?;
    let result = tokio::time::timeout(Duration::from_secs(timeout_s + 2), command.wasi_cli_run().call_run(&mut store)).await;
    let run_ms = t_run.elapsed().as_secs_f64() * 1e3;
    ticker.abort();

    let fuel_used = if fuel { max_instr - store.get_fuel().unwrap_or(0) } else { 0 };
    let out = stdout.contents().to_vec();
    let err = String::from_utf8_lossy(&stderr.contents()).to_string();
    let status = match &result {
        Ok(Ok(Ok(()))) => "ok".to_string(),
        Ok(Ok(Err(()))) => "guest exited non-zero".to_string(),
        Ok(Err(e)) => match e.downcast_ref::<wasmtime_wasi::I32Exit>() {
            Some(wasmtime_wasi::I32Exit(0)) => "ok".to_string(),
            Some(wasmtime_wasi::I32Exit(c)) => format!("guest exit code {c}"),
            None => format!("trap: {}", format!("{e:#}").lines().next().unwrap_or_default()),
        },
        Err(_) => "wall-clock timeout".to_string(),
    };
    if let Some(p) = arg(&args, "--out") {
        std::fs::write(p, &out)?;
    }
    let guest_metrics: serde_json::Value = err
        .lines()
        .rev()
        .find_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .unwrap_or(serde_json::Value::Null);
    let receipt: serde_json::Value = serde_json::from_slice(&out).unwrap_or(serde_json::Value::Null);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "status": status,
            "wasm_bytes": wasm.len(),
            "wasm_sha256": wasm_sha256,
            "input_bytes": input.len(),
            "fuel_metering": fuel,
            "instructions": fuel_used,
            "compile_ms": (compile_ms * 10.0).round() / 10.0,
            "run_ms": (run_ms * 10.0).round() / 10.0,
            "peak_linear_memory_bytes": peak.load(Ordering::Relaxed),
            "guest_metrics": guest_metrics,
            "receipt_ok": receipt.get("schema").and_then(|s| s.as_str()) == Some("antibody.receipt.v1"),
            "failure": receipt.get("ok").map(|_| receipt.clone()),
            "stderr_tail": err.lines().rev().take(3).collect::<Vec<_>>(),
        }))?
    );
    Ok(())
}
