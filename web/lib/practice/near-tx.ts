/**
 * NEAR transactions as the CLI signs them (crates/cli/src/near.rs): borsh(Transaction V0) with
 * FunctionCall actions, the SignedTransaction (the transaction, key type 0 = ED25519, the signature
 * over its SHA-256), and the practice vending contract's Borsh arguments and views
 * (crates/cli/src/practice.rs). Borsh integers are little-endian, strings a u32 length ‖ UTF-8.
 */
import { ed25519 } from "@noble/curves/ed25519.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { base58, base64 } from "@scure/base";
import { ByteReader, concat, need, u128le, u32le, u64le, u8, utf8 } from "./bytes";

/** near.rs `TGAS`. */
export const TGAS = BigInt(1_000_000_000_000);
export const ONE_YOCTO = BigInt(1);
/** practice.rs: what a practice account's calls attach, in Tgas. */
export const VEND_TGAS = BigInt(60);
export const CLOSE_TGAS = BigInt(15);
export const RECLAIM_TGAS = BigInt(20);
export const ALLOCATE_TGAS = BigInt(40);
export const PAY_TGAS = BigInt(30);
/** practice.rs `PUBLIC_VEND_SEED`: deliberately public, so a first try needs no NEAR account. */
export const PUBLIC_VEND_SEED = "antibody/practice/public-vend-key/v1";

/** near.rs `Action::FunctionCall` (the only action the browser sends). */
export interface FunctionCall {
  method: string;
  args: Uint8Array;
  /** Gas units (Tgas × `TGAS`). */
  gas: bigint;
  /** yoctoNEAR. */
  deposit: bigint;
}

/** near.rs `b_str` / practice.rs `borsh_str`: u32 length ‖ UTF-8 (`reclaim`'s and `vault_of`'s argument). */
export function borshString(s: string): Uint8Array {
  const b = utf8(s);
  return concat(u32le(b.length), b);
}

/** near.rs `b_pk`: key type 0 (ED25519) ‖ the 32-byte key. */
function borshPublicKey(pk: Uint8Array): Uint8Array {
  return concat(u8(0), need(pk, 32, "public key"));
}

/** `Action::borsh` for a FunctionCall: variant 2 ‖ method ‖ u32 length ‖ args ‖ u64 gas ‖ u128 deposit. */
function encodeAction(a: FunctionCall): Uint8Array {
  return concat(u8(2), borshString(a.method), u32le(a.args.length), a.args, u64le(a.gas), u128le(a.deposit));
}

/**
 * near.rs `encode_tx`, borsh(Transaction V0): signer ‖ public key ‖ u64 nonce ‖ receiver ‖ block hash
 * (32 bytes, or its base58 as the RPC prints it) ‖ u32 count ‖ actions.
 */
export function encodeTx(
  signerId: string,
  publicKey: Uint8Array,
  nonce: bigint,
  receiverId: string,
  blockHash: Uint8Array | string,
  actions: FunctionCall[],
): Uint8Array {
  const hash = need(typeof blockHash === "string" ? base58.decode(blockHash) : blockHash, 32, "block hash");
  return concat(
    borshString(signerId),
    borshPublicKey(publicKey),
    u64le(nonce),
    borshString(receiverId),
    hash,
    u32le(actions.length),
    ...actions.map(encodeAction),
  );
}

export interface SignedTx {
  /** borsh(SignedTransaction). */
  signed: Uint8Array;
  /** `signed` in standard base64: `send_tx`'s `signed_tx_base64`. */
  signedBase64: string;
  /** The transaction hash: base58 of SHA-256(tx). */
  hash: string;
}

/** near.rs `sign_tx`: tx ‖ 0x00 (ED25519) ‖ ed25519(seed, SHA-256(tx)). */
export function signTx(tx: Uint8Array, secretSeed: Uint8Array): SignedTx {
  const digest = sha256(tx);
  const signed = concat(tx, u8(0), ed25519.sign(digest, need(secretSeed, 32, "secret seed")));
  return { signed, signedBase64: base64.encode(signed), hash: base58.encode(digest) };
}

