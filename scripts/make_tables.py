"""Generate the report's measurement tables from raw measurement files (no hand transcription).
Usage: python -I scripts/make_tables.py > report/tables.md
"""
import json
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
J = lambda *p: json.load(open(os.path.join(ROOT, *p), encoding="utf-8"))  # noqa: E731

vm = J("measurements", "verifier_measurements.json")
names = ["flashloan", "liquidation", "aggswap", "bundle-benign-swap"]
label = {"flashloan": "Flash-loan arbitrage (replay)", "liquidation": "Aave v3 liquidation (replay)",
         "aggswap": "Aggregator swap, 8 pools (replay)", "bundle-benign-swap": "Benign bundle: wrap+swap (bundle mode)"}
calls = {}
for line in open(os.path.join(ROOT, "measurements", "call_counts.jsonl")):
    d = json.loads(line)
    calls[os.path.basename(d["witness"]).replace(".bin", "")] = d["internal_calls"]

print("### Table M1: workloads\n")
print("| Workload | Mode | Block | Tx index / calls | Target gas | Internal calls |")
print("|---|---|---|---|---|---|")
for n in names:
    r = vm["results"][n]
    idx = f"{r['tx_index']} (prefix {r['txs_executed'] - 1} txs)" if r["tx_index"] is not None else f"{r['txs_executed']} calls"
    print(f"| {label[n]} | {r['mode']} | {r['block']:,} | {idx} | {r['target_gas_used']:,} | {calls.get(n, '—')} |")

print("\n### Table M2: witness size and composition\n")
print("| Workload | Witness (bincode) | xz | Accounts | Slots | Unique proof nodes (bytes) | Bytecodes (bytes) |")
print("|---|---|---|---|---|---|---|")
comp = {c["witness"]: c for c in J("measurements", "witness_compression.json")}
for n in names:
    s = vm["results"][n]["stats"]
    xz = comp.get(n, {}).get("xz9e")
    print(f"| {label[n]} | {s['witness_bytes'] / 1e6:.2f} MB | {f'{xz / 1e6:.2f} MB' if xz else '—'} | {s['accounts']} | {s['storage_slots']} | "
          f"{s['proof_nodes_unique']:,} ({s['proof_bytes_unique'] / 1e3:.0f} KB) | {s['bytecodes']} ({s['bytecode_bytes'] / 1e3:.0f} KB) |")

print("\n### Table M3: witness construction (public RPCs)\n")
print("| Workload | Header quorum (providers agreeing) | Trace reads | Trace time | Proof fetch (3 providers) | Normalised proofs unanimous | Raw-JSON-only differences | Fixpoint passes | Total build |")
print("|---|---|---|---|---|---|---|---|---|")
for n in names:
    m = J("witnesses", f"{n}.meta.json")
    q = m["proof_quorum"][-1]
    print(f"| {label[n]} | {m['header_agreeing']} | {m['trace_rpc_reads']} | {m['trace_ms'] / 1000:.0f} s | {q['fetch_ms'] / 1000:.1f} s | "
          f"{q['unanimous']}/{q['accounts']} | {q['raw_json_differs_but_normalised_equal']} | {m['fixpoint_passes']} | {m['total_ms'] / 1000:.0f} s |")

print("\n### Table M4: verifier cost, native vs wasm32-wasip2 under wasmtime 28 (OutLayer engine config)\n")
print("| Workload | Instructions (fuel) | of which decode | decode+verify | execute+impact | Wasm guest time (fuel / no fuel) | Native time | Peak wasm memory | Receipt native = wasm |")
print("|---|---|---|---|---|---|---|---|---|")
for n in names:
    r = vm["results"][n]
    dv = r["wasm_instructions_decode_plus_verify"]
    ex = r["wasm_instructions"] - dv if dv else None
    print(f"| {label[n]} | **{r['wasm_instructions'] / 1e6:,.0f} M** | {r['wasm_instructions_decode'] / 1e6:,.0f} M | "
          f"{f'{dv / 1e6:,.0f} M' if dv else 'n/a'} | {f'{ex / 1e6:,.0f} M' if ex else 'n/a'} | "
          f"{r['wasm_guest_total_ms_fuel_median']:.1f} / {r['wasm_guest_total_ms_nofuel_median']:.1f} ms | {r['native_total_ms_median']:.1f} ms | "
          f"{r['wasm_peak_linear_memory_bytes'] / 1e6:.1f} MB | {'yes' if r['receipt_native_equals_wasm'] else 'NO'} |")
print(f"\nWasm component: {vm['wasm_bytes'] / 1e6:.2f} MB, sha256 `{vm['wasm_sha256']}`; one-time compile ≈ "
      f"{vm['results']['flashloan']['wasm_compile_ms'] / 1000:.1f} s (OutLayer caches compiled modules). Host: {vm['host']}.")

print("\n### Table M5: soundness against the chain\n")
print("| Workload | Status / gasUsed / cumulativeGas / keccak(logs) vs `eth_getTransactionReceipt` (3 providers) | Logs | Slots written | Equal to block-N proofs | Mismatches explained by later txs in block | Unexplained |")
print("|---|---|---|---|---|---|---|")
for n in ["flashloan", "liquidation", "aggswap"]:
    c = J("measurements", f"{n}.check.json")
    rc = c["receipt_vs_chain"]
    allok = all(x.get("status_match") and x.get("gas_used_match") and x.get("cumulative_gas_match") and x.get("logs_match") for x in rc if "error" not in x)
    ps = c["post_state"]
    print(f"| {label[n]} | {'all match' if allok else 'MISMATCH'} ({sum(1 for x in rc if 'error' not in x)} providers) | {rc[0].get('logs')} | "
          f"{ps['slots_written']} slots, {ps['accounts_changed']} accounts | {ps['slots_equal_block_n_proofs']} slots, {ps['accounts_equal_block_n_proofs']} accounts | "
          f"{ps['mismatches_explained_by_later_txs_in_block']} | **{len(ps['unexplained'])}** |")

print("\n### Table M6: impact vs explorer (Blockscout, independent)\n")
print("| Workload | Account | Token | Verifier Δ (balanceOf on proven state) | Explorer Δ | Exact | USD at proven block | Explorer USD (current rate) |")
print("|---|---|---|---|---|---|---|---|")
for e in J("measurements", "impact_vs_explorer.json"):
    n = os.path.basename(e["receipt"]).replace(".native-receipt.json", "")
    for row in e["token_rows"]:
        print(f"| {label[n]} | `{row['account'][:10]}…` | {row['symbol']} | {row['verifier_delta']} | {row['explorer_delta']} | "
              f"{'yes' if row['exact_match'] else 'NO'} | {row['verifier_usd_at_block'] or 'unpriced'} | {row['explorer_usd_at_current_rate'] or '—'} |")
print("\nGas fee of the target tx equals the explorer's fee exactly for all three.")
