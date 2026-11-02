"""Probe Ethereum mainnet RPC endpoints for historical eth_getProof support (read-only).

Usage: python -I probe_rpcs.py [--extra-env]   (reads ETH_RPC_* from antibody/.env if present, never prints them)
Writes measurements/rpc_probe.json and prints a table with endpoint labels (private URLs are masked).
"""
import concurrent.futures as cf
import hashlib
import json
import os
import sys
import time
import urllib.request

PUBLIC = [
    "https://ethereum-rpc.publicnode.com",
    "https://eth.llamarpc.com",
    "https://eth.drpc.org",
    "https://rpc.ankr.com/eth",
    "https://cloudflare-eth.com",
    "https://1rpc.io/eth",
    "https://eth.merkle.io",
    "https://mainnet.gateway.tenderly.co",
    "https://gateway.tenderly.co/public/mainnet",
    "https://rpc.mevblocker.io",
    "https://eth-pokt.nodies.app",
    "https://ethereum.blockpi.network/v1/rpc/public",
    "https://rpc.payload.de",
    "https://eth1.lava.build",
    "https://api.zan.top/eth-mainnet",
    "https://eth.api.onfinality.io/public",
    "https://rpc.flashbots.net",
    "https://eth-mainnet.public.blastapi.io",
    "https://endpoints.omniatech.io/v1/eth/mainnet/public",
    "https://ethereum.rpc.subquery.network/public",
    "https://public.stackup.sh/api/v1/node/ethereum-mainnet",
    "https://rpc.eth.gateway.fm",
    "https://eth.meowrpc.com",
    "https://eth.rpc.blxrbdn.com",
    "https://ethereum-json-rpc.stakely.io",
    "https://eth.blockrazor.xyz",
    "https://eth.nodeconnect.org",
    "https://rpc.therpc.io/ethereum",
    "https://eth-mainnet.g.alchemy.com/v2/demo",
]

WETH = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
SLOT0 = "0x" + "00" * 32


def load_env(path):
    env = {}
    if os.path.exists(path):
        for line in open(path, encoding="utf-8"):
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            k, v = line.split("=", 1)
            env[k.strip()] = v.strip().strip('"').strip("'")
    return env


def rpc(url, method, params, timeout=25):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json", "User-Agent": "antibody-probe/0.1"})
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=timeout) as r:
        data = json.loads(r.read().decode())
    dt = time.time() - t0
    if "error" in data:
        raise RuntimeError(str(data["error"])[:160])
    return data["result"], dt


def digest(obj):
    return hashlib.sha256(json.dumps(obj, sort_keys=True).encode()).hexdigest()[:16]


def probe(label, url, blocks):
    out = {"label": label}
    try:
        cid, dt = rpc(url, "eth_chainId", [])
        out["chainId"] = int(cid, 16)
        head, _ = rpc(url, "eth_blockNumber", [])
        out["head"] = int(head, 16)
        out["latency_ms"] = round(dt * 1000)
    except Exception as e:  # noqa: BLE001
        out["error"] = f"basic: {type(e).__name__}: {str(e)[:120]}"
        return out
    for name, bn in blocks.items():
        try:
            res, dt = rpc(url, "eth_getProof", [WETH, [SLOT0], hex(bn)])
            out[f"proof_{name}"] = {"ok": True, "ms": round(dt * 1000), "digest": digest(res),
                                     "nodes": len(res.get("accountProof", []))}
        except Exception as e:  # noqa: BLE001
            out[f"proof_{name}"] = {"ok": False, "err": f"{type(e).__name__}: {str(e)[:120]}"}
    try:
        res, dt = rpc(url, "debug_executionWitness", [hex(blocks["recent"])], timeout=40)
        out["executionWitness"] = {"ok": True, "ms": round(dt * 1000), "keys": sorted(res.keys())[:6]}
    except Exception as e:  # noqa: BLE001
        out["executionWitness"] = {"ok": False, "err": f"{type(e).__name__}: {str(e)[:100]}"}
    return out


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    env = load_env(os.path.join(root, ".env"))
    endpoints = [(u.split("//")[1].split("/")[0], u) for u in PUBLIC]
    for k in sorted(env):
        if k.startswith("ETH_RPC_") and env[k]:
            endpoints.insert(0, (f"private:{k}", env[k]))
    # reference head from a reliable public endpoint
    head = None
    for _, u in endpoints:
        try:
            h, _ = rpc(u, "eth_blockNumber", [])
            head = int(h, 16)
            break
        except Exception:  # noqa: BLE001
            continue
    blocks = {"recent": head - 20, "week": head - 50_400, "jan2026": 24_200_000, "jun2026": 25_300_000}
    print("reference head", head, "blocks", blocks)
    results = []
    with cf.ThreadPoolExecutor(max_workers=12) as ex:
        futs = {ex.submit(probe, l, u, blocks): l for l, u in endpoints}
        for f in cf.as_completed(futs):
            results.append(f.result())
    results.sort(key=lambda r: r["label"])
    os.makedirs(os.path.join(root, "measurements"), exist_ok=True)
    with open(os.path.join(root, "measurements", "rpc_probe.json"), "w", encoding="utf-8") as fh:
        json.dump({"time": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "blocks": blocks, "results": results}, fh, indent=1)
    for r in results:
        if "error" in r:
            print(f"{r['label']:45s} ERROR {r['error']}")
            continue
        cols = []
        for name in blocks:
            p = r.get(f"proof_{name}", {})
            cols.append(f"{name}:{'OK ' + p['digest'][:6] if p.get('ok') else 'no'}")
        ew = r.get("executionWitness", {})
        print(f"{r['label']:45s} chain={r.get('chainId')} head={r.get('head')} {r.get('latency_ms')}ms | " + " | ".join(cols) + f" | execWitness:{'OK' if ew.get('ok') else 'no'}")
        for name in blocks:
            p = r.get(f"proof_{name}", {})
            if not p.get("ok"):
                print(f"{'':45s}   {name}: {p.get('err')}")


if __name__ == "__main__":
    main()
