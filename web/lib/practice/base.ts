/**
 * The practice program's pinned base witness (fixtures/web/practice-base.bin): a bincode `Envelope`
 * whose payload is `Payload::Base` (crates/verifier/src/witness.rs). bincode 1.x legacy configuration:
 * little-endian fixed-width integers, a u64 length before every Vec, String and byte string, enum
 * variants as u32; alloy's `B256` and `Bytes` serialize as byte strings (u64 length ‖ bytes).
 *
 * Only what the browser needs is read: the envelope header, the manifest, the invariants code and the
 * anchor header. The ancestors, state and bytecodes after it stay unread; a program pins the whole
 * file by its SHA-256 (cmd.rs `pinned_base_at`), which `parseBase` checks when given.
 */
import { keccak_256 } from "@noble/hashes/sha3.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { beToBigInt, ByteReader, equalBytes, fromHex, fromHexN, fromUtf8, toHex, toHex0x } from "./bytes";

/** `Payload::Base`: variant 4 of Replay, Block, Bundle, Program, Base, Submission. */
export const PAYLOAD_BASE = 4;

/** witness.rs `QuorumVote`: one RPC source's view of the anchor hash. */
export interface QuorumVote {
  source: string;
  blockHash: Uint8Array;
}

/** A manifest target (`targets[]`): the practice program's are `factory` and `vault-0` … `vault-23`. */
export interface ManifestTarget {
  name: string;
  address: string;
  code_hash?: string;
}

/** The manifest JSON (antibody.manifest.v2); only `targets` is read here. */
export interface Manifest {
  targets: ManifestTarget[];
  [field: string]: unknown;
}

export interface BaseWitness {
  /** SHA-256 of the whole file: what the program pins. */
  sha256: Uint8Array;
  version: number;
  chainId: bigint;
  /** The anchor hash; checked to equal keccak256(headerRlp) (the header's `hash_slow`). */
  claimedBlockHash: Uint8Array;
  quorum: QuorumVote[];
  manifestJson: Uint8Array;
  manifest: Manifest;
  /** SHA-256 of `manifestJson`, the manifest hash (crypto-spec §8). */
  manifestSha256: Uint8Array;
  invariantsCode: Uint8Array;
  headerRlp: Uint8Array;
  /** The anchor header's `state_root` (RLP item 3). */
  stateRoot: Uint8Array;
  /** The anchor header's `number` (RLP item 8). */
  anchorBlock: bigint;
}

/** A B256 in bincode: u64 length 32 ‖ 32 bytes (alloy `FixedBytes` refuses any other length). */
function b256(r: ByteReader): Uint8Array {
  if (r.len64() !== 32) throw new Error("base witness: a B256 that is not 32 bytes");
  return r.take(32);
}

/** One RLP item at `at`: whether it is a list, and its payload's [start, end). */
function rlpItem(b: Uint8Array, at: number): { list: boolean; start: number; end: number } {
  if (at >= b.length) throw new Error("header rlp: truncated");
  const p = b[at];
  let list = false;
  let start: number;
  let len: number;
  if (p < 0x80) return { list: false, start: at, end: at + 1 };
  if (p < 0xb8 || (p >= 0xc0 && p < 0xf8)) {
    list = p >= 0xc0;
    start = at + 1;
    len = p - (list ? 0xc0 : 0x80);
    if (!list && len === 1 && b[start] < 0x80) throw new Error("header rlp: non-canonical single byte");
  } else {
    list = p >= 0xf8;
    const ll = p - (list ? 0xf7 : 0xb7);
    if (at + 1 + ll > b.length || b[at + 1] === 0) throw new Error("header rlp: bad length");
    len = Number(beToBigInt(b.subarray(at + 1, at + 1 + ll)));
    if (len < 56) throw new Error("header rlp: non-canonical length");
    start = at + 1 + ll;
  }
  if (start + len > b.length) throw new Error("header rlp: truncated");
  return { list, start, end: start + len };
}

/** The anchor header's `state_root` (item 3) and `number` (item 8), as alloy's `Header` decodes them. */
function parseHeader(rlp: Uint8Array): { stateRoot: Uint8Array; number: bigint } {
  const top = rlpItem(rlp, 0);
  if (!top.list || top.end !== rlp.length) throw new Error("header rlp: not one list");
  const items: { list: boolean; start: number; end: number }[] = [];
  for (let at = top.start; at < top.end; ) {
    const it = rlpItem(rlp, at);
    items.push(it);
    at = it.end;
  }
  if (items.length < 15) throw new Error("header rlp: too few fields");
  const root = items[3];
  const num = items[8];
  if (root.list || root.end - root.start !== 32) throw new Error("header rlp: state root");
  const n = rlp.subarray(num.start, num.end);
  if (num.list || n.length > 8 || (n.length > 0 && n[0] === 0)) throw new Error("header rlp: block number");
  return { stateRoot: rlp.subarray(root.start, root.end), number: beToBigInt(n) };
}

function parseManifest(json: Uint8Array): Manifest {
  const m: unknown = JSON.parse(fromUtf8(json));
  const targets = typeof m === "object" && m !== null && "targets" in m ? m.targets : undefined;
  const ok = (t: unknown) => typeof t === "object" && t !== null && "name" in t && typeof t.name === "string" && "address" in t && typeof t.address === "string";
  if (!Array.isArray(targets) || !targets.every(ok)) throw new Error("base manifest: no targets");
  return m as Manifest;
}

/**
 * Parses a base witness envelope (witness.rs `Envelope` with `Payload::Base`). With `expectedSha256`
 * (hex, optional `0x`), refuses bytes that hash otherwise, as cmd.rs `pinned_base_at`. Refuses a header
 * whose keccak256 is not the claimed block hash.
 */
export function parseBase(bytes: Uint8Array, expectedSha256?: string): BaseWitness {
  const digest = sha256(bytes);
  if (expectedSha256 !== undefined && !equalBytes(digest, fromHex(expectedSha256.toLowerCase()))) {
    throw new Error(`the base witness hashes to ${toHex(digest)}, not the pinned ${expectedSha256}`);
  }
  const r = new ByteReader(bytes);
  const version = r.u32le();
  const chainId = r.u64le();
  const claimedBlockHash = b256(r);
  const quorum: QuorumVote[] = [];
  for (let n = r.len64(); n > 0; n--) quorum.push({ source: fromUtf8(r.take(r.len64())), blockHash: b256(r) });
  if (r.u32le() !== PAYLOAD_BASE) throw new Error("not a base witness");
  const manifestJson = r.take(r.len64());
  const invariantsCode = r.take(r.len64());
  const headerRlp = r.take(r.len64());
  if (!equalBytes(keccak_256(headerRlp), claimedBlockHash)) {
    throw new Error("base witness: keccak256(header) is not the claimed block hash");
  }
  const header = parseHeader(headerRlp);
  return {
    sha256: digest,
    version,
    chainId,
    claimedBlockHash,
    quorum,
    manifestJson,
    manifest: parseManifest(manifestJson),
    manifestSha256: sha256(manifestJson),
    invariantsCode,
    headerRlp,
    stateRoot: header.stateRoot,
    anchorBlock: header.number,
  };
}

/** practice.rs `vault_address`: the address of target `vault-<i>`, lowercase `0x` hex. */
export function vaultAddress(manifest: Manifest, i: number): string {
  const t = manifest.targets.find((x) => x.name === `vault-${i}`);
  if (t === undefined) throw new Error(`no vault-${i} in the manifest`);
  return toHex0x(fromHexN(t.address, 20, `vault-${i} address`));
}
