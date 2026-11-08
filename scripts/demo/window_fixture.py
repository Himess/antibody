#!/usr/bin/env python3
"""Builds fixtures/window (a small fork-mode deployment for `antibody window`'s tests) with Foundry, in a temporary
copy, and writes the artifacts the tests read: fixtures/window/artifacts/<File>.sol/<Contract>.json, each with the
contract's ABI and creation code only (the rest of a Foundry artifact is not read).

    python3 scripts/demo/window_fixture.py [--forge forge]

Run it after editing fixtures/window/src; the tests compare nothing else with Foundry's output.
"""

import argparse
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "fixtures" / "window"
CONTRACTS = {"Window.sol": ["Coin", "Vault", "Note", "VaultRules"]}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--forge", default="forge")
    a = ap.parse_args()
    with tempfile.TemporaryDirectory() as tmp:
        work = pathlib.Path(tmp) / "window"
        shutil.copytree(FIXTURE / "src", work / "src")
        shutil.copy(FIXTURE / "foundry.toml", work / "foundry.toml")
        subprocess.run([a.forge, "build", "--root", str(work)], check=True)
        out = FIXTURE / "artifacts"
        shutil.rmtree(out, ignore_errors=True)
        for file, names in CONTRACTS.items():
            for name in names:
                art = json.loads((work / "out" / file / f"{name}.json").read_text(encoding="utf-8"))
                kept = {"abi": art["abi"], "bytecode": {"object": art["bytecode"]["object"], "linkReferences": art["bytecode"].get("linkReferences", {})}}
                dest = out / file / f"{name}.json"
                dest.parent.mkdir(parents=True, exist_ok=True)
                with dest.open("w", encoding="utf-8", newline="\n") as f:
                    f.write(json.dumps(kept, indent=1) + "\n")
                print(f"{dest.relative_to(ROOT)}: {len(kept['bytecode']['object']) // 2 - 1} bytes of creation code")
    return 0


if __name__ == "__main__":
    sys.exit(main())
