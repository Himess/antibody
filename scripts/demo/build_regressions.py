#!/usr/bin/env python3
"""Rebuilds the self-attack regression inputs in fixtures/regressions from the acme-local fixture.

  python scripts/demo/build_regressions.py --rpc http://127.0.0.1:8545

Run it after scripts/demo/acme_fixture.py, against the same anvil: the regressions are judged under that
fixture's manifest and addresses, so they are rebuilt whenever it is.
- SA-1 steering: PocSteer (contracts/evm/test/regressions/PocSteer.sol) breaks the known issue, restores
  the share price inside a frame that reverts, then calls an unrelated selector. Built like a submission.
- SA-2 jump tables: the KNOWN and B witnesses with every JUMPDEST of their largest bytecode marked
  invalid (`antibody-witness tamper --kind jump-table`).
Then scripts/demo/check_regressions.py checks the pinned verdicts and keys.
"""
import argparse
import hashlib
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(__file__))
from acme_fixture import BIN, EVM, EXE, ROOT, ETHER, abi_word_addr, abi_word_uint  # noqa: E402

FX = os.path.join(ROOT, "fixtures", "acme-local")
REG = os.path.join(ROOT, "fixtures", "regressions")


def witness_tool(*args):
    r = subprocess.run([os.path.join(BIN, "antibody-witness" + EXE), *args], capture_output=True, text=True, cwd=ROOT)
    if r.returncode != 0:
        sys.exit(f"antibody-witness {args[0]} failed: {r.stderr[-2000:]}")
    return r.stdout


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rpc", required=True)
    a = ap.parse_args()
    honest = json.load(open(os.path.join(FX, "expected.json")))
    program_id = honest["program_id"]
    dep = json.load(open(os.path.join(EVM, "deployments", "31337.json")))

    # SA-1: the steering PoC, five dust withdrawals then the reverted restoration and 0xdeadbeef.
    art = json.load(open(os.path.join(EVM, "out", "PocSteer.sol", "PocSteer.json")))["bytecode"]["object"]
    args = abi_word_addr(dep["AcmeVault"]) + abi_word_addr(dep["acUSD"]) + abi_word_uint(5) + "deadbeef".ljust(64, "0")
    bundle = {"senders": 1, "endowment_wei": str(ETHER), "protected": [],
              "calls": [{"sender": 0, "to": None, "value": "0", "data": art + args, "gas_limit": 5_000_000}]}
    prefix = os.path.join(REG, "steering")
    json.dump(bundle, open(prefix + ".bundle.json", "w"), indent=1)
    salt = "0x" + hashlib.sha256(b"antibody/regression/steering").hexdigest()
    witness_tool("program", "--rpc", a.rpc, "--manifest", os.path.join(FX, "manifest.json"),
                 "--invariants-code", os.path.join(FX, "invariants.hex"), "--bundle", prefix + ".bundle.json",
                 "--salt", salt, "--out", prefix, "--program-key", program_id)
    print("SA-1 steering witness rebuilt")

    # SA-2: tampered jump tables in two honest witnesses.
    for agent in ["KNOWN", "B"]:
        witness_tool("tamper", "--witness", os.path.join(FX, "submissions", f"{agent}.witness.bin"), "--kind", "jump-table",
                     "--out", os.path.join(REG, f"jump-table-{agent}.witness.bin"))
        print(f"SA-2 jump table ({agent}) rebuilt")


if __name__ == "__main__":
    main()
