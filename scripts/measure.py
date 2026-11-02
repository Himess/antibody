"""Measure the verifier natively and under wasmtime 28 (OutLayer engine config) on each witness.

Usage: python -I scripts/measure.py [names...]   (default: flashloan liquidation aggswap)
Writes measurements/verifier_measurements.json and prints a markdown table.
"""
import hashlib
import json
import os
import statistics
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NATIVE = os.path.join(ROOT, "target", "release", "antibody-verify.exe")
RUNNER = os.path.join(ROOT, "target", "release", "antibody-run.exe")
WASM = os.path.join(ROOT, "target", "wasm32-wasip2", "release", "antibody-verify.wasm")


def sh(cmd, env=None):
    e = dict(os.environ)
    if env:
        e.update(env)
    t = time.perf_counter()
    p = subprocess.run(cmd, capture_output=True, env=e)
    return p, (time.perf_counter() - t) * 1000


def native_run(w, stop=None):
    env = {"ANTIBODY_FULL_DIFF": "0"}
    if stop:
        env["ANTIBODY_STOP"] = stop
    p, wall = sh([NATIVE, w], env)
    metrics = {}
    for line in p.stderr.decode(errors="replace").splitlines():
        try:
            metrics = json.loads(line)["metrics"]
        except Exception:  # noqa: BLE001
            pass
    return p.returncode, p.stdout, metrics, wall


def wasm_run(w, fuel=True, stop=None):
    cmd = [RUNNER, WASM, w]
    if not fuel:
        cmd.append("--no-fuel")
    if stop:
        cmd += ["--env", f"ANTIBODY_STOP={stop}"]
    out = os.path.join(ROOT, "measurements", "_tmp_receipt.json")
    cmd += ["--out", out]
    p, wall = sh(cmd)
    r = json.loads(p.stdout)
    receipt = open(out, "rb").read()
    return r, receipt, wall


def main():
    names = sys.argv[1:] or ["flashloan", "liquidation", "aggswap", "bundle-benign-swap"]
    results = {}
    wasm_bytes = open(WASM, "rb").read()
    for name in names:
        w = os.path.join(ROOT, "witnesses", f"{name}.bin")
        wb = open(w, "rb").read()
        meta = json.load(open(os.path.join(ROOT, "witnesses", f"{name}.meta.json")))
        is_bundle = "anchor_block" in meta
        nat = [native_run(w) for _ in range(5)]
        assert all(n[0] == 0 for n in nat), f"native failed on {name}: {nat[0][1][:300]}"
        nat_receipt = nat[0][1]
        fuel_runs = [wasm_run(w, True) for _ in range(3)]
        nofuel_runs = [wasm_run(w, False) for _ in range(3)]
        phases = {}
        for stop in ["decode", "verify"]:
            if is_bundle and stop == "verify":
                phases[stop] = None
                continue
            r, _, _ = wasm_run(w, True, stop)
            phases[stop] = r["instructions"]
        f0 = fuel_runs[0][0]
        receipt_json = json.loads(nat_receipt)
        res = {
            "witness_bytes": len(wb),
            "witness_sha256": hashlib.sha256(wb).hexdigest(),
            "mode": receipt_json["mode"],
            "block": meta["anchor_block"] if is_bundle else meta["block_number"],
            "tx_index": None if is_bundle else meta["tx_index"],
            "txs_executed": len(receipt_json["bundle"]["calls"]) if is_bundle else meta["tx_index"] + 1,
            "target_gas_used": receipt_json["bundle"]["total_gas_used"] if is_bundle else receipt_json["target"]["gas_used"],
            "prefix_and_target_gas": receipt_json["bundle"]["total_gas_used"] if is_bundle else receipt_json["target"]["cumulative_gas_used"],
            "stats": meta["stats"],
            "native_total_ms_median": statistics.median(n[2]["total_ms"] for n in nat),
            "native_decode_ms_median": statistics.median(n[2]["decode_ms"] for n in nat),
            "native_run_ms_median": statistics.median(n[2]["run_ms"] for n in nat),
            "native_peak_heap_bytes": nat[0][2]["peak_heap_bytes"],
            "wasm_instructions": f0["instructions"],
            "wasm_instructions_all_runs_equal": len({r[0]["instructions"] for r in fuel_runs}) == 1,
            "wasm_instructions_decode": phases["decode"],
            "wasm_instructions_decode_plus_verify": phases["verify"],
            "wasm_guest_total_ms_fuel_median": statistics.median(r[0]["guest_metrics"]["metrics"]["total_ms"] for r in fuel_runs),
            "wasm_guest_total_ms_nofuel_median": statistics.median(r[0]["guest_metrics"]["metrics"]["total_ms"] for r in nofuel_runs),
            "wasm_run_ms_fuel_median": statistics.median(r[0]["run_ms"] for r in fuel_runs),
            "wasm_compile_ms": f0["compile_ms"],
            "wasm_peak_linear_memory_bytes": f0["peak_linear_memory_bytes"],
            "wasm_guest_peak_heap_bytes": f0["guest_metrics"]["metrics"]["peak_heap_bytes"],
            "wasm_status": f0["status"],
            "receipt_native_equals_wasm": all(r[1] == nat_receipt for r in fuel_runs + nofuel_runs),
            "receipt_sha256": hashlib.sha256(nat_receipt).hexdigest(),
        }
        results[name] = res
        print(f"{name}: instr={res['wasm_instructions']:,} (decode {res['wasm_instructions_decode']:,}, +verify {res['wasm_instructions_decode_plus_verify']}) "
              f"native={res['native_total_ms_median']:.1f}ms wasm(fuel)={res['wasm_guest_total_ms_fuel_median']:.1f}ms "
              f"wasm(nofuel)={res['wasm_guest_total_ms_nofuel_median']:.1f}ms mem={res['wasm_peak_linear_memory_bytes']/1e6:.1f}MB "
              f"equal={res['receipt_native_equals_wasm']} det={res['wasm_instructions_all_runs_equal']}")
    out = {"time_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
           "wasm_bytes": len(wasm_bytes), "wasm_sha256": hashlib.sha256(wasm_bytes).hexdigest(),
           "host": "AMD Ryzen 5 5600 (6C/12T), 16 GB, Windows 10; wasmtime 28 (component model, async, consume_fuel, epoch)",
           "results": results}
    with open(os.path.join(ROOT, "measurements", "verifier_measurements.json"), "w", encoding="utf-8") as fh:
        json.dump(out, fh, indent=1)
    os.remove(os.path.join(ROOT, "measurements", "_tmp_receipt.json"))


if __name__ == "__main__":
    main()
