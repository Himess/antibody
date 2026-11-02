# Antibody cryptography specification (v1)

This document specifies every cryptographic object in Antibody: keys, sealed submissions,
commitments, fingerprint tags, project wraps, disclosure and receipts. The verifier
(`crates/verifier`), the CLI and the NEAR contract (`contracts/near-antibody`) implement exactly
this. "‖" is byte concatenation. Strings are UTF-8 and are hashed without a terminator.

## 1. Goals

| # | Property | Mechanism |
|---|---|---|
| G1 | A PoC stays confidential until the program's disclosure time, except to the TEE and, for valid findings, to the project | Sealed submission (§4), project wrap (§7) |
| G2 | Priority is decided by commit order and cannot be stolen or front-run | Commit-reveal binding the hunter account (§5); hunter and program inside the AEAD payload (§4.2) |
| G3 | "Same rule, same function" is provable before disclosure without revealing the function | Fingerprint tags (§6) |
| G4 | After disclosure, anyone can recompute every receipt and tag, duplicates included | Disclosure of content keys and `k_program` (§8); deterministic receipts (§9) |
| G5 | The verdict reaching NEAR comes from the pinned verifier build | OutLayer `WasmUrl` hash pin, a `PROTECTED_` secret bound to that build and to the calling contract, and attestation tiers (§10) |

## 2. Primitives

| Primitive | Instance | Notes |
|---|---|---|
| Hash | SHA-256 | Every hash in this document, unless it says keccak-256 |
| MAC / PRF | HMAC-SHA256 (RFC 2104) | |
| KDF | HKDF-SHA256 (RFC 5869) | |
| Key agreement | X25519 (RFC 7748) | |
| AEAD | ChaCha20-Poly1305 (RFC 8439) | 96-bit nonce, 128-bit tag |
| EVM hashes | keccak-256 | Only where Ethereum defines them: header hash, code hash, selectors |

These are the same primitives OutLayer's keystore uses for its secrets (X25519 + HKDF-SHA256 + ChaCha20-Poly1305).

## 3. Keys

### 3.1 Verifier seed S

`S` is 32 bytes held in OutLayer's keystore as the secret `PROTECTED_ANTIBODY_SEED`.

- **Generation:** the keystore generates it inside its TEE (`PROTECTED_` prefix). Nobody, including the deployer, ever sees its value.
- **Access condition:** `And[WasmHash{h}, Predecessor{Whitelist[near-antibody]}]`. Only the pinned verifier build `h` can read it, and only when that build is invoked through the near-antibody contract.
- **Read path:** the guest reads `S` from its environment. It never prints `S` or any key derived from it, except the public key (§3.2) and, at disclosure, `k_program` and the content keys (§8). The reproducible build lets anyone check this.
- **Fallback (labelled "weaker" in the trust table):** a manually stored secret, whose value the developer knows.

### 3.2 Derived keys

```
sk_v      = HKDF(ikm = S, salt = "antibody/v1", info = "antibody/verifier-x25519/v1", L = 32)   (X25519-clamped)
pk_v      = X25519(sk_v, 9)
k_program = HMAC(S, "antibody/k_program/v1" ‖ program_key)                                       (32 bytes)
```

- `program_key` is the globally unique program identifier `"<near-antibody account>/<program id>"`.
- `pk_v` is published by a `pubkey` run of the verifier (`{"mode":"pubkey"}`). The contract stores it, and every program pins it at creation. Programs are therefore bound to one verifier build.

### 3.3 Project key

- The project generates an X25519 key pair locally (`antibody keygen`).
- `pk_p` (32 bytes, hex) is committed in `create_program`.
- `sk_p` never leaves the project.

### 3.4 Per-submission secrets

- **Content key `K`:** 32 bytes from the hunter's CSPRNG.
- **Sender salt:** 32 bytes inside the bundle (`sender_salt`). It derives the fresh sender addresses `keccak256("antibody/sender/v1" ‖ salt ‖ i)[12..]`.

## 4. Sealed submission (`ABSEAL1`)

### 4.1 Wire format

```
blob    = "ABSEAL1\0"            (8 bytes, magic + version)
        ‖ wrap_v                 (92 bytes, §4.3)
        ‖ nonce_b                (12 bytes, random)
        ‖ AEAD_K(nonce_b, P, ad_b)
ad_b    = "antibody/sealed/v1" ‖ program_key
```