export interface KeyPair {
  /** The 32-byte ed25519 seed (ed25519-dalek's `SigningKey::from_bytes`). */
  seed: Uint8Array;
  publicKey: Uint8Array;
}

/** near.rs `Signer::from_seed`: the key pair of a 32-byte seed. */
export function keyFromSeed(seed: Uint8Array): KeyPair {
  return { seed, publicKey: ed25519.getPublicKey(need(seed, 32, "secret seed")) };
}

/** near.rs `public_key_str`: `ed25519:<base58>`. */
export function publicKeyString(pk: Uint8Array): string {
  return "ed25519:" + base58.encode(need(pk, 32, "public key"));
}

/** practice.rs `public_vend_signer`'s seed: SHA-256("antibody/practice/public-vend-key/v1"). */
export function publicVendSeed(): Uint8Array {
  return sha256(utf8(PUBLIC_VEND_SEED));
}

/**
 * practice.rs `vend_args`: the vending contract's `vend(name, public_key)` in Borsh. near-sdk's
 * `PublicKey` is a `Vec<u8>`: its length (33), the curve (0, ed25519) and the key.
 */
export function vendArgs(name: string, publicKey: Uint8Array): Uint8Array {
  return concat(borshString(name), u32le(33), u8(0), need(publicKey, 32, "public key"));
}

/** practice.rs `check_name`: a practice name is 1 to 32 of a-z, 0-9, - and _. */
export function checkName(name: string): void {
  if (!/^[a-z0-9_-]{1,32}$/.test(name)) throw new Error("a practice name is 1 to 32 of a-z, 0-9, - and _");
}

/** The vending contract's `get_config` (Borsh): owner, antibody and token account ids, then the program id. */
export interface VendingConfig {
  owner: string;
  antibody: string;
  token: string;
  programId: number;
}

const lenient = new TextDecoder();

/** practice.rs `vending_program`: reads three Borsh strings, then the u32 program id; later fields are ignored. */
export function parseVendingConfig(b: Uint8Array): VendingConfig {
  const r = new ByteReader(b);
  try {
    // The CLI only skips the strings, so they are decoded leniently: never an error it would not raise.
    const [owner, antibody, token] = [0, 1, 2].map(() => lenient.decode(r.take(r.u32le())));
    return { owner, antibody, token, programId: r.u32le() };
  } catch {
    throw new Error("get_config: truncated");
  }
}

/** practice.rs `vault_of`: Borsh `Option<u32>`, the vault the vending contract assigned (null: none). */
export function parseVaultOf(b: Uint8Array): number | null {
  if (b.length >= 5 && b[0] === 1) return (b[1] | (b[2] << 8) | (b[3] << 16) | (b[4] << 24)) >>> 0;
  return null;
}

function byUtf8(a: string, b: string): number {
  const x = utf8(a);
  const y = utf8(b);
  for (let i = 0; i < Math.min(x.length, y.length); i++) if (x[i] !== y[i]) return x[i] - y[i];
  return x.length - y.length;
}

function sortKeys(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(sortKeys);
  if (typeof v === "object" && v !== null) {
    // No prototype, so a "__proto__" key stays an ordinary key.
    const out: Record<string, unknown> = Object.create(null);
    for (const k of Object.keys(v).sort(byUtf8)) out[k] = sortKeys((v as Record<string, unknown>)[k]);
    return out;
  }
  return v;
}

/**
 * JSON arguments as near.rs `function_call` sends them (`serde_json::Value::to_string`): compact, keys
 * sorted by their UTF-8 bytes. Plain JSON values only; u64 and u128 amounts go as strings, as NEAR
 * contracts take them.
 */
export function jsonArgs(v: unknown): Uint8Array {
  return utf8(JSON.stringify(sortKeys(v)));
}
