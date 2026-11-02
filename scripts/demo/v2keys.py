"""TEST-ONLY v2 keys for the recorded fixtures (docs/spec.md §11.2), recomputed independently of Rust.

In the TEE every program key is a `mac` under OutLayer's encryption key, which only the pinned verifier
build called by near-antibody holds. Locally a published TEST root key stands in for it, with
HMAC-SHA256 as the `mac`.
"""
import hashlib
import hmac
import subprocess
import os

TEST_ROOT = hashlib.sha256(b"antibody/TEST-ONLY-root/never-use-in-production").digest()
# The fixtures' program key: "<near-antibody account>/<program id>".
PROGRAM_KEY = "antibody.testnet/acme-local"
EXE = ".exe" if os.name == "nt" else ""


def label(name, program_key, rest=b""):
    pk = program_key.encode()
    return name.encode() + b"\x00" + len(pk).to_bytes(4, "big") + pk + rest


def mac(data):
    return hmac.new(TEST_ROOT, data, hashlib.sha256).digest()


def tag(program_key, slot, target_hex, selector_hex, group=None):
    """A root-cause key's tag: `mac("antibody/v2/tag" ‖ program ‖ slot ‖ group)`, the group being a committed
    group's id or the entry point ("f" ‖ target ‖ selector)."""
    if group is None:
        g = b"f" + bytes.fromhex(target_hex[2:]) + bytes.fromhex(selector_hex[2:])
    else:
        g = b"g" + group.to_bytes(2, "big")
    return mac(label("antibody/v2/tag", program_key, bytes([slot]) + g)).hex()


def keccak_hex(data_hex):
    out = subprocess.run([os.path.expanduser("~/.foundry/bin/cast" + EXE), "keccak", data_hex], capture_output=True, text=True, check=True)
    return out.stdout.strip()


def reserved_address(program_key, anchor_hash):
    """`keccak256("antibody/invariants/v2" ‖ program_key ‖ anchor hash)[12..]` (spec §5.7)."""
    data = b"antibody/invariants/v2" + program_key.encode() + bytes.fromhex(anchor_hash[2:])
    return "0x" + keccak_hex("0x" + data.hex())[-40:]
