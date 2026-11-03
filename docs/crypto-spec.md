# Antibody cryptography specification

Every cryptographic object of a v2 program: keys, sealed submissions, commitments, tags, the project
wrap, disclosure, and the hashes that bind a run. The verifier (`crates/verifier`: `keys.rs`,
`sealed.rs`, `tee.rs`), the CLI and near-antibody implement exactly this; the golden vectors
(`golden/vectors.json`) pin the commitments and derived addresses on both sides. "‖" is byte
concatenation; strings are UTF-8, hashed without a terminator; integers are big-endian.

## 1. Goals

| # | Property | Mechanism |
|---|---|---|
| G1 | A proof stays confidential until disclosure, except to the TEE and, once paid, to the project | Sealed blob (§4), release wrap (§6) |
| G2 | Priority is the commit's order and cannot be stolen or front-run | A commitment binding program, round and hunter (§5); program and hunter inside the AEAD (§4.2) |
| G3 | "Same rule, same root cause" is decidable before disclosure without revealing the function | Tags under the program's key (§7.1) |
| G4 | After disclosure, anyone can recompute every verdict and tag, duplicates included | Per-tag disclosure keys (§7.2); deterministic receipts (§8) |
| G5 | The facts reaching NEAR come from the pinned verifier build | OutLayer's build- and caller-bound key (§3.1), the request hash in the facts, and T1 (§9) |

## 2. Primitives

| Primitive | Instance | Notes |
|---|---|---|
| Hash | SHA-256 | Every hash here, unless it says keccak-256 |
| MAC / PRF | HMAC-SHA256 (RFC 2104) | |
| KDF | HKDF-SHA256 (RFC 5869) | |
| Key agreement | X25519 (RFC 7748) | Non-contributory shared secrets are refused |
| AEAD | ChaCha20-Poly1305 (RFC 8439) | 96-bit nonce, 128-bit tag |
| EVM hashes | keccak-256 | Only where Ethereum defines them, and for EVM addresses (§8) |

## 3. Keys

### 3.1 The root key

The verifier wasm declares one OutLayer encryption key in its `outlayer.manifest` section:
`{"path": "antibody", "bind": "wasm", "caller": "predecessor"}`. OutLayer's keystore derives it inside
its TEE from the build's SHA-256 and the account that called OutLayer, so only this exact build, called
by near-antibody, ever holds it. There is no secret row, owner or access rule to change. The guest
never sees the key: it asks the host for `mac(data)`, HMAC-SHA256 under a key derived for MACs alone.

The verifier also refuses, before deriving anything, a request whose `program_key` does not start with
`<caller>/`: a build called by another account derives other keys and would not serve this program.

Natively and in the local wasm runner, a TEST root key stands in for the host
(`mac = HMAC(SHA-256("antibody/TEST-ONLY-root/never-use-in-production"), data)`), so local outputs are
reproducible and are never mistaken for TEE outputs.

### 3.2 Per-program keys

`program_key = "<near-antibody account>/<program id>"`. Every derivation is domain-separated:

```
label(l, rest) = l ‖ 0x00 ‖ u32(len(program_key)) ‖ program_key ‖ rest

seal_sk     = mac(label("antibody/v2/seal", ""))                    X25519 secret (clamped)
seal_pk     = X25519(seal_sk, 9)
key_epoch   = hex(SHA-256(seal_pk)[0..8])
tag(s, g)   = mac(label("antibody/v2/tag", u8(s) ‖ g))               §7.1
disclose(t, r) = mac(label("antibody/v2/disclose", t ‖ u32(r)))     §7.2, one key per tag and round
```

- **Activation.** The `activate` run outputs `seal_pk`; near-antibody records it, and the program's
  `key_epoch`, only from a T1-confirmed activation. Hunters seal to `seal_pk`.
- **Epochs.** Every later request carries the program's `key_epoch`. A run whose build derives another
  epoch stops with `InfraError` (never `InvalidPoC`), so a key change can never forfeit a bond.

### 3.3 Project key

The project commits an X25519 public key at creation; near-antibody and the verifier refuse low-order
points. `antibody keygen x25519 --nep413` derives it from the project account's NEAR key (NEP-413):

```
message   = "Antibody project key v2. Contract: <near-antibody>. Project: <project>. Sign this only in
             the antibody CLI: the signature decrypts every finding paid to you."
nonce     = SHA-256("antibody/v2/project-key-nonce|" ‖ message)
signature = ed25519(SHA-256(u32_le(2^31 + 413) ‖ borsh(message, nonce, recipient = <near-antibody>, callback = None)))
sk_p      = HKDF(ikm = signature, salt = "antibody/v2/project-key", info = message, L = 32)   (X25519-clamped)
```

