"""Find heavy, ordinary, recent mainnet transactions (read-only) for verifier benchmarking.

Categories: (a) aggregator swap through >= 5 pools, (b) Aave v3 liquidation, (c) flash-loan arbitrage.
Prefers low transaction index (cheaper prefix replay) and many internal calls (callTracer if available).
Usage: python -I find_candidates.py [lookback_blocks]
Writes measurements/candidates.json.
"""
import json
import os
import sys
import time
import urllib.request

RPCS = ["https://api.zan.top/eth-mainnet", "https://eth-mainnet.public.blastapi.io", "https://ethereum-rpc.publicnode.com",
        "https://rpc.mevblocker.io"]

T_LIQ_AAVE3 = "0xe413a321e8681d831f4dbccbca790d2952b56f977908e45be37335533e005286"
T_FLASH_AAVE3 = "0xefefaba5e921573100900a3ad9cf29f222d995fb3b6045797eaea7521bd8d6f0"
T_FLASH_BAL = "0x0d7d75e01ab95780d3cd1c8ec0dd6c2ce19e3a20427eec8bf53283b6fb8e95f0"
T_FLASH_UNIV3 = "0xbdbdb71d7860376ba52b25a5028beea23581364a40522f6bcfb86bb1f2dca633"
SWAP_TOPICS = {
    "0xd78ad95fa46c994b6551d0da85fc275fe613ce37657fb8d5e3d130840159d822": "univ2",
    "0xc42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67": "univ3",
    "0x40e9cecb9f5f1f1c5b9c97dec2917b7ee92e57ba5563708daca94dd84ad7112f": "univ4",
    "0x8b3e96f2b889fa771c53c981b40daf005f63f637f1869f707052d15a3dd97140": "curve",
    "0x2170c741c41531aec20e7c107c24eecfdd15e69c9bb0a8dd37b1840b9e0b207b": "balancer",
    "0x19b47279256b2a23a1665c810c8d55a1758940ee09377d4f8d26497a3577dc83": "pancakev3",
}
AGGREGATORS = {
    "0x111111125421ca6dc452d289314280a0f8842a65": "1inch v6",
    "0xcf5540fffcdc3d510b18bfca6d2b9987b0772559": "Odos v2",
    "0x0d05a7d3448512b78fa8a9e46c4872c88c4a0d05": "Odos v3",
    "0x6131b5fae19ea4f9d964eac0408e4408b66337b5": "KyberSwap",
    "0x6a000f20005980200259b80c5102003040001068": "ParaSwap v6.2",
    "0x9008d19f58aabd9ed0d60971565aa8510560ab41": "CoW settlement",
    "0x66a9893cc07d91d95644aedd05d03f95e1dba8af": "Uniswap UR v2",
    "0x3fc91a3afd70395cd496c647d5a6cc9d4b2b7fad": "Uniswap UR",
    "0xdef1c0ded9bec7f1a1670819833240f027b25eff": "0x Exchange Proxy",
    "0x00000000009726632680fb29d3f7a9734e3010e2": "Rainbow",
    "0x881d40237659c251811cec9c364ef91dc08d300c": "MetaMask Swaps",
    "0x1231deb6f5749ef6ce6943a275a1d3e7486f4eae": "LI.FI",
    "0x3a23f943181408eac424116af7b7790c94cb97a5": "Socket",
    "0x6352a56caadc4f1e25cd6c75970fa768a3304e64": "OpenOcean",
    "0x99a58482bd75cbab83b27ec03ca68ff489b5788f": "Curve router",
    "0x16a1063df4f8e0e8a8bd5a1d1cd0b8c3a7e3d3a1": "?",
}


def rpc(method, params, timeout=60, prefer=0):
    last = None
    for k in range(len(RPCS)):
        url = RPCS[(prefer + k) % len(RPCS)]
        body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
        req = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json", "User-Agent": "antibody-find/0.1"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                d = json.loads(r.read().decode())
            if "error" in d:
                last = d["error"]
                continue
            return d["result"]
        except Exception as e:  # noqa: BLE001
            last = e
            time.sleep(0.5)
    raise RuntimeError(f"{method} failed: {last}")


