#!/usr/bin/env python3
"""Builds a round fixture of Acme Lend (DELIBERATELY VULNERABLE TEST FIXTURE).

  python scripts/demo/acme_round.py --rpc http://127.0.0.1:8545 --round 1   (then --round 2)

Each round is a fix committed as code overrides at the same anchor (docs/rounds.md): the fixed runtime
code replaces the code of an implementation behind a proxy (the proxy keeps its storage; a passed release
gate upgrades the proxy to the same code).
- Round 1, a partial fix: AcmeLendFixed for planted bug #1. A and D no longer break anything; B, F and
  the known issue still do, so the release gate stays shut.
- Round 2, the final fix: AcmeLendFinal (bugs #1 and #3) and AcmeVaultFixed (bug #2). Every paid
  finding is fixed; only the declared known issue still breaks.

The script
  1. computes each override's runtime code with a read-only eth_call of its creation code (no transaction),
  2. derives fixtures/acme-local/round<N>/manifest.json with `antibody manifest-round`,
  3. rebuilds a witness at the round manifest for every finding that held a key in round 0, with the
     exact same bundle and salt (so the poc_hash is unchanged, as the TEE's re-run mode requires),
  4. runs the native verifier and writes round<N>/expected.json in the layout check_fixtures.py reads.
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

sys.path.insert(0, os.path.dirname(__file__))
import v2keys  # noqa: E402,F401

LEND_ARGS = ["AcmeGuard", "acUSD", "acETH", "AcmePool", "pRef"]
VAULT_ARGS = ["AcmeGuard", "acUSD", "AcmeLend", "AcmeTreasury", "admin"]
FIXED = ("NoViolation", "0x0")

# Each round's overrides, (target, contract, source file, constructor arguments from the deployment), and
# the verdict every round-0 finding must get when re-run there.
ROUNDS = {
    1: {"overrides": [("AcmeLendImpl", "AcmeLendFixed", "AcmeLendFixed.sol", LEND_ARGS)],
        "expected": {"A": FIXED, "D": FIXED, "B": ("Violation", "0x2"), "F": ("Violation", "0x1"), "KNOWN": ("Violation", "0x4")}},
    2: {"overrides": [("AcmeLendImpl", "AcmeLendFinal", "AcmeFinalFix.sol", LEND_ARGS),
                      ("AcmeVaultImpl", "AcmeVaultFixed", "AcmeFinalFix.sol", VAULT_ARGS)],
        "expected": {"A": FIXED, "D": FIXED, "B": FIXED, "F": FIXED, "KNOWN": ("Violation", "0x4")}},
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
    ap.add_argument("--round", type=int, choices=sorted(ROUNDS), default=1)
    a = ap.parse_args()
    cfg = ROUNDS[a.round]
    out_dir = os.path.join(FX, f"round{a.round}")
    base = json.load(open(os.path.join(FX, "expected.json")))
    anchor = base["anchor"]["number"]
    dep = json.load(open(os.path.join(EVM, "deployments", "31337.json")))
    os.makedirs(os.path.join(out_dir, "submissions"), exist_ok=True)
    flags = []
    for target, contract, source, args in cfg["overrides"]:
        art = json.load(open(os.path.join(EVM, "out", source, f"{contract}.json")))
        init = art["bytecode"]["object"] + "".join(word(int(dep[x]) if x == "pRef" else dep[x]) for x in args)
        code_file = os.path.join(out_dir, f"{contract}.runtime.hex")
        open(code_file, "w").write(rpc(a.rpc, "eth_call", [{"data": init}, hex(anchor)]) + "\n")
        flags += ["--override", f"{dep[target]}={code_file}"]

    man = os.path.join(out_dir, "manifest.json")
    r = subprocess.run([os.path.join(BIN, "antibody" + EXE), "manifest-round", "--base", os.path.join(FX, "manifest.json"),
                        "--round", str(a.round), *flags, "--out", man], capture_output=True, text=True, cwd=ROOT)
    if r.returncode != 0:
        sys.exit(r.stderr)
    mh = json.loads(r.stdout)["sha256"]
    print(f"round-{a.round} manifest sha256 {mh}")

    pid = base["program_id"]
    results = {}
    for agent, (want_kind, want_mask) in cfg["expected"].items():
        e0 = base["submissions"][agent]
        prefix = os.path.join(out_dir, "submissions", agent)
        r = subprocess.run([os.path.join(BIN, "antibody-witness" + EXE), "program", "--rpc", a.rpc, "--manifest", man,
                            "--invariants-code", os.path.join(FX, "invariants.hex"),
                            "--bundle", os.path.join(FX, "submissions", f"{agent}.bundle.json"),
                            "--salt", e0["salt"], "--out", prefix, "--program-key", pid], capture_output=True, text=True, cwd=ROOT)
        if r.returncode != 0:
            sys.exit(f"[{agent}] witness build failed: {r.stderr[-2000:]}")
        env = dict(os.environ, ANTIBODY_PROGRAM_ID=pid, ANTIBODY_COMMIT_ID=str(e0["commit_id"]),
                   ANTIBODY_EXPECTED_MANIFEST=mh)
        v = subprocess.run([os.path.join(BIN, "antibody-verify" + EXE), prefix + ".witness.bin"], capture_output=True, env=env)
        out = json.loads(v.stdout)
        open(prefix + ".native.json", "wb").write(v.stdout)
        rec, facts = out["receipt"], out["facts"]
        same_poc = rec["poc_hash"] == e0["poc_hash"]
        ok = rec["kind"] == want_kind and rec["broken_mask"] == want_mask and same_poc
        if want_kind == "Violation":
            # A finding that still breaks keeps exactly its round-0 key(s).
            ok = ok and facts.get("keys", []) == e0["keys"]
        results[agent] = {
            "title": e0["title"], "commit_id": e0["commit_id"], "salt": e0["salt"],
            "kind": rec["kind"], "broken_mask": rec["broken_mask"],
            "broken": [{"slot": f["slot"], "name": f["name"], "severity": f["severity"], "target": f["target_name"],
                        "selector": f["selector"], "target_address": f["target"],
                        "function": next((b["function"] for b in e0["broken"] if b["slot"] == f["slot"]), None)} for f in rec["broken"]],
            "keys": facts.get("keys", []), "impact_usd": (rec.get("impact") or {}).get("impact_usd"),
            "evaluations": rec["evaluations"], "reason": rec["reason"],
            "poc_hash": rec["poc_hash"], "same_poc_as_round0": same_poc,
            "receipt_hash": facts["receipt_sha256"], "witness_sha256": rec["witness_sha256"], "matches_expected": ok,
        }
        print(f"[{agent}] round {a.round}: {rec['kind']:<12} mask={rec['broken_mask']:<4} same PoC={same_poc} {'OK' if ok else 'MISMATCH'}")
    hashes = {o["address"].lower(): o["code_hash"] for o in json.load(open(man))["overrides"]}
    exp = {"program_id": pid, "chain_id": base["chain_id"], "anchor": base["anchor"], "round": a.round,
           "base_manifest_sha256": base["manifest_sha256"], "manifest_sha256": mh,
           "overrides": [{"target": t, "contract": c, "address": dep[t], "code_hash": hashes[dep[t].lower()]}
                         for t, c, _, _ in cfg["overrides"]],
           "invariants_code_hash": base["invariants_code_hash"], "invariants_address": base["invariants_address"],
           "keys_note": base["keys_note"], "submissions": results}
    json.dump(exp, open(os.path.join(out_dir, "expected.json"), "w"), indent=1)
    bad = [k for k, v in results.items() if not v["matches_expected"]]
    print("ALL MATCH" if not bad else f"MISMATCH: {bad}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