`ciphertext_sha256 = SHA-256(blob)`. The blob may be published anywhere: FastFS, a GitHub raw URL, a NEAR transaction. It is fetched by URL and accepted only if its hash matches.

### 4.2 Payload `P`

```
P = "ABPOC1"                     (6 bytes)
  ‖ u16be(len(program_key)) ‖ program_key
  ‖ u16be(len(hunter))      ‖ hunter        (NEAR account id of the committing account)
  ‖ u32be(len(envelope))    ‖ envelope      (verifier witness envelope, bincode; contains the bundle,
                                             the manifest JSON and the invariants code)
  ‖ u32be(len(report))      ‖ report        (optional free text; never parsed or used by the verifier)
```

- **Why the hunter and program are inside the AEAD.** A ciphertext is public once uploaded. Without this binding, a copycat could commit to someone else's ciphertext under their own account. The TEE rejects any payload whose `program_key` or `hunter` differs from the request with `InvalidPoC`. A copycat cannot re-encrypt without `K`.

### 4.3 Key wrap to the verifier (`wrap_v`)

```
e        = 32 random bytes;  epk = X25519(e, 9)
shared   = X25519(e, pk_v)
kek      = HKDF(ikm = shared, salt = epk ‖ pk_v, info = "antibody/wrap-v/v1" ‖ program_key, L = 32)
nonce_w  = 12 random bytes
wrap_v   = epk (32) ‖ nonce_w (12) ‖ AEAD_kek(nonce_w, K, ad = "antibody/wrap-v/v1") (48)    = 92 bytes
```

- The contract stores `wrap_v`, the first 92 bytes after the magic, with the commit. A disclosure run then needs no ciphertext downloads.
- The TEE checks that the `wrap_v` given in the request equals the one inside the fetched blob.

## 5. Commitment (priority)

```
commit_hash = SHA-256("antibody/commit/v1|" ‖ program_key ‖ "|" ‖ hunter ‖ "|" ‖ hex(ciphertext_sha256))
```

1. **Commit.** `commit(program, commit_hash)` locks the bond and records the NEAR block height and timestamp.
2. **Reveal.** Within `reveal_window_blocks`, the hunter calls `submit(commit, {url, sha256, wrap_v})`. The contract recomputes `commit_hash` from the predecessor account and `sha256`.
3. **Order.** Priority is the commit's position in the contract's commit sequence. That is the NEAR execution order, so block height first, then receipt order within a block.

Properties:
- **Hiding.** `commit_hash` reveals nothing about the PoC, because `K` and the nonces are random.
- **Binding.** The commit fixes the ciphertext, and therefore its content, and the account.
- **Front-running.** Copying someone else's ciphertext does not help (§4.2). Seeing a commit reveals nothing.

Recommended order of operations: commit, then upload, then submit. The ciphertext is never public before its commit exists.

## 6. Fingerprint tags

For each broken invariant `i`, the verifier records the protocol-entry call `(target, selector)` at whose end invariant `i` last flipped from holding to broken.

```
fp_tag(i) = HMAC(k_program, "antibody/fp/v1" ‖ u8(i) ‖ target (20 bytes) ‖ selector (4 bytes))
```

- **Key.** A finding's key is `(i, fp_tag(i))`.
- **Equality.** Equal keys mean the same invariant broke at the end of a call to the same function of the same contract.
- **Before disclosure.** HMAC is a PRF under the secret `k_program`, so tags reveal only equality. The slot `i` is public, because it determines severity.
- **After disclosure.** `k_program` is published, and anyone can recompute every tag from its receipt.
- **Unattributed flips.** A flip outside any protocol-entry call uses `target = 0x00…00` and `selector = 0x00000000`.

## 7. Project wrap

For a `Violation`, the TEE re-encrypts `K` to the project so that a paying project can always read the report it pays for:

```
e_p      = HKDF(ikm = K, salt = pk_p, info = "antibody/wrap-p-eph/v1", L = 32)   (X25519-clamped; deterministic)
epk_p    = X25519(e_p, 9);  shared = X25519(e_p, pk_p)
kek_p    = HKDF(ikm = shared, salt = epk_p ‖ pk_p, info = "antibody/wrap-p/v1" ‖ program_key, L = 44)
           -> key = kek_p[0..32], nonce = kek_p[32..44]
project_wrap = epk_p (32) ‖ AEAD_key(nonce, K, ad = "antibody/wrap-p/v1") (48)          = 80 bytes
```

