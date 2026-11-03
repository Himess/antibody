# How mainnet works

There is no mainnet deployment and no mainnet transaction is ever sent. This page explains what a
mainnet program would change, and checks the one piece a mainnet program's trust rests on that can be
checked today, with view calls only: NEAR mainnet's Ethereum light client.

## What stays the same

- near-antibody's contracts and their settlement rules (docs/spec.md, docs/properties.md).
- The verifier build, pinned by its hash, and its reproducible build (`make verifier-wasm`).
- Sealed proofs, T1 quotes verified on NEAR, rounds and the release gate.

## What changes

| Part | Testnet | Mainnet |
|---|---|---|
| The anchor's check | `client-eth2.sepolia.testnet` (one relayer) | `client-eth2.bridge.near`: Ethereum mainnet's finalized chain, from BLS-verified sync-committee updates |
| Verification | OutLayer on testnet | OutLayer's mainnet workers; their TDX quotes verified on NEAR mainnet |
| Pauses and releases | Chain Signatures on testnet; adapters on Sepolia and Base Sepolia | Chain Signatures on NEAR mainnet; the program's adapters on Ethereum and other EVM chains |
| Payouts | wNEAR (`wrap.testnet`) | the NEP-141 token the program commits |

**Latency.** A program's anchor must be final on Ethereum (about 13 minutes) and held by the light
client before the program can be created on it; the check below shows how far the client trails.

## The live check

```bash
antibody mainnet-check        # view calls on NEAR mainnet, eth_getBlockByNumber on Ethereum mainnet
```

It reads the latest Ethereum block `client-eth2.bridge.near` holds (`last_block_number`), that block's
hash there (`block_hash_safe`), the same block's hash from a public Ethereum mainnet RPC, and
Ethereum's finalized head. The API serves the same at `GET /v1/mainnet`.

Recorded on 2026-11-03:

| Field | Value |
|---|---|
| `last_block` | 26,331,101 |
| `hash_on_near` | `0x1e6952785f2b93e54a731268f46196e3a5c0cbdf6424cb7f509c61535c4e0060` |
| `hash_on_ethereum` | the same (`ethereum-rpc.publicnode.com`, `1rpc.io/eth`) |
| `lag_blocks` | 0 (Ethereum's finalized head was 26,331,101) |
| `transactions_sent` | 0 |
