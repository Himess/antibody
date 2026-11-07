/**
 * Byte helpers for the browser try: hex and UTF-8, concatenation, the fixed-width integers the Rust
 * writes with `to_be_bytes` / `to_le_bytes`, and a bounds-checked reader. Strings are UTF-8 and every
 * length is a byte length, as in Rust (docs/crypto-spec.md §10).
 */

const HEX = "0123456789abcdef";

/** Lowercase hex without `0x` (docs/crypto-spec.md §10: hashes and keys). */
export function toHex(b: Uint8Array): string {
  let s = "";
  for (const x of b) s += HEX[x >> 4] + HEX[x & 15];
  return s;
}

/** `0x` + lowercase hex: EVM values, as Rust's `format!("{:#x}", …)` prints a B256 or an Address. */
export function toHex0x(b: Uint8Array): string {
  return "0x" + toHex(b);
}

function nibble(c: number): number {
  if (c >= 48 && c <= 57) return c - 48; // 0-9
  if (c >= 97 && c <= 102) return c - 87; // a-f
  if (c >= 65 && c <= 70) return c - 55; // A-F
  throw new Error("hex: invalid digit");
}

/** Hex (either case, optional `0x`) to bytes. Throws on an odd length or a non-hex digit. */
export function fromHex(hex: string): Uint8Array {
  const h = hex.startsWith("0x") ? hex.slice(2) : hex;
  if (h.length % 2 !== 0) throw new Error("hex: odd length");
  const out = new Uint8Array(h.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = (nibble(h.charCodeAt(2 * i)) << 4) | nibble(h.charCodeAt(2 * i + 1));
  return out;
}

/** Hex of exactly `n` bytes (a B256, an Address): alloy's `FixedBytes::from_hex`. */
export function fromHexN(hex: string, n: number, what: string): Uint8Array {
  const b = fromHex(hex);
  if (b.length !== n) throw new Error(`${what}: expected ${n} bytes, got ${b.length}`);
  return b;
}

const encoder = new TextEncoder();
// `fatal` refuses invalid UTF-8 and `ignoreBOM` keeps a leading U+FEFF, as Rust's `String::from_utf8`.
const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

/** UTF-8 bytes of `s`. */
export function utf8(s: string): Uint8Array {
  return encoder.encode(s);
}

/** UTF-8 to string; throws on invalid UTF-8 (`String::from_utf8`). */
export function fromUtf8(b: Uint8Array): string {
  return decoder.decode(b);
}

/** `a ‖ b ‖ …`. */
export function concat(...parts: Uint8Array[]): Uint8Array {
  let n = 0;
  for (const p of parts) n += p.length;
  const out = new Uint8Array(n);
  let i = 0;
  for (const p of parts) {
    out.set(p, i);
    i += p.length;
  }
  return out;
}

export function equalBytes(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

/** Bytes of exactly length `n`, or an error naming `what`. */
export function need(b: Uint8Array, n: number, what: string): Uint8Array {
  if (b.length !== n) throw new Error(`${what} must be ${n} bytes`);
  return b;
}

// Rust's integer types make an out-of-range value unrepresentable; here it is an error, never a silent
// truncation.
function small(n: number, bits: number): number {
  if (!Number.isInteger(n) || n < 0 || n >= 2 ** bits) throw new RangeError(`not a u${bits}: ${n}`);
  return n;
}

function big(n: bigint, bits: number): bigint {
  if (n < BigInt(0) || n >= BigInt(1) << BigInt(bits)) throw new RangeError(`not a u${bits}: ${n}`);
  return n;
}

/** Unsigned little-endian of `bytes` bytes. */
function uintLE(n: bigint, bytes: number): Uint8Array {
  big(n, bytes * 8);
  const out = new Uint8Array(bytes);
  for (let i = 0; i < bytes; i++, n >>= BigInt(8)) out[i] = Number(n & BigInt(0xff));
  return out;
}

export function u8(n: number): Uint8Array {
  return Uint8Array.of(small(n, 8));
}

export function u16be(n: number): Uint8Array {
  small(n, 16);
  return Uint8Array.of(n >>> 8, n & 0xff);
}

export function u32be(n: number): Uint8Array {
  small(n, 32);
  return Uint8Array.of(n >>> 24, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff);
}

export function u32le(n: number): Uint8Array {
  return u32be(n).reverse();
}

export function u64le(n: bigint): Uint8Array {
  return uintLE(n, 8);
}

export function u128le(n: bigint): Uint8Array {
  return uintLE(n, 16);
}

/** A U256 as ruint's `to_be_bytes`: 32 bytes, big-endian. */
export function u256be(n: bigint): Uint8Array {
  return uintLE(n, 32).reverse();
}

/** Big-endian bytes to an unsigned integer. */
export function beToBigInt(b: Uint8Array): bigint {
  let n = BigInt(0);
  for (const x of b) n = (n << BigInt(8)) | BigInt(x);
  return n;
}

/** A bounds-checked cursor over bytes (sealed.rs `Reader`); a read past the end throws "truncated". */
export class ByteReader {
  private i = 0;
  private readonly b: Uint8Array;

  constructor(b: Uint8Array) {
    this.b = b;
  }

  get offset(): number {
    return this.i;
  }

  get remaining(): number {
    return this.b.length - this.i;
  }

  /** The next `n` bytes (a view, not a copy). */
  take(n: number): Uint8Array {
    if (!Number.isSafeInteger(n) || n < 0 || n > this.remaining) throw new RangeError("truncated");
    const s = this.b.subarray(this.i, this.i + n);
    this.i += n;
    return s;
  }

  u8(): number {
    return this.take(1)[0];
  }

  u16be(): number {
    const s = this.take(2);
    return (s[0] << 8) | s[1];
  }

  u32be(): number {
    const s = this.take(4);
    return ((s[0] << 24) | (s[1] << 16) | (s[2] << 8) | s[3]) >>> 0;
  }

  u32le(): number {
    const s = this.take(4);
    return ((s[3] << 24) | (s[2] << 16) | (s[1] << 8) | s[0]) >>> 0;
  }

  u64le(): bigint {
    return beToBigInt(this.take(8).slice().reverse());
  }

  /** A bincode length (u64): a count that must also fit a JS array or buffer. */
  len64(): number {
    const n = this.u64le();
    if (n > BigInt(Number.MAX_SAFE_INTEGER)) throw new RangeError("truncated");
    return Number(n);
  }
}
