/**
 * The practice try's submission envelope, byte for byte as crates/witness/src/program.rs
 * `build_submission_from_base` encodes it (bincode 1.x legacy, as in base.ts):
 *
 *   Envelope { version: 1, chain_id, claimed_block_hash: anchor hash, quorum,
 *              payload: Payload::Submission (variant 5) { bundle, state: trie::digest_only(state_root), bytecodes: [] } }
 *
 * The bundle is crates/witness/src/build.rs `parse_bundle` applied to practice.rs `tutorial_bundle`.
 * Serde layouts (checked against golden/web-vectors.json):
 * - `U256` (ruint, binary formats): a byte string of its 32 big-endian bytes, u64 length 32 ‖ 32 bytes.
 * - `Option<Address>`: 0x00 for None; 0x01 ‖ u64 length 20 ‖ 20 bytes for Some.
 * - digest-only `EthereumState` (rsp-mpt): `state_trie` = `MptNode { data: MptNodeData::Digest(root) }`
 *   (the node's cached reference is `#[serde(skip)]`): u32 variant 4 ‖ u64 32 ‖ root; then
 *   `storage_tries`, an empty map: u64 0.
 *
 * The CLI also runs the proof locally on the base (`execute_program_witness`) before it seals; the
 * browser does not run the EVM: the tutorial proof's verdict is known (a Violation of slot 0).
 */
import type { BaseWitness, QuorumVote } from "./base";
import { concat, fromHex, fromHexN, need, toHex, u128le, u256be, u32le, u64le, u8, utf8 } from "./bytes";
import { createAddress, senderAddress } from "./sender";

export const WITNESS_VERSION = 1;
/** `Payload::Submission`: variant 5 of Replay, Block, Bundle, Program, Base, Submission. */
export const PAYLOAD_SUBMISSION = 5;
/** rsp-mpt `MptNodeData::Digest`: variant 4 of Null, Branch, Leaf, Extension, Digest. */
export const MPT_DIGEST = 4;

/** practice.rs `TUTORIAL_CALL`: `sweep(0x…dEaD)`, the tutorial proof's one call. */
export const TUTORIAL_CALL = "0x01681a62000000000000000000000000000000000000000000000000000000000000dead";

/** build.rs `BundleFileCall`: a bundle file's call (JSON). */
export interface BundleFileCall {
  sender: number;
  /** An address; null or absent (serde's missing `Option`) for a contract creation. */
  to?: string | null;
  value: string;
  data: string;
  gas_limit: number;
}

/** build.rs `BundleFile`: a bundle's JSON form. */
export interface BundleFile {
  senders: number;
  endowment_wei: string;
  protected?: string[];
  calls: BundleFileCall[];
}

/** bundle.rs `BundleCall`. */
export interface BundleCall {
  sender: number;
  /** null = contract creation. */
  to: Uint8Array | null;
  value: bigint;
  data: Uint8Array;
  gasLimit: bigint;
}

/** bundle.rs `Bundle`. */
export interface Bundle {
  senderSalt: Uint8Array;
  senders: number;
  endowmentWei: bigint;
  calls: BundleCall[];
}

/** practice.rs `tutorial_bundle`: the tutorial proof against one vault. */
export function tutorialBundle(vault: string): BundleFile {
  return {
    senders: 1,
    endowment_wei: "1000000000000000",
    protected: [],
    calls: [{ sender: 0, to: vault, value: "0", data: TUTORIAL_CALL, gas_limit: 100_000 }],
  };
}

const MAX_U256 = (BigInt(1) << BigInt(256)) - BigInt(1);
const MAX_U128 = (BigInt(1) << BigInt(128)) - BigInt(1);

/** ruint's `U256::from_str`: `0x`, `0o` or `0b` prefixes, else decimal; `_` ignored; overflow refused. */
export function parseU256(s: string): bigint {
  const pre = s.slice(0, 2).toLowerCase();
  const radix = pre === "0x" ? 16 : pre === "0o" ? 8 : pre === "0b" ? 2 : 10;
  const digits = radix === 10 ? s : s.slice(2);
  let n = BigInt(0);
  for (const c of digits) {
    if (c === "_") continue;
    const d = parseInt(c, 36);
    if (!/^[0-9a-zA-Z]$/.test(c) || d >= radix) throw new Error(`U256: invalid digit ${JSON.stringify(c)}`);
    n = n * BigInt(radix) + BigInt(d);
    if (n > MAX_U256) throw new Error("U256: overflow");
  }
  return n;
}

/** Rust's `u128::from_str`: decimal digits, an optional leading `+`, at least one digit, no overflow. */
export function parseU128(s: string): bigint {
  if (!/^\+?[0-9]+$/.test(s)) throw new Error(`u128: invalid ${JSON.stringify(s)}`);
  const n = BigInt(s.replace("+", ""));
  if (n > MAX_U128) throw new Error("u128: overflow");
  return n;
}

function u8Field(n: number, what: string): number {
  if (!Number.isInteger(n) || n < 0 || n > 255) throw new Error(`${what} is not a u8`);
  return n;
}

/**
 * build.rs `parse_bundle`: a bundle file under `salt`. Calldata may hold `5e…5e` (20 bytes) for sender 0
 * and `5f…5f` for sender 1, and `5c…5c` (19 bytes) followed by a byte k for the contract call k creates,
 * which `to` may name the same way.
 */