def logs(frm, to, topics, chunk):
    out = []
    b = frm
    while b <= to:
        e = min(to, b + chunk - 1)
        try:
            out += rpc("eth_getLogs", [{"fromBlock": hex(b), "toBlock": hex(e), "topics": topics}])
        except Exception as ex:  # noqa: BLE001
            print("  getLogs failed", b, e, str(ex)[:120])
        b = e + 1
    return out


def count_calls(tx_hash):
    try:
        tr = rpc("debug_traceTransaction", [tx_hash, {"tracer": "callTracer"}], timeout=90)
    except Exception as e:  # noqa: BLE001
        return None, str(e)[:100]
    n = 0
    depth = 0
    stack = [(tr, 0)]
    while stack:
        f, d = stack.pop()
        n += 1
        depth = max(depth, d)
        for c in f.get("calls", []) or []:
            stack.append((c, d + 1))
    return {"frames": n, "internal_calls": n - 1, "max_depth": depth}, None


def main():
    lookback = int(sys.argv[1]) if len(sys.argv) > 1 else 3000
    head = int(rpc("eth_blockNumber", []), 16)
    frm = head - lookback
    print("head", head, "from", frm)
    cands = {}

    def add(txh, cat, extra):
        c = cands.setdefault(txh, {"tx": txh, "cats": set(), "swaps": set(), "flash": set()})
        c["cats"].add(cat)
        for k, v in extra.items():
            if isinstance(v, set):
                c[k] |= v
            else:
                c[k] = v

    for l in logs(frm, head, [T_LIQ_AAVE3], 1000):
        add(l["transactionHash"], "liquidation", {"block": int(l["blockNumber"], 16), "index": int(l["transactionIndex"], 16)})
    for l in logs(frm, head, [[T_FLASH_AAVE3, T_FLASH_BAL, T_FLASH_UNIV3]], 500):
        add(l["transactionHash"], "flashloan", {"block": int(l["blockNumber"], 16), "index": int(l["transactionIndex"], 16),
                                                "flash": {l["address"].lower()}})
    swap_from = head - min(lookback, 400)
    for l in logs(swap_from, head, [list(SWAP_TOPICS.keys())], 20):
        add(l["transactionHash"], "swap", {"block": int(l["blockNumber"], 16), "index": int(l["transactionIndex"], 16),
                                           "swaps": {l["address"].lower() + ":" + SWAP_TOPICS[l["topics"][0]]}})
    print("raw candidates", len(cands))
    # Rank and enrich.
    ranked = []
    for c in cands.values():
        pools = len(c["swaps"])
        cat = "liquidation" if "liquidation" in c["cats"] else ("flashloan" if "flashloan" in c["cats"] else "swap")
        if cat == "swap" and pools < 5:
            continue
        ranked.append((cat, pools, c))
    out = []
    for cat in ["liquidation", "flashloan", "swap"]:
        group = [r for r in ranked if r[0] == cat]
        group.sort(key=lambda r: (-r[1], r[2]["index"]))
        for _, pools, c in group[:12]:
            tx = rpc("eth_getTransactionByHash", [c["tx"]])
            rc = rpc("eth_getTransactionReceipt", [c["tx"]])
            to = (tx.get("to") or "").lower()
            entry = {"category": cat, "tx": c["tx"], "block": c["block"], "index": c["index"], "to": to,
                     "router": AGGREGATORS.get(to), "from": tx["from"], "pools": pools, "pool_kinds": sorted({s.split(':')[1] for s in c["swaps"]}),
                     "logs": len(rc["logs"]), "gas_used": int(rc["gasUsed"], 16), "status": rc["status"],
                     "flash_sources": sorted(c["flash"])}
            calls, err = count_calls(c["tx"])
            entry["calls"] = calls
            if err:
                entry["trace_error"] = err
            out.append(entry)
            print(f"{cat:11s} idx={entry['index']:3d} pools={pools:2d} logs={entry['logs']:3d} gas={entry['gas_used']:8d} "
                  f"calls={calls['internal_calls'] if calls else '?':>4} router={entry['router']} {c['tx']}")
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    with open(os.path.join(root, "measurements", "candidates.json"), "w", encoding="utf-8") as fh:
        json.dump({"head": head, "from": frm, "candidates": out}, fh, indent=1)


if __name__ == "__main__":
    main()
