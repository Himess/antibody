"""Compare the verifier's measured impact with an independent explorer (Blockscout API, read-only).

For each receipt: per watched account, net token-unit deltas from Blockscout's token-transfer list
must equal the verifier's balanceOf()-based deltas exactly; ETH: sender delta = -(value + fee) etc.
USD: verifier uses proven-block Chainlink/AaveOracle prices; Blockscout shows current rates (approx).

Usage: python -I scripts/impact_vs_explorer.py receipt.json [...]
"""
import json
import sys
import time
import urllib.request
from collections import defaultdict
from decimal import Decimal

BS = "https://eth.blockscout.com/api/v2"


def get(url):
    for attempt in range(4):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "antibody-impact-check/0.1", "Accept": "application/json"})
            with urllib.request.urlopen(req, timeout=40) as r:
                return json.loads(r.read().decode())
        except Exception as e:  # noqa: BLE001
            last = e
            time.sleep(1.5 * (attempt + 1))
    raise RuntimeError(f"{url}: {last}")


def transfers(tx):
    items, params = [], ""
    while True:
        d = get(f"{BS}/transactions/{tx}/token-transfers{params}")
        items += d.get("items", [])
        nxt = d.get("next_page_params")
        if not nxt:
            break
        params = "?" + "&".join(f"{k}={v}" for k, v in nxt.items())
    return items


def main():
    report = []
    for path in sys.argv[1:]:
        r = json.load(open(path))
        tx = r["target"]["tx_hash"]
        txd = get(f"{BS}/transactions/{tx}")
        tts = transfers(tx)
        net = defaultdict(int)
        meta = {}
        for t in tts:
            if t.get("token", {}).get("type") != "ERC-20":
                continue
            tok = t["token"]["address_hash"].lower() if "address_hash" in t["token"] else t["token"]["address"].lower()
            amt = int(t["total"]["value"])
            frm = t["from"]["hash"].lower()
            to = t["to"]["hash"].lower()
            net[(frm, tok)] -= amt
            net[(to, tok)] += amt
            meta[tok] = (t["token"].get("symbol"), int(t["token"].get("decimals") or 0), t["token"].get("exchange_rate"))
        rows = []
        for acc in r["impact"]["accounts"]:
            a = acc["account"].lower()
            for td in acc["tokens"]:
                tok = td["token"].lower()
                ours = int(td["delta_by_balance_of"])
                theirs = net.get((a, tok), 0)
                sym, dec, rate = meta.get(tok, ("?", td.get("decimals") or 0, None))
                usd_explorer = (Decimal(theirs) / (Decimal(10) ** dec) * Decimal(rate)) if rate else None
                rows.append({"account": a, "token": tok, "symbol": sym, "verifier_delta": str(ours), "explorer_delta": str(theirs),
                             "exact_match": ours == theirs, "verifier_usd_at_block": td.get("usd"),
                             "explorer_usd_at_current_rate": f"{usd_explorer:.2f}" if usd_explorer is not None else None})
        fee = int(txd.get("fee", {}).get("value") or 0)
        value = int(txd.get("value") or 0)
        sender = r["target"]["from"].lower()
        eth_ours = next((int(a["eth_delta_wei"]) for a in r["impact"]["accounts"] if a["account"].lower() == sender), None)
        eth_expected_basic = -(fee + value)
        entry = {"receipt": path, "tx": tx, "explorer_status": txd.get("status"), "explorer_fee_wei": str(fee),
                 "verifier_fee_wei": r["impact"]["sender_fee_wei"], "fee_match": str(fee) == r["impact"]["sender_fee_wei"],
                 "sender_eth_delta_verifier": str(eth_ours), "sender_eth_delta_if_no_internal_eth": str(eth_expected_basic),
                 "token_rows": rows, "prices_used": r["impact"]["prices"],
                 "verifier_usd_totals": {a["account"]: a["usd_total"] for a in r["impact"]["accounts"]}}
        report.append(entry)
        print(f"== {path} tx {tx}")
        print(f"   fee: verifier {entry['verifier_fee_wei']} explorer {fee} match={entry['fee_match']}; sender ETH delta {eth_ours} (if no internal ETH: {eth_expected_basic})")
        for row in rows:
            print(f"   {row['symbol']:>8s} {row['account'][:10]}.. verifier {row['verifier_delta']:>28s} explorer {row['explorer_delta']:>28s} "
                  f"exact={row['exact_match']} usd@block={row['verifier_usd_at_block']} usd@now={row['explorer_usd_at_current_rate']}")
        for p in r["impact"]["prices"]:
            print(f"   price {p['token'][:10]}.. feed {p['feed'][:10]}.. answer {p['answer']} dec {p['feed_decimals']} age {p['age_seconds']}s")
    json.dump(report, open("measurements/impact_vs_explorer.json", "w"), indent=1)


if __name__ == "__main__":
    main()
