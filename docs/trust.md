# Trust table

What you have to trust, and how to check it, for each part of Antibody. "Pre-launch" is the real testnet
deployment. "Live (MOCK)" describes the mainnet design, which the API shows only as recorded data with a
MOCK badge. No mainnet transactions are sent.

| Component | Pre-launch (testnet, real) | Live (MOCK) | What you trust | How to check it yourself |
|---|---|---|---|---|
| Anchor block hash | `client-eth2.sepolia.testnet` checks `block_hash_safe(M) == H` at program creation; otherwise the program is labelled *project-attested* | `client-eth2.bridge.near` (sync committee); the v0.3 forged-header test was rejected there | **Testnet:** the light client's single trusted relayer. **Live:** Ethereum's sync committee | Compare `anchor_trust` in `get_program` with any Sepolia RPC |
| State at the anchor | MPT proofs verified inside the verifier | Same | Nothing (hashes) | `antibody check` / `antibody verify` re-run it |
| EVM semantics | revm 38 + reth v2.2.0 under the program's committed `evm_spec` (Osaka) | Same | That the pinned EVM matches the committed rules | Native and wasm runs are byte-identical; the receipt hash is reproducible |
| Who runs the verifier | OutLayer TEE worker, wasm pinned by sha256 (`WasmUrl`) | Same | **T0:** the OutLayer operator delivers the worker's output. **T1:** Intel TDX quote. **T2:** zk (roadmap) | Above the instant cap: `challenge()` during the delay. After disclosure: recompute every verdict |
| Verifier key (sealed PoCs) | OutLayer keystore `PROTECTED_ANTIBODY_SEED`, bound to the wasm hash and to near-antibody as caller | Same | The keystore TEE and its governance (see OutLayer's documentation) | The reproducible build shows the key is never printed; `pubkey` mode output |
| Fallback key (if the keystore is unavailable) | Manual secret known to the deployer, labelled **weaker** | — | The deployer | — |
| PoC confidentiality | TEE until disclosure; the project sees Violations only | Same | TEE + keystore | Disclosure publishes every content key; check that nothing leaked earlier |
| Settlement rules | near-antibody (immutable per program) | Same, with NEAR Intents payouts | NEAR consensus | Read the contract, call `get_program` |
| Payout asset | tUSD (testnet token, no value) | NEAR Intents (shown as MOCK) | — | `ft_balance_of` |
| Guardian | near-guardian + `v1.signer-prod.testnet` (11 of 16) | `v1.signer` | MPC threshold; the contract builds every transaction | `cast decode-transaction` on the signed transaction; `paused()` on each chain |
| Blob storage | FastFS (NEAR transaction arguments) | Same | Nothing (sha256 commitment) | `antibody verify` fetches by hash |
| Relayer / indexer / API / notifier | Informational, permissionless | Same | Nothing | Everything is re-derivable from NEAR |
| Sandbox demo (`make demo`) | MOCK OutLayer, MOCK MPC, TEST keys | — | Nothing (local) | It is a test harness |

## What is mock

- **The mainnet view.** It uses recorded data only, served by `/v1/mock/mainnet` with `"mock": true`.
- **The sandbox demo.** OutLayer and the MPC are replaced there by `mock-outlayer` and `mock-mpc`, resolved by the real verifier (TEST seed) and a TEST secp256k1 key.
- **Test-only keys in fixtures.** The fp_tags in `fixtures/acme-local/expected.json` use a TEST `k_program`.
