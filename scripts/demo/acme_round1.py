#!/usr/bin/env python3
"""Builds the round-1 fixture of Acme Lend (DELIBERATELY VULNERABLE TEST FIXTURE).

  python scripts/demo/acme_round1.py --rpc http://127.0.0.1:8545

Round 1 is the project's fix for planted bug #1, committed as a code override at the same anchor
(docs/rounds.md): AcmeLendFixed's runtime code replaces AcmeLend's. The script

  1. computes that runtime code with a read-only eth_call of the creation code (no transaction),
  2. derives fixtures/acme-local/round1/manifest.json with `antibody manifest-round`,
  3. rebuilds a witness at the round manifest for every finding that held a key in round 0, with the
     exact same bundle and salt (so the poc_hash is unchanged, as the TEE's re-run mode requires),
  4. runs the native verifier and writes round1/expected.json in the layout check_fixtures.py reads.

Expected: A and D (the spot-price bug) no longer break anything; B, F and the known issue still do.
"""
import argparse
import json
import os
import subprocess
import sys
import urllib.request

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
EVM = os.path.join(ROOT, "contracts", "evm")
BIN = os.path.join(ROOT, "target", "release")
EXE = ".exe" if os.name == "nt" else ""
FX = os.path.join(ROOT, "fixtures", "acme-local")
OUT = os.path.join(FX, "round1")

sys.path.insert(0, os.path.dirname(__file__))
from acme_fixture import k_program  # noqa: E402

# Round-0 findings re-run at the fix, with the verdict each must get there.
EXPECTED = {
    "A": ("NoViolation", "0x0"),
    "D": ("NoViolation", "0x0"),
    "B": ("Violation", "0x2"),
    "F": ("Violation", "0x1"),
    "KNOWN": ("Violation", "0x4"),
}


def rpc(url, method, params):
    req = urllib.request.Request(url, data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
                                 headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        out = json.load(r)
    if "error" in out:
        raise RuntimeError(f"{method}: {out['error']}")
    return out["result"]


def word(v):
    return hex(v)[2:].rjust(64, "0") if isinstance(v, int) else v[2:].lower().rjust(64, "0")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rpc", required=True)
    a = ap.parse_args()
    base = json.load(open(os.path.join(FX, "expected.json")))
    anchor = base["anchor"]["number"]
    dep = json.load(open(os.path.join(EVM, "deployments", "31337.json")))
    art = json.load(open(os.path.join(EVM, "out", "AcmeLendFixed.sol", "AcmeLendFixed.json")))
    p_ref = dep["pRef"]
    init = art["bytecode"]["object"] + "".join(word(x) for x in [dep["AcmeGuard"], dep["acUSD"], dep["acETH"], dep["AcmePool"], int(p_ref)])
    runtime = rpc(a.rpc, "eth_call", [{"data": init}, hex(anchor)])
    os.makedirs(os.path.join(OUT, "submissions"), exist_ok=True)
    code_file = os.path.join(OUT, "AcmeLendFixed.runtime.hex")
    open(code_file, "w").write(runtime + "\n")

    man = os.path.join(OUT, "manifest.json")
    r = subprocess.run([os.path.join(BIN, "antibody" + EXE), "manifest-round", "--base", os.path.join(FX, "manifest.json"),
                        "--round", "1", "--override", f"{dep['AcmeLend']}={code_file}", "--out", man],
                       capture_output=True, text=True, cwd=ROOT)
    if r.returncode != 0:
        sys.exit(r.stderr)
    mh = json.loads(r.stdout)["sha256"]
    print(f"round-1 manifest sha256 {mh}")

    pid = base["program_id"]
    kp = k_program(pid)
    results = {}
    for agent, (want_kind, want_mask) in EXPECTED.items():
        e0 = base["submissions"][agent]
        prefix = os.path.join(OUT, "submissions", agent)
        r = subprocess.run([os.path.join(BIN, "antibody-witness" + EXE), "program", "--rpc", a.rpc, "--manifest", man,
                            "--invariants-code", os.path.join(FX, "invariants.hex"),
                            "--bundle", os.path.join(FX, "submissions", f"{agent}.bundle.json"),
                            "--salt", e0["salt"], "--out", prefix], capture_output=True, text=True, cwd=ROOT)
        if r.returncode != 0:
            sys.exit(f"[{agent}] witness build failed: {r.stderr[-2000:]}")
        env = dict(os.environ, ANTIBODY_PROGRAM_ID=pid, ANTIBODY_COMMIT_ID=str(e0["commit_id"]),
                   ANTIBODY_EXPECTED_MANIFEST=mh, ANTIBODY_K_PROGRAM=kp.hex())
        v = subprocess.run([os.path.join(BIN, "antibody-verify" + EXE), prefix + ".witness.bin"], capture_output=True, env=env)
        out = json.loads(v.stdout)
        open(prefix + ".native.json", "wb").write(v.stdout)
        rec, facts = out["receipt"], out["facts"]
        same_poc = facts["poc_hash"] == e0["poc_hash"]
        ok = rec["kind"] == want_kind and rec["broken_mask"] == want_mask and same_poc
        if want_kind == "Violation":
            # A finding that still breaks keeps exactly its round-0 key(s).
            ok = ok and facts["fp_tags"] == e0["fp_tags"]
        results[agent] = {
            "title": e0["title"], "commit_id": e0["commit_id"], "salt": e0["salt"],
            "kind": rec["kind"], "broken_mask": rec["broken_mask"],
            "broken": [{"slot": f["slot"], "name": f["name"], "severity": f["severity"], "target": f["target_name"],
                        "selector": f["selector"], "target_address": f["target"],
                        "function": next((b["function"] for b in e0["broken"] if b["slot"] == f["slot"]), None)} for f in rec["broken"]],
            "fp_tags": facts["fp_tags"], "impact_usd": (rec.get("impact") or {}).get("impact_usd"),
            "evaluations": rec["evaluations"], "reason": rec["reason"],
            "poc_hash": facts["poc_hash"], "same_poc_as_round0": same_poc,
            "receipt_hash": facts["receipt_hash"], "witness_sha256": rec["witness_sha256"], "matches_expected": ok,
        }
        print(f"[{agent}] round 1: {rec['kind']:<12} mask={rec['broken_mask']:<4} same PoC={same_poc} {'OK' if ok else 'MISMATCH'}")
    exp = {"program_id": pid, "chain_id": base["chain_id"], "anchor": base["anchor"], "round": 1,
           "base_manifest_sha256": base["manifest_sha256"], "manifest_sha256": mh,
           "override": {"target": "AcmeLend", "address": dep["AcmeLend"], "code_hash": json.load(open(man))["overrides"][0]["code_hash"]},
           "invariants_code_hash": base["invariants_code_hash"], "invariants_address": base["invariants_address"],
           "k_program_note": base["k_program_note"], "submissions": results}
    json.dump(exp, open(os.path.join(OUT, "expected.json"), "w"), indent=1)
    bad = [k for k, v in results.items() if not v["matches_expected"]]
    print("ALL MATCH" if not bad else f"MISMATCH: {bad}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
