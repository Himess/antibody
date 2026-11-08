#!/usr/bin/env python3
"""native == wasm for a NEAR program (docs/near-targets.md §8).

Runs a NEAR program witness through the native antibody-verify-v2 and through its wasm32-wasip2 build
under antibody-run (crates/runner: OutLayer's engine configuration), both reading the witness on stdin
with the same environment, and requires their output to be byte-identical (full {receipt,facts} and
facts alone).

  python scripts/demo/check_v2_near_native_vs_wasm.py <witness.json> <native-exe> <wasm> <antibody-run>
"""
import subprocess
import sys

ENV = {"ANTIBODY_PROGRAM_ID": "antibody.testnet/12", "ANTIBODY_COMMIT_ID": "1"}


def native(exe, witness, output):
    import os
    env = dict(os.environ, **ENV)
    if output == "facts":
        env["ANTIBODY_OUTPUT"] = "facts"
    with open(witness, "rb") as f:
        return subprocess.run([exe], stdin=f, capture_output=True, env=env, check=True).stdout


def wasm(runner, wasm_path, witness, output):
    import json
    import os
    import tempfile
    cmd = [runner, wasm_path, witness]
    for k, v in ENV.items():
        cmd += ["--env", f"{k}={v}"]
    if output == "facts":
        cmd += ["--env", "ANTIBODY_OUTPUT=facts"]
    with tempfile.TemporaryDirectory() as d:
        out_path = os.path.join(d, f"wasm.{output}.out")
        cmd += ["--out", out_path]
        r = subprocess.run(cmd, capture_output=True, text=True, env=dict(os.environ))
        if r.returncode != 0:
            sys.exit(f"antibody-run failed: {r.stderr[-800:]}")
        status = json.loads(r.stdout)
        if status["status"] != "ok":
            sys.exit(f"wasm run not ok: {status['status']}")
        return open(out_path, "rb").read()


def main():
    witness, native_exe, wasm_path, runner = sys.argv[1:5]
    failures = []
    for output in ("full", "facts"):
        n = native(native_exe, witness, output)
        w = wasm(runner, wasm_path, witness, output)
        same = n.rstrip(b"\n") == w.rstrip(b"\n")
        print(f"{output:<6} native {len(n)} bytes, wasm {len(w)} bytes, byte-identical: {same}")
        if not same:
            failures.append(output)
    if failures:
        sys.exit(f"FAIL: native != wasm for {failures}")
    print("OK: native == wasm on the NEAR fixture, byte for byte")


if __name__ == "__main__":
    main()
