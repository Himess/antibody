"""Compare eth_getProof results field-by-field across providers for one (address, slot, block).

Usage: python -I proof_diff.py <block> [address] [slot]
"""
import json
import sys
import urllib.request

PROVIDERS = {
    "zan": "https://api.zan.top/eth-mainnet",
    "blast": "https://eth-mainnet.public.blastapi.io",
    "mevblocker": "https://rpc.mevblocker.io",
}


def rpc(url, method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json", "User-Agent": "antibody-probe/0.1"})
    with urllib.request.urlopen(req, timeout=30) as r:
        d = json.loads(r.read().decode())
    if "error" in d:
        raise RuntimeError(d["error"])
    return d["result"]


def main():
    bn = int(sys.argv[1])
    addr = sys.argv[2] if len(sys.argv) > 2 else "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
    slot = sys.argv[3] if len(sys.argv) > 3 else "0x" + "00" * 32
    out = {}
    for name, url in PROVIDERS.items():
        try:
            hdr = rpc(url, "eth_getBlockByNumber", [hex(bn), False])
            pr = rpc(url, "eth_getProof", [addr, [slot], hex(bn)])
            out[name] = {"hash": hdr["hash"], "stateRoot": hdr["stateRoot"], "balance": pr["balance"], "nonce": pr["nonce"],
                         "codeHash": pr["codeHash"], "storageHash": pr["storageHash"],
                         "acctProofLen": len(pr["accountProof"]), "acctProof0": pr["accountProof"][0][:20],
                         "acctProofLast": pr["accountProof"][-1][-40:],
                         "slotValue": pr["storageProof"][0]["value"], "slotKey": pr["storageProof"][0]["key"],
                         "slotProofLen": len(pr["storageProof"][0]["proof"])}
        except Exception as e:  # noqa: BLE001
            out[name] = {"error": str(e)[:200]}
    keys = sorted({k for v in out.values() for k in v})
    for k in keys:
        vals = {n: out[n].get(k) for n in out}
        same = len({json.dumps(v) for v in vals.values()}) == 1
        print(f"{'  ' if same else '!!'} {k:14s} " + " | ".join(f"{n}={vals[n]}" for n in vals))


if __name__ == "__main__":
    main()