export function parseBundle(f: BundleFile, salt: Uint8Array): Bundle {
  need(salt, 32, "sender salt");
  const senders = u8Field(f.senders, "senders");
  // Every call is a transaction from a fresh sender, so call k creates at its sender's count of earlier calls.
  const nonces: bigint[] = Array.from({ length: senders }, () => BigInt(0));
  const created: (Uint8Array | null)[] = [];
  for (const c of f.calls) {
    const s = u8Field(c.sender, "a call's sender");
    if (s >= senders) throw new Error(`call from sender ${s} of ${senders}`);
    created.push(c.to === null || c.to === undefined ? createAddress(senderAddress(salt, s), nonces[s]) : null);
    nonces[s] += BigInt(1);
  }
  const creation = (k: number, at: number): Uint8Array => {
    const a = created[k];
    if (a === undefined || a === null) throw new Error(`call ${at} names the contract call ${k} creates, but call ${k} creates none`);
    return a;
  };
  const placeholders: [string, number][] = [
    ["5e".repeat(20), 0],
    ["5f".repeat(20), 1],
  ];
  const prefix = "5c".repeat(19);
  const calls = f.calls.map((c, at): BundleCall => {
    let data = c.data;
    while (data.startsWith("0x")) data = data.slice(2); // `trim_start_matches("0x")`
    data = data.toLowerCase();
    for (const [ph, i] of placeholders) if (i < senders) data = data.split(ph).join(toHex(senderAddress(salt, i)));
    for (let p = 0; p + 40 <= data.length; ) {
      if (data.slice(p, p + 38) === prefix) {
        const kHex = data.slice(p + 38, p + 40); // `usize::from_str_radix(…, 16)`
        if (!/^(\+[0-9a-f]|[0-9a-f]{2})$/.test(kHex)) throw new Error(`call ${at}: invalid created-contract index`);
        data = data.slice(0, p) + toHex(creation(parseInt(kHex, 16), at)) + data.slice(p + 40);
        p += 40;
      } else {
        p += 2;
      }
    }
    let to = c.to === null || c.to === undefined ? null : fromHexN(c.to, 20, `call ${at}'s to`);
    if (to !== null && to.subarray(0, 19).every((b) => b === 0x5c)) to = creation(to[19], at);
    if (!Number.isSafeInteger(c.gas_limit) || c.gas_limit < 0) throw new Error(`call ${at}: gas_limit is not a u64`);
    const value = parseU256(c.value);
    // `hex::decode`: no prefix left to strip (a "0X" was not trimmed above, so it fails here).
    if (!/^([0-9a-f]{2})*$/.test(data)) throw new Error(`call ${at}: data is not hex`);
    return { sender: c.sender, to, value, data: fromHex(data), gasLimit: BigInt(c.gas_limit) };
  });
  return { senderSalt: salt, senders, endowmentWei: parseU128(f.endowment_wei), calls };
}

/** bincode of a byte string (`Bytes`, `B256`, `Address`, `String`): u64 length ‖ bytes. */
function bytesField(b: Uint8Array): Uint8Array {
  return concat(u64le(BigInt(b.length)), b);
}

/** bincode of bundle.rs `Bundle`. */
export function encodeBundle(b: Bundle): Uint8Array {
  const parts = [bytesField(need(b.senderSalt, 32, "sender salt")), u8(b.senders), u128le(b.endowmentWei), u64le(BigInt(b.calls.length))];
  for (const c of b.calls) {
    const to = c.to === null ? u8(0) : concat(u8(1), bytesField(need(c.to, 20, "to")));
    parts.push(u8(c.sender), to, bytesField(u256be(c.value)), bytesField(c.data), u64le(c.gasLimit));
  }
  return concat(...parts);
}

/** bincode of trie.rs `digest_only(root)`: the partial trie that proves nothing, the root's digest alone. */
export function digestOnlyState(stateRoot: Uint8Array): Uint8Array {
  return concat(u32le(MPT_DIGEST), bytesField(need(stateRoot, 32, "state root")), u64le(BigInt(0)));
}

/** bincode of an `Envelope` whose payload is `Payload::Submission { bundle, state: digest_only(stateRoot), bytecodes: [] }`. */
export function encodeSubmissionEnvelope(o: {
  chainId: bigint;
  anchorHash: Uint8Array;
  quorum: QuorumVote[];
  bundle: Bundle;
  stateRoot: Uint8Array;
}): Uint8Array {
  const parts = [u32le(WITNESS_VERSION), u64le(o.chainId), bytesField(need(o.anchorHash, 32, "anchor hash")), u64le(BigInt(o.quorum.length))];
  for (const q of o.quorum) parts.push(bytesField(utf8(q.source)), bytesField(need(q.blockHash, 32, "quorum block hash")));
  parts.push(u32le(PAYLOAD_SUBMISSION), encodeBundle(o.bundle), digestOnlyState(o.stateRoot), u64le(BigInt(0)));
  return concat(...parts);
}

/**
 * program.rs `build_submission_from_base` for the tutorial proof against `vault` (a `0x` address) under
 * `salt`: the envelope the CLI seals (363 bytes for the practice program's base).
 */
export function buildPracticeEnvelope(base: BaseWitness, vault: string, salt: Uint8Array): Uint8Array {
  return encodeSubmissionEnvelope({
    chainId: base.chainId,
    anchorHash: base.claimedBlockHash,
    quorum: base.quorum,
    bundle: parseBundle(tutorialBundle(vault), salt),
    stateRoot: base.stateRoot,
  });
}
