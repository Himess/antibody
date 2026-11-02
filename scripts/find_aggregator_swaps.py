"""Find user swaps routed through a known aggregator that touch >= 5 pools (read-only)."""
import json, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from find_candidates import rpc, logs, SWAP_TOPICS, AGGREGATORS  # noqa: E402

head = int(rpc("eth_blockNumber", []), 16)
lookback = int(sys.argv[1]) if len(sys.argv) > 1 else 600
by_tx = {}
for l in logs(head - lookback, head, [list(SWAP_TOPICS.keys())], 20):
    t = by_tx.setdefault(l["transactionHash"], {"pools": set(), "block": int(l["blockNumber"], 16), "index": int(l["transactionIndex"], 16)})
    t["pools"].add(l["address"].lower())
multi = [(h, t) for h, t in by_tx.items() if len(t["pools"]) >= 5]
print("txs with >=5 swap pools:", len(multi))
found = []
for h, t in sorted(multi, key=lambda x: (-len(x[1]["pools"]), x[1]["index"]))[:250]:
    tx = rpc("eth_getTransactionByHash", [h])
    to = (tx.get("to") or "").lower()
    if to in AGGREGATORS:
        rc = rpc("eth_getTransactionReceipt", [h])
        e = {"tx": h, "router": AGGREGATORS[to], "to": to, "from": tx["from"], "block": t["block"], "index": t["index"],
             "pools": len(t["pools"]), "logs": len(rc["logs"]), "gas_used": int(rc["gasUsed"], 16), "status": rc["status"]}
        found.append(e)
        print(f"{e['router']:16s} idx={e['index']:3d} pools={e['pools']:2d} logs={e['logs']:3d} gas={e['gas_used']:8d} {h}")
json.dump({"head": head, "found": found}, open(os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "measurements", "aggregator_candidates.json"), "w"), indent=1)
