#!/usr/bin/env python3
"""CI gate for recorded program fixtures (no network, no keys).

For every submission in <fixture>/expected.json it runs the native verifier and the wasm32-wasip2 build
under wasmtime with OutLayer's engine config, then requires:
  * native output == wasm output, byte for byte (full receipt + facts, and facts alone);
  * the verdict kind, broken-invariant mask and root-cause keys equal the expected ones;
  * every fp_tag equals an independent HMAC recomputation, and equal keys give equal tags;
  * the receipt hash equals the recorded one (receipts are platform-independent).

  python scripts/demo/check_fixtures.py fixtures/acme-local [--native-only]
"""
import hashlib
import hmac
import json
import os
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
EXE = ".exe" if os.name == "nt" else ""
NATIVE = os.path.join(ROOT, "target", "release", "antibody-verify" + EXE)
RUNNER = os.path.join(ROOT, "target", "release", "antibody-run" + EXE)
WASM = os.path.join(ROOT, "target", "wasm32-wasip2", "release", "antibody-verify.wasm")

# TEST-ONLY fingerprint secret (the TEE derives k_program from a protected secret instead).
TEST_SECRET = hashlib.sha256(b"antibody/TEST-ONLY-secret/never-use-in-production").digest()


def k_program(program_id):
    return hmac.new(TEST_SECRET, b"antibody/k_program/v1" + program_id.encode(), hashlib.sha256).digest()


def fp_tag(k, slot, target, selector):
    msg = b"antibody/fp/v1" + bytes([slot]) + bytes.fromhex(target[2:]) + bytes.fromhex(selector[2:])
    return hmac.new(k, msg, hashlib.sha256).hexdigest()


def run_native(witness, env):
    r = subprocess.run([NATIVE, witness], capture_output=True, env=dict(os.environ, **env))
    if r.returncode != 0:
        sys.exit(f"native verifier failed on {witness}: {r.stderr.decode()[-500:]}")
    return r.stdout


def run_wasm(witness, env, out_path):
    cmd = [RUNNER, WASM, witness, "--out", out_path]
    for k, v in env.items():
        cmd += ["--env", f"{k}={v}"]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"wasm runner failed on {witness}: {r.stderr[-500:]}")
    status = json.loads(r.stdout)
    if status["status"] != "ok":
        sys.exit(f"wasm run not ok on {witness}: {status['status']}")
    with open(out_path, "rb") as f:
        return f.read(), status


def main():
    fx = os.path.abspath(sys.argv[1])
    native_only = "--native-only" in sys.argv
    exp = json.load(open(os.path.join(fx, "expected.json")))
    pid = exp["program_id"]
    kp = k_program(pid)
    failures = []
    tags_by_key = {}
    for agent, e in exp["submissions"].items():
        w = os.path.join(fx, "submissions", f"{agent}.witness.bin")
        env = {"ANTIBODY_PROGRAM_ID": pid, "ANTIBODY_COMMIT_ID": str(e["commit_id"]),
               "ANTIBODY_EXPECTED_MANIFEST": exp["manifest_sha256"], "ANTIBODY_K_PROGRAM": kp.hex()}
        full = run_native(w, env)
        facts_only = run_native(w, dict(env, ANTIBODY_OUTPUT="facts"))
        out = json.loads(full)
        rec, facts = out["receipt"], out["facts"]
        if facts != json.loads(facts_only):
            failures.append(f"{agent}: facts-only output differs from the full output's facts")
        if rec["kind"] != e["kind"] or rec["broken_mask"] != e["broken_mask"]:
            failures.append(f"{agent}: verdict {rec['kind']} {rec['broken_mask']} != expected {e['kind']} {e['broken_mask']}")
        got = [(b["slot"], b["target_name"], b["selector"]) for b in rec["broken"]]
        want = [(b["slot"], b["target"], b["selector"]) for b in e["broken"]]
        if got != want:
            failures.append(f"{agent}: keys {got} != expected {want}")
        if facts["receipt_hash"] != e["receipt_hash"]:
            failures.append(f"{agent}: receipt hash {facts['receipt_hash']} != recorded {e['receipt_hash']}")
        if len(json.dumps(facts, separators=(",", ":"))) > 1024:
            failures.append(f"{agent}: facts exceed 1 KB")
        for b, t in zip(rec["broken"], facts["fp_tags"]):
            if t["tag"] != fp_tag(kp, b["slot"], b["target"], b["selector"]):
                failures.append(f"{agent}: fp_tag for slot {b['slot']} does not match the HMAC recomputation")
            key = (b["slot"], b["target"].lower(), b["selector"])
            tags_by_key.setdefault(key, set()).add(t["tag"])
        if not native_only:
            wfull, st = run_wasm(w, env, os.path.join(fx, "submissions", f"{agent}.wasm.full.json"))
            wfacts, _ = run_wasm(w, dict(env, ANTIBODY_OUTPUT="facts"), os.path.join(fx, "submissions", f"{agent}.wasm.facts.json"))
            if wfull != full or wfacts != facts_only:
                failures.append(f"{agent}: wasm output differs from native")
            print(f"{agent:<6} {rec['kind']:<12} {rec['broken_mask']:<4} keys={got} native==wasm={wfull == full} instr={st['instructions']:,}")
        else:
            print(f"{agent:<6} {rec['kind']:<12} {rec['broken_mask']:<4} keys={got}")
    for key, tags in tags_by_key.items():
        if len(tags) != 1:
            failures.append(f"key {key} produced different tags {tags}")
    if failures:
        print("\n".join(["FAIL"] + failures))
        sys.exit(1)
    print(f"OK: {len(exp['submissions'])} submissions match {os.path.relpath(fx, ROOT)}/expected.json")


if __name__ == "__main__":
    main()
