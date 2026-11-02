# T1 test vectors (public data, recorded 2026-11-02)

These are real, public values used by `contracts/near-antibody` (`t1_accepts_a_real_quote_only_for_the_run_it_binds`)
and documented in docs/trust.md. Nothing here is secret.

| File | Source |
|---|---|
| `outlayer-attestation-request-6496.json` | `GET https://testnet-api.outlayer.ai/attestations/by-request/6496`: OutLayer's per-execution attestation (TDX v4 quote, 5,006 B) for testnet request 6496 |
| `collateral.borsh` | `worker.outlayer.testnet.get_collaterals` (newest entry; `tcb_info` `nextUpdate` 2026-11-08), converted to the borsh `Collateral` that `verify_quote` takes |
| `verify_quote-result.borsh` | `tee-verifier-2026-07-22.testnet.verify_quote(quote, collateral)`, called as a view at testnet block 275,887,354: `Verified`, status `UpToDate` |
| `approved-measurement-109.json` | Entry #109 of `worker.outlayer.testnet.get_approved_measurements`: the quote's MRTD and RTMR0–3 |

Reproduce:

```bash
cargo build --release -p antibody-cli
./target/release/antibody --network testnet tee check --request-id 6496
```

The test contract stands in for that request's caller so every binding check runs on real values:
- the wasm hash;
- the input and output hashes;
- the caller;
- the secrets reference;
- the `report_data` recomputed from the task fields.