- **Deterministic ephemeral key.** `e_p` comes from `K`, which is secret and unique per submission. The TEE output is therefore a pure function of its inputs, so native and TEE outputs can be compared byte for byte.
- **No nonce reuse.** Each `(e_p, pk_p)` key encrypts exactly one message.
- **Who gets it.** Facts carry `project_wrap` (hex) only for `Violation`. NoViolation and InvalidPoC submissions are never disclosed to the project before disclosure time.

## 8. Disclosure

After `deadline + disclosure_after`, anyone may call `disclose(program)`. The contract runs the verifier with:

```
{"mode":"disclose","program_id":program_key,"entries":[{"commit_id":n,"wrap_v":hex}, …]}
```

The TEE unwraps each `wrap_v` with `sk_v` and returns:

```
{"program":program_key,"k_program":hex,"keys":[{"commit_id":n,"content_key":hex}, …]}
```

The contract stores the result. Anyone can then:
1. fetch each blob;
2. decrypt it with its `K`;
3. re-run `antibody verify` locally;
4. recompute the receipt hash, the fp_tags (with `k_program`) and the priority order.

Duplicates become fully checkable: "same rule, same function, committed earlier".

Entries for re-run blobs (§8.1) carry their round: `{"commit_id":n,"round":r,"wrap_v":hex}` in, and
`{"commit_id":n,"round":r,"content_key":hex}` out.

### 8.1 Re-runs (rounds, docs/rounds.md)

When a project proposes round `r` (a fix as code overrides), a claimed finding can be re-run against it:

```
{"mode":"rerun","program_id":program_key,"commit_id":n,"round":r,"hunter":account,
 "manifest_hash":hex(sha256(round-r manifest)),"poc_hash":hex(original verdict's poc_hash),
 "sealed":{"url":…,"sha256":hex,"wrap_v":hex}}
```

- **The blob.** It is a fresh `ABSEAL1` blob whose payload is bound to the original `program_key` and
  `hunter`. Its witness is built at the round-`r` manifest from the same bundle.
- **Who builds it.** The project, from its key wrap, or the finder.
- **TEE checks.** The same blob checks as `verify`. In addition:
  - `poc_hash(bundle)` must equal the request's `poc_hash`, otherwise `InvalidPoC`;
  - the witness's manifest must hash to `manifest_hash`;
  - `round` must be > 0, otherwise the request fails.
- **Output.** Facts as in §9, with no `project_wrap`. near-antibody checks the program, the commit, the
  round's manifest hash and the `poc_hash`, then records which keys the PoC still breaks.

## 9. Receipts and facts

- **Receipt v2.** The full deterministic output: anchor, checks, calls, broken invariants with plaintext fingerprints, and impact. It is encoded as compact JSON in struct order. `receipt_hash = SHA-256(receipt bytes)`. The receipt stays sealed with the PoC until disclosure, and anyone can recompute it after.
- **Facts.** The only TEE output that becomes public, at most 1 KB:
  `{v, program, commit_id, poc_hash, kind, broken_mask, fp_tags, impact_usd_micro, project_wrap, receipt_hash, manifest_hash, wasm_hash}`.
- **`poc_hash`.** `SHA-256("antibody/poc/v1" ‖ bincode(bundle))`. It binds the transactions and the sender salt.
- **`manifest_hash`.** SHA-256 of the canonical manifest JSON. The contract checks it against the program.

## 10. Trust tiers (who vouches for the facts)

| Tier | Mechanism | What a relying party trusts |
|---|---|---|
| T0 (testnet default) | OutLayer worker output, delivered on-chain by the operator account; wasm pinned by `WasmUrl` hash | The OutLayer operator's attested workers |
| T1 | The run's TDX quote, verified on NEAR by `tee-verifier-*.verify_quote` through `confirm_tee`. `report_data` binds the wasm hash, the input and output hashes, the caller and the secrets (docs/trust.md) | Intel TDX, the DCAP collateral and the pinned worker measurements |
| T2 (roadmap) | A zk proof of the same verifier execution | Mathematics only |

All tiers rely on the same determinism: anyone can always re-run the verifier natively from the disclosed inputs. A wrong T0 verdict is detectable after the fact. Before payout, T0 adds an optimistic delay with `challenge()` above a payout cap.

## 11. Encodings

- Hashes and keys: lowercase hex without `0x` in facts and contract arguments. EVM values use `0x` hex.
- `program_key`: `"<near-antibody account id>/<decimal program id>"`.
- Every length prefix is big-endian.
