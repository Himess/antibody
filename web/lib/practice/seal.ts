/**
 * The sealed blob (`ABSEAL1`) and the commitment, docs/crypto-spec.md §4–§5: a mirror of
 * crates/verifier/src/sealed.rs, crates/cli/src/blob.rs (`practice_content_key`, `seal_with`) and
 * crates/cli/src/cmd.rs (`commitment`). Every byte equals the Rust's (golden/web-vectors.json).
 */
import { x25519 } from "@noble/curves/ed25519.js";
import { chacha20poly1305 } from "@noble/ciphers/chacha.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { ByteReader, concat, equalBytes, fromUtf8, u16be, u32be, utf8 } from "./bytes";

/** §4.1 magic, 8 bytes. */
export const MAGIC = utf8("ABSEAL1\0");
/** §4.2 payload magic, 6 bytes. */
export const PAYLOAD_MAGIC = utf8("ABPOC1");
/** §4.3: epk (32) ‖ nonce_w (12) ‖ AEAD(κ) (48). */
export const WRAP_V_LEN = 92;
const AD_WRAP_V = utf8("antibody/wrap-v/v1");
const AD_SEALED = utf8("antibody/sealed/v1");

/** sealed.rs `SealError`: a malformed blob ("format") or one that does not authenticate ("crypto"). */
export class SealError extends Error {
  readonly kind: "format" | "crypto";

  constructor(kind: "format" | "crypto", detail: string) {
    super(`sealed blob ${kind}: ${detail}`);
    this.name = "SealError";
    this.kind = kind;
  }
}

function sized(b: Uint8Array, n: number, what: string): Uint8Array {
  if (b.length !== n) throw new SealError("format", `${what} must be ${n} bytes`);
  return b;
}

/** ChaCha20-Poly1305 (RFC 8439): ciphertext ‖ 16-byte tag. */
function aeadSeal(key: Uint8Array, nonce: Uint8Array, msg: Uint8Array, ad: Uint8Array): Uint8Array {
  return chacha20poly1305(key, nonce, ad).encrypt(msg);
}

function aeadOpen(key: Uint8Array, nonce: Uint8Array, ct: Uint8Array, ad: Uint8Array): Uint8Array {
  try {
    return chacha20poly1305(key, nonce, ad).decrypt(ct);
  } catch {
    throw new SealError("crypto", "authentication failed");
  }
}

/**
 * sealed.rs `dh`: X25519, refusing a non-contributory shared secret (all zeros, x25519-dalek's
 * `was_contributory`; crypto-spec §2). noble refuses the low-order points that produce one before the
 * ladder; the all-zero check states the Rust rule itself.
 */
function dh(secret: Uint8Array, pub: Uint8Array): Uint8Array {
  let shared: Uint8Array;
  try {
    shared = x25519.getSharedSecret(secret, pub);
  } catch {
    throw new SealError("crypto", "non-contributory X25519 point");
  }
  if (shared.every((b) => b === 0)) throw new SealError("crypto", "non-contributory X25519 point");
  return shared;
}

/** The key-encryption key: HKDF-SHA256(shared, salt = epk ‖ pk_v, info = "antibody/wrap-v/v1" ‖ program_key). */
function kek(shared: Uint8Array, epk: Uint8Array, pkV: Uint8Array, programKey: string): Uint8Array {
  return hkdf(sha256, shared, concat(epk, pkV), concat(AD_WRAP_V, utf8(programKey)), 32);
}

/**
 * §4.3, sealed.rs `wrap_v`: wraps the content key `k` to the program's sealing key `pkV`. `e` and
 * `nonceW` come from the sealer's CSPRNG. Returns `epk ‖ nonce_w ‖ AEAD_kek(nonce_w, k, ad = "antibody/wrap-v/v1")`.
 */
export function wrapV(k: Uint8Array, pkV: Uint8Array, programKey: string, e: Uint8Array, nonceW: Uint8Array): Uint8Array {
  sized(k, 32, "content key");
  sized(pkV, 32, "sealing key");
  sized(e, 32, "e");
  sized(nonceW, 12, "nonce_w");
  const epk = x25519.getPublicKey(e);
  const shared = dh(e, pkV);
  return concat(epk, nonceW, aeadSeal(kek(shared, epk, pkV, programKey), nonceW, k, AD_WRAP_V));
}

/** sealed.rs `unwrap_v`, the program's side of §4.3 (the TEE holds `skV`): the content key. */
export function unwrapV(wrap: Uint8Array, skV: Uint8Array, programKey: string): Uint8Array {
  if (wrap.length !== WRAP_V_LEN) throw new SealError("format", "wrap_v must be 92 bytes");
  sized(skV, 32, "sealing secret");
  const epk = wrap.subarray(0, 32);
  const pkV = x25519.getPublicKey(skV);
  const shared = dh(skV, epk);
  const k = aeadOpen(kek(shared, epk, pkV, programKey), wrap.subarray(32, 44), wrap.subarray(44), AD_WRAP_V);
  return sized(k, 32, "content key");
}

/** §4.2 payload `P`. */
export interface SealedPayload {
  programKey: string;
  /** The committing NEAR account. */
  hunter: string;
  /** The witness envelope (bincode). */
  envelope: Uint8Array;
  /** Free text, never parsed. */
  report: Uint8Array;
}

/** §4.2, `SealedPayload::encode`: "ABPOC1" ‖ u16 ‖ program_key ‖ u16 ‖ hunter ‖ u32 ‖ envelope ‖ u32 ‖ report (big-endian lengths). */
export function encodePayload(p: SealedPayload): Uint8Array {
  const pk = utf8(p.programKey);
  const h = utf8(p.hunter);
  return concat(PAYLOAD_MAGIC, u16be(pk.length), pk, u16be(h.length), h, u32be(p.envelope.length), p.envelope, u32be(p.report.length), p.report);
}

