"""On-chain anchor check (read-only): compare each receipt's (block_number, block_hash) with NEAR's
Ethereum light client `client-eth2.bridge.near.block_hash_safe(n)` — the check Antibody's NEAR
contract performs before paying. Also reports the light client's head and GC window.

Usage: python -I scripts/anchor_check.py receipt1.json [receipt2.json ...]
"""
import base64
import json
import struct
import sys
import time
import urllib.request

NEAR_RPC = "https://rpc.mainnet.fastnear.com"
CLIENT = "client-eth2.bridge.near"


def view(method, args_bytes):
    body = {"jsonrpc": "2.0", "id": 1, "method": "query",
            "params": {"request_type": "call_function", "finality": "final", "account_id": CLIENT,
                       "method_name": method, "args_base64": base64.b64encode(args_bytes).decode()}}
    req = urllib.request.Request(NEAR_RPC, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    t = time.time()
    with urllib.request.urlopen(req, timeout=30) as r:
        d = json.loads(r.read().decode())
    if "error" in d or "error" in d.get("result", {}):
        raise RuntimeError(d.get("error") or d["result"]["error"])
    return bytes(d["result"]["result"]), d["result"]["block_height"], (time.time() - t) * 1000


def block_hash_safe(n):
    raw, near_height, ms = view("block_hash_safe", struct.pack("<Q", n))
    if raw and raw[0] == 1:
        return "0x" + raw[1:33].hex(), near_height, ms
    return None, near_height, ms


def main():
    head_raw, near_h, _ = view("last_block_number", b"")
    lc_head = struct.unpack("<Q", head_raw[:8])[0] if len(head_raw) >= 8 else None
    print(f"light client {CLIENT}: last_block_number={lc_head} (NEAR height {near_h})")
    rows = []
    for path in sys.argv[1:]:
        r = json.load(open(path))
        n, h = r["env"]["block_number"], r["env"]["block_hash"].lower()
        anchored, near_height, ms = block_hash_safe(n)
        verdict = "PAY (anchored)" if anchored == h else ("REJECT (hash mismatch)" if anchored else "REJECT (not in light client window)")
        rows.append({"receipt": path, "block": n, "receipt_hash": h, "light_client_hash": anchored, "verdict": verdict,
                     "near_height": near_height, "view_ms": round(ms)})
        print(f"{path}: block {n} receipt {h[:18]}.. light-client {str(anchored)[:18]}.. -> {verdict} ({ms:.0f} ms)")
    print(json.dumps({"light_client_head": lc_head, "checks": rows}, indent=1))


if __name__ == "__main__":
    main()
