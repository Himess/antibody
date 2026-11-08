#!/usr/bin/env python3
"""Verifier v2 on v1's recorded EVM fixtures (no network, no keys).

v2 dispatches an EVM program to v1's code path unchanged, so for every submission in
<fixture>/expected.json, `antibody-verify-v2` must print exactly the bytes v1's native verifier printed
when the fixture was recorded (<fixture>/submissions/<agent>.native.json, receipt and facts), and the
facts alone must match too (verdict, mask, keys, receipt hash).

  python scripts/demo/check_v2_evm.py fixtures/acme-local [path/to/antibody-verify-v2]
"""
import json
import os
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
EXE = ".exe" if os.name == "nt" else ""


def main():
    fx = os.path.abspath(sys.argv[1])
    v2 = sys.argv[2] if len(sys.argv) > 2 else os.path.join(ROOT, "target", "release", "antibody-verify-v2" + EXE)
    exp = json.load(open(os.path.join(fx, "expected.json")))
    failures = []
    for agent, e in exp["submissions"].items():
        w = os.path.join(fx, "submissions", f"{agent}.witness.bin")
        env = dict(os.environ, ANTIBODY_PROGRAM_ID=exp["program_id"], ANTIBODY_COMMIT_ID=str(e["commit_id"]),
                   ANTIBODY_EXPECTED_MANIFEST=exp["manifest_sha256"])
        full = subprocess.run([v2, w], capture_output=True, env=env, check=True).stdout
        facts = subprocess.run([v2, w], capture_output=True, env=dict(env, ANTIBODY_OUTPUT="facts"), check=True).stdout
        recorded = open(os.path.join(fx, "submissions", f"{agent}.native.json"), "rb").read()
        same = full.rstrip(b"\n") == recorded.rstrip(b"\n")
        out = json.loads(full)
        f = json.loads(facts)
        if not same:
            failures.append(f"{agent}: v2's output differs from v1's recorded output")
        if f != out["facts"]:
            failures.append(f"{agent}: facts-only output differs from the full output's facts")
        if f["kind"] != e["kind"] or out["receipt"]["broken_mask"] != e["broken_mask"]:
            failures.append(f"{agent}: verdict {f['kind']} {out['receipt']['broken_mask']} != expected {e['kind']} {e['broken_mask']}")
        if f["receipt_sha256"] != e["receipt_hash"]:
            failures.append(f"{agent}: receipt hash {f['receipt_sha256']} != recorded {e['receipt_hash']}")
        print(f"{agent:<6} {f['kind']:<12} receipt {f['receipt_sha256'][:16]}  byte-identical to v1's recorded output: {same}")
    if failures:
        print("\n".join(["FAIL"] + failures))
        sys.exit(1)
    print(f"OK: {len(exp['submissions'])} EVM submissions through v2 match v1's recorded outputs byte for byte")


if __name__ == "__main__":
    main()
