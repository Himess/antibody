#!/usr/bin/env python3
"""CI gate for self-attack regressions (no network, no keys).

Each case in fixtures/regressions/expected.json is an input that once broke Antibody's own verifier. The
check runs the native verifier (and, unless --native-only, the wasm32-wasip2 build) and requires:
  * the verdict kind and root-cause keys equal the pinned ones;
  * `same_key_as` / `same_verdict_as`: the keys equal those of an honest acme-local submission;
  * native output == wasm output, byte for byte.
Only verdicts and keys are pinned, so the check survives receipt-format changes.

  python scripts/demo/check_regressions.py [--native-only]
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
from check_fixtures import ROOT, run_native, run_wasm  # noqa: E402


def keys(receipt):
    return [(b["slot"], b["target_name"], b["selector"]) for b in receipt["broken"]]


def main():
    native_only = "--native-only" in sys.argv
    reg = os.path.join(ROOT, "fixtures", "regressions")
    exp = json.load(open(os.path.join(reg, "expected.json")))
    fx = os.path.join(ROOT, exp["fixture"])
    honest = json.load(open(os.path.join(fx, "expected.json")))
    pid = honest["program_id"]
    base_env = {"ANTIBODY_PROGRAM_ID": pid, "ANTIBODY_EXPECTED_MANIFEST": honest["manifest_sha256"]}
    failures = []
    for name, c in exp["cases"].items():
        env = dict(base_env, ANTIBODY_COMMIT_ID=str(c["commit_id"]))
        w = os.path.join(reg, c["witness"])
        full = run_native(w, env)
        rec = json.loads(full)["receipt"]
        got = keys(rec)
        if rec["kind"] != c["kind"]:
            failures.append(f"{name}: verdict {rec['kind']} != {c['kind']} ({rec.get('reason')})")
        want = None
        if "broken" in c:
            want = [(b["slot"], b["target"], b["selector"]) for b in c["broken"]]
        ref = c.get("same_key_as") or c.get("same_verdict_as")
        if ref:
            ref_keys = [(b["slot"], b["target"], b["selector"]) for b in honest["submissions"][ref]["broken"]]
            if want is not None and want != ref_keys:
                failures.append(f"{name}: pinned keys {want} differ from {ref}'s {ref_keys}")
            want = ref_keys
            if c.get("same_verdict_as") and rec["broken_mask"] != honest["submissions"][ref]["broken_mask"]:
                failures.append(f"{name}: mask {rec['broken_mask']} != {ref}'s")
        if got != want:
            failures.append(f"{name}: keys {got} != expected {want}")
        same = ""
        if not native_only:
            out = os.path.join(reg, f".{c['witness']}.wasm.json")
            wfull, _ = run_wasm(w, env, out)
            os.remove(out)
            if wfull != full:
                failures.append(f"{name}: wasm output differs from native")
            same = f" native==wasm={wfull == full}"
        print(f"{name:<26} {rec['kind']:<12} keys={got}{same}")
    if failures:
        print("\n".join(["FAIL"] + failures))
        sys.exit(1)
    print(f"OK: {len(exp['cases'])} self-attack regressions hold")


if __name__ == "__main__":
    main()