ed25519 signatures are deterministic, so the same NEAR key re-derives the same project key, and a
wallet's NEP-413 `signMessage` over the same three fields gives the same signature. The key file is the
export. Whoever holds the signature can read every finding paid to the project, hence the message's
warning: sign it nowhere else. `antibody keygen x25519` without `--nep413` draws a random key instead.
`rotate_project_key` changes the key for later wraps.

### 3.4 Per-submission secrets

- **Content key κ:** 32 bytes from the hunter's CSPRNG.
- **Sender salt:** 32 bytes, chosen by the hunter and kept secret until disclosure. It derives the
  proof's fresh senders (§8).

## 4. Sealed blob (`ABSEAL1`)

### 4.1 Wire format

```
blob   = "ABSEAL1\0"              (8 bytes)
       ‖ wrap_v                   (92 bytes, §4.3)
       ‖ nonce_b                  (12 bytes, random)
       ‖ AEAD_κ(nonce_b, P, ad = "antibody/sealed/v1" ‖ program_key)
```

A blob is revealed inline (at most 15 KB) or by location: a FastFS upload, a NEAR transaction named
`near-tx:<hash>`, with the blob's SHA-256. The TEE tries each of its archival RPCs until one serves bytes matching the hash, and accepts
only bytes matching the committed hash; an unavailable or mismatched blob is the RPC's fault
(`InfraError`).

### 4.2 Payload `P`

```
P = "ABPOC1"                                      (6 bytes)
  ‖ u16(len(program_key)) ‖ program_key
  ‖ u16(len(hunter))      ‖ hunter                (the committing NEAR account)
  ‖ u32(len(envelope))    ‖ envelope              (the witness envelope, bincode)
  ‖ u32(len(report))      ‖ report                (free text, never parsed)
```

The TEE refuses a payload whose `program_key` or `hunter` differs from the request with `InvalidPoC`.
A ciphertext is public once revealed; without this binding a copycat could commit someone else's blob
under its own account, and without κ it cannot re-encrypt one. Trailing bytes are refused.

### 4.3 Key wrap to the program (`wrap_v`)

```
e       = 32 random bytes;   epk = X25519(e, 9);   shared = X25519(e, seal_pk)
kek     = HKDF(ikm = shared, salt = epk ‖ seal_pk, info = "antibody/wrap-v/v1" ‖ program_key, L = 32)
nonce_w = 12 random bytes
wrap_v  = epk (32) ‖ nonce_w (12) ‖ AEAD_kek(nonce_w, κ, ad = "antibody/wrap-v/v1") (48)      = 92 bytes
```

The TEE unwraps κ with `seal_sk`, then opens the body. A blob sealed to another program's key, or to
another epoch's, does not open.

## 5. Commitment

```
commitment = SHA-256("antibody/commit/v2"
                     ‖ u32(len(program_key)) ‖ program_key
                     ‖ u32(round)
                     ‖ u32(len(hunter)) ‖ hunter
                     ‖ SHA-256(blob))
```

1. **Commit.** `commit(program_id, round, commitment, terms_sha256)` locks the bond. Priority is the
   commit's position in near-antibody's sequence: NEAR's execution order.
2. **Reveal.** `reveal(commit_id, blob)` recomputes the commitment from the commit's program, round and
   hunter and the blob's hash (inline bytes are hashed on chain; a location carries its hash), and
   refuses any mismatch. Anyone may reveal: only the committed blob matches, and only the hunter could
   have sealed it for that hunter (§4.2).

- **Hiding.** κ, `e` and the nonces are random, so a commitment reveals nothing about the proof.
- **Binding.** It fixes the blob, so the content, and the account and round.
- **Front-running.** Copying a revealed blob does not help (§4.2), and a commitment shows nothing to copy.

## 6. Release wrap (the project reads what it paid for)

The verify run emits no wrap: its facts are public in OutLayer's resolve transaction, and a wrap there
would let a project read a duplicate's proof for free. After payment, a `release_wrap` run, for paid
first findings and promoted duplicates only, re-encrypts κ to the project's key:

```
e_p   = HKDF(ikm = κ, salt = pk_p, info = "antibody/wrap-p-eph/v1", L = 32)      X25519 secret (clamped)
epk_p = X25519(e_p, 9);   shared = X25519(e_p, pk_p)
kn    = HKDF(ikm = shared, salt = epk_p ‖ pk_p, info = "antibody/wrap-p/v1" ‖ program_key, L = 44)
wrap  = epk_p (32) ‖ AEAD_{kn[0..32]}(kn[32..44], κ, ad = "antibody/wrap-p/v1") (48)          = 80 bytes
```

- **Deterministic.** `e_p` comes from κ, secret and unique per submission, so the run's output is a pure
  function of its inputs and each `(e_p, pk_p)` pair encrypts one message.
- **Reading it.** `antibody program read --commit N --project-key <file>` requests the wrap the first time,
  unwraps κ, opens the blob (from its reveal transaction, a file or an Antibody API, checked against the
  committed hash) and re-runs the proof locally: its receipt hash equals the one the TEE committed to.

