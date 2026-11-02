#!/usr/bin/env python3
"""Runs every fixture witness through the native verifier and through the wasm32-wasip2 build under
wasmtime with OutLayer's engine config (crates/runner), then requires byte-identical output.

  python scripts/demo/native_vs_wasm.py fixtures/acme-local [--facts]
"""
import hashlib
import json
import os
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
EXE = ".exe" if os.name == "nt" else ""
NATIVE = os.path.join(ROOT, "target", "release", "antibody-verify" + EXE)
RUNNER = os.path.join(ROOT, "target", "release", "antibody-run" + EXE)
WASM = os.path.join(ROOT, "target", "wasm32-wasip2", "release", "antibody-verify.wasm")
sys.path.insert(0, os.path.dirname(__file__))
from acme_fixture import COMMIT_ID  # noqa: E402


def main():
    fx = os.path.abspath(sys.argv[1])
    facts_only = "--facts" in sys.argv
    exp = json.load(open(os.path.join(fx, "expected.json")))
    program_id = exp["program_id"]
    wasm_sha = hashlib.sha256(open(WASM, "rb").read()).hexdigest()
    # Keys come from the TEST root on both sides (the runner's encryption-keys host, the native default).
    envs = {"ANTIBODY_PROGRAM_ID": program_id, "ANTIBODY_EXPECTED_MANIFEST": exp["manifest_sha256"], "ANTIBODY_WASM_HASH": wasm_sha}
    if facts_only:
        envs["ANTIBODY_OUTPUT"] = "facts"
    rows, all_ok = [], True
    for agent in exp["submissions"]:
        w = os.path.join(fx, "submissions", f"{agent}.witness.bin")
        env = dict(envs, ANTIBODY_COMMIT_ID=str(COMMIT_ID[agent]))
        n = subprocess.run([NATIVE, w], capture_output=True, env=dict(os.environ, **env))
        out_path = os.path.join(fx, "submissions", f"{agent}.wasm.{'facts' if facts_only else 'full'}.json")
        cmd = [RUNNER, WASM, w, "--out", out_path]
        for k, v in env.items():
            cmd += ["--env", f"{k}={v}"]
        r = subprocess.run(cmd, capture_output=True, text=True)
        run = json.loads(r.stdout)
        wasm_out = open(out_path, "rb").read()
        same = wasm_out == n.stdout
        all_ok &= same and run["status"] == "ok"
        native_metrics = json.loads(n.stderr.strip().splitlines()[-1])["metrics"]
        rows.append({"agent": agent, "identical": same, "status": run["status"], "instructions": run["instructions"],
                     "guest_run_ms": (run.get("guest_metrics") or {}).get("metrics", {}).get("run_ms"),
                     "native_run_ms": native_metrics["run_ms"], "wasm_peak_mem_mb": run.get("peak_memory_mb"),
                     "output_sha256": hashlib.sha256(n.stdout).hexdigest(), "output_bytes": len(n.stdout)})
        print(f"{agent:<6} identical={same} status={run['status']} instr={run['instructions']:,} "
              f"native_run={native_metrics['run_ms']}ms out={len(n.stdout)}B")
    res = {"wasm_sha256": wasm_sha, "facts_only": facts_only, "all_identical": all_ok, "runs": rows}
    json.dump(res, open(os.path.join(fx, f"native_vs_wasm{'_facts' if facts_only else ''}.json"), "w"), indent=1)
    print("ALL IDENTICAL" if all_ok else "DIFFERENCES FOUND")
    sys.exit(0 if all_ok else 1)


if __name__ == "__main__":
    main()