function text(b: Uint8Array, what: string): string {
  try {
    return fromUtf8(b);
  } catch {
    throw new SealError("format", `${what} utf-8`);
  }
}

/** `SealedPayload::decode`: refuses a wrong magic, invalid UTF-8, a truncated payload and trailing bytes. */
export function decodePayload(b: Uint8Array): SealedPayload {
  const r = new ByteReader(b);
  try {
    if (!equalBytes(r.take(6), PAYLOAD_MAGIC)) throw new SealError("format", "payload magic");
    const programKey = text(r.take(r.u16be()), "program key");
    const hunter = text(r.take(r.u16be()), "hunter");
    const envelope = r.take(r.u32be()).slice();
    const report = r.take(r.u32be()).slice();
    if (r.remaining !== 0) throw new SealError("format", "trailing bytes");
    return { programKey, hunter, envelope, report };
  } catch (e) {
    if (e instanceof SealError) throw e;
    throw new SealError("format", "truncated");
  }
}

/** §4.1, sealed.rs `seal`: "ABSEAL1\0" ‖ wrap_v ‖ nonce_b ‖ AEAD_k(nonce_b, P, ad = "antibody/sealed/v1" ‖ program_key). */
export function seal(k: Uint8Array, payload: SealedPayload, wrap: Uint8Array, nonceB: Uint8Array): Uint8Array {
  sized(k, 32, "content key");
  sized(wrap, WRAP_V_LEN, "wrap_v");
  sized(nonceB, 12, "nonce_b");
  const ad = concat(AD_SEALED, utf8(payload.programKey));
  return concat(MAGIC, wrap, nonceB, aeadSeal(k, nonceB, encodePayload(payload), ad));
}

/** sealed.rs `split`: a blob's (wrap_v, nonce_b, ciphertext). */
export function splitBlob(blob: Uint8Array): { wrap: Uint8Array; nonce: Uint8Array; ciphertext: Uint8Array } {
  if (blob.length < 8 + WRAP_V_LEN + 12 + 16 || !equalBytes(blob.subarray(0, 8), MAGIC)) {
    throw new SealError("format", "not an ABSEAL1 blob");
  }
  return {
    wrap: blob.subarray(8, 8 + WRAP_V_LEN),
    nonce: blob.subarray(8 + WRAP_V_LEN, 8 + WRAP_V_LEN + 12),
    ciphertext: blob.subarray(8 + WRAP_V_LEN + 12),
  };
}

/** sealed.rs `open_with_key`: decrypts with a known content key (anyone, for a practice try). */
export function openWithKey(blob: Uint8Array, k: Uint8Array, programKey: string): SealedPayload {
  const { nonce, ciphertext } = splitBlob(blob);
  sized(k, 32, "content key");
  return decodePayload(aeadOpen(k, nonce, ciphertext, concat(AD_SEALED, utf8(programKey))));
}

/**
 * blob.rs `practice_content_key`: a practice try's content key is public by construction,
 * `SHA-256("antibody/practice/public-content-key/v1|<program key>|<hunter>")` (docs/zk.md).
 */
export function practiceContentKey(programKey: string, hunter: string): Uint8Array {
  return sha256(utf8(`antibody/practice/public-content-key/v1|${programKey}|${hunter}`));
}

/**
 * §5, cmd.rs `commitment`: `SHA-256("antibody/commit/v2" ‖ u32(len) ‖ program_key ‖ u32(round) ‖
 * u32(len) ‖ hunter ‖ SHA-256(blob))`, big-endian integers.
 */
export function commitment(programKey: string, round: number, hunter: string, blobSha256: Uint8Array): Uint8Array {
  sized(blobSha256, 32, "blob sha256");
  const pk = utf8(programKey);
  const h = utf8(hunter);
  return sha256(concat(utf8("antibody/commit/v2"), u32be(pk.length), pk, u32be(round), u32be(h.length), h, blobSha256));
}

/** A source of random bytes: `n` bytes per call. */
export type Rng = (n: number) => Uint8Array;

/** The platform CSPRNG (`crypto.getRandomValues`), what the CLI's `getrandom` is. */
export const cryptoRng: Rng = (n) => crypto.getRandomValues(new Uint8Array(n));

function draw(rng: Rng, n: number): Uint8Array {
  const b = rng(n);
  if (b.length !== n) throw new Error(`rng returned ${b.length} bytes, not ${n}`);
  return b;
}

/** blob.rs `Sealed`. */
export interface Sealed {
  blob: Uint8Array;
  sha256: Uint8Array;
  wrap: Uint8Array;
  contentKey: Uint8Array;
}

/**
 * A practice try's seal, blob.rs `seal_with` under the public content key (`practice_content_key`), as
 * cmd.rs `commit` seals for the practice program. `rng` gives `e` (32 bytes), then `nonce_w` (12), then
 * `nonce_b` (12), in the CLI's order; tests pass fixed bytes.
 */
export function sealPractice(o: {
  sealingPk: Uint8Array;
  programKey: string;
  hunter: string;
  envelope: Uint8Array;
  report?: Uint8Array;
  rng?: Rng;
}): Sealed {
  const rng = o.rng ?? cryptoRng;
  const k = practiceContentKey(o.programKey, o.hunter);
  const wrap = wrapV(k, o.sealingPk, o.programKey, draw(rng, 32), draw(rng, 12));
  const payload: SealedPayload = { programKey: o.programKey, hunter: o.hunter, envelope: o.envelope, report: o.report ?? new Uint8Array(0) };
  const blob = seal(k, payload, wrap, draw(rng, 12));
  return { blob, sha256: sha256(blob), wrap, contentKey: k };
}