## 7. Tags and disclosure

### 7.1 Tags

For each broken slot `s`, the verifier records the protocol-entry call `(target, selector)` at whose end
the slot last flipped from holding to broken. Its group bytes are the committed group, if the manifest
assigns that entry point to one, else the entry point itself:

```
g = "g" ‖ u16(group id)              (a committed group)
g = "f" ‖ target (20) ‖ selector (4) (otherwise; an unattributed flip uses zero bytes)
key = (s, tag(s, g))
```

- **Before disclosure.** `tag` is a PRF under a key nobody holds, so tags reveal only equality. The slot
  is public: it sets the severity.
- **Equality.** Equal keys mean the same slot broke at the same root cause, so the later commit is a
  duplicate.
- **Primary tag.** The tag of the highest-severity broken slot (a verdict carries at most four keys).

### 7.2 Disclosure keys

A verify or second run of a commit of round `r` whose verdict has a primary tag `t` also wraps κ under
that tag's disclosure key for round `r`:

```
nonce_d = SHA-256("antibody/v2/wrap-d" ‖ u64(commit_id) ‖ t)[0..12]
wrap_d  = nonce_d ‖ AEAD_{disclose(t, r)}(nonce_d, κ, ad = program_key)
```

`wrap_d` is public in the facts, and useless until `disclose(t, r)` is. A `disclose` run, which
near-antibody requests (`request_disclosure(program_id, [{round, tag}])`, at most eight items) only once
each item's condition holds (spec §11.6), outputs `disclose(t, r)` for each. One key then opens every
proof of `t` committed in round `r`, duplicates included, and nothing of a later round: a new bug behind
the same entry point in round `r + 1` stays sealed under its own key. Anyone can then:
1. fetch each blob and open it with κ;
2. re-run the verifier locally on the envelope;
3. recompute the receipt, its hash and every tag's root cause, and the priority order.

## 8. Hashes and addresses that bind a run

| Object | Definition |
|---|---|
| Request hash | `SHA-256(request bytes)`: near-antibody keeps it with the run; the facts repeat it, and a T1 quote binds it (§9) |
| Manifest hash | SHA-256 of the canonical manifest JSON; the TEE judges under the manifest whose bytes hash to the request's, never a witness's copy |
| Witness hash | SHA-256 of the sealed envelope bytes, in the receipt |
| `poc_hash` | `SHA-256("antibody/poc/v1" ‖ bincode(bundle))`: binds the calls and the sender salt; a re-run must carry the original's |
| Receipt hash | SHA-256 of the receipt's canonical bytes (compact JSON in struct order); the receipt itself stays sealed until disclosure |
| Senders | `keccak256("antibody/sender/v1" ‖ salt ‖ u8(i))[12..]`: fresh by construction, never read from the anchor |
| Reserved rules address | `keccak256("antibody/invariants/v2" ‖ program_key ‖ anchor hash)[12..]`: the rules run there; nobody holds its key and no CREATE reaches it |
| Fork-mode deployer | `keccak256("antibody/deployer/v1")[12..]`: the sender of a manifest's deployment transactions |
| Activation receipt | `SHA-256("antibody/v2/activation" ‖ program_key ‖ manifest hash ‖ seal_pk)` |
| Round-check receipt | `SHA-256("antibody/v2/round-check" ‖ SHA-256(round manifest) ‖ u8(ok) ‖ u8(canaries_ok))` |
| Distinctness receipt (the top-up is off, spec §7.5) | `SHA-256("antibody/v2/distinct" ‖ SHA-256(patch) ‖ u64(commit) ‖ u64(earlier commit) ‖ u8(distinct))` |
| Disclosure receipt | `SHA-256("antibody/v2/disclose" ‖ (hex tag ‖ u32(round)) for each item)` |

## 9. Tiers (who vouches for the facts)

| Tier | Mechanism | What a relying party trusts |
|---|---|---|
| T0 | OutLayer's worker output, delivered on chain by its operator; the wasm pinned by hash | OutLayer's operator, until T1 or the challenge window |
| T1 | The run's TDX quote verified on NEAR through `confirm_tee`: its `report_data` binds the wasm hash, the input and output hashes, the caller and the secrets, and near-antibody recomputes it from the run it requested (docs/trust.md) | Intel TDX, its collateral and the pinned worker images |
| T2 (roadmap) | A proof of the same verifier execution checked on NEAR | Not built |

Every tier relies on the same determinism: after disclosure anyone can re-run the verifier natively on
the disclosed inputs and compare.

## 10. Encodings

- Hashes and keys are lowercase hex without `0x` in facts and contract arguments; EVM values use `0x`.
- Length prefixes and integers are big-endian.
- Facts are compact JSON, at most 1 KB.
