/**
 * Blobs and base witnesses stored with FastFS (docs/crypto-spec.md §4.1), read from NEAR: a mirror of
 * crates/verifier/src/fastfs.rs. An upload lives in the arguments of a `__fastdata_fastfs` function call;
 * `near-tx:<tx hash>:<signer>` names it, and any archival RPC returns that transaction. The reader checks
 * the bytes' SHA-256 against the pinned hash, so the RPC is untrusted.
 */
import { base64 } from "@scure/base";
import { ByteReader, concat, u32le, u8, utf8 } from "./bytes";

/** fastfs.rs `SCHEME`. */
export const NEAR_TX = "near-tx:";

/** fastfs.rs `parse_location`: `near-tx:<base58 tx hash>:<signer account>`, else null. */
export function parseLocation(url: string): { hash: string; signer: string } | null {
  if (!url.startsWith(NEAR_TX)) return null;
  const rest = url.slice(NEAR_TX.length);
  const i = rest.indexOf(":");
  if (i < 0) return null;
  const hash = rest.slice(0, i);
  const signer = rest.slice(i + 1);
  return hash && signer ? { hash, signer } : null;
}

/** fastfs.rs `simple_content`: borsh `FastfsData::Simple { relative_path, content: Some { mime_type, content } }` → the file. */
export function simpleContent(args: Uint8Array): Uint8Array {
  const r = new ByteReader(args);
  try {
    if (r.u8() !== 0) throw new Error("only FastfsData::Simple uploads are supported (<= 1 MiB)");
    r.take(r.u32le());
    if (r.u8() !== 1) throw new Error("FastFS upload has no content");
    r.take(r.u32le());
    return r.take(r.u32le()).slice();
  } catch (e) {
    if (e instanceof RangeError) throw new Error("truncated FastFS data");
    throw e;
  }
}

/** fastfs.rs `simple_args`: the borsh arguments of an upload (the tests build transactions with it). */
export function simpleArgs(relativePath: string, mimeType: string, content: Uint8Array): Uint8Array {
  const p = utf8(relativePath);
  const m = utf8(mimeType);
  return concat(u8(0), u32le(p.length), p, u8(1), u32le(m.length), m, u32le(content.length), content);
}

const field = (v: unknown, k: string): unknown =>
  typeof v === "object" && v !== null && !Array.isArray(v) ? (v as Record<string, unknown>)[k] : undefined;

/**
 * fastfs.rs `content_from_tx_response`, given the `tx` call's result (the RPC client returns `result`):
 * the first `__fastdata_fastfs` call's file.
 */
export function contentFromTx(result: unknown): Uint8Array {
  const actions = field(field(result, "transaction"), "actions");
  if (!Array.isArray(actions)) throw new Error("no transaction in RPC response");
  for (const a of actions) {
    const fc = field(a, "FunctionCall");
    if (field(fc, "method_name") === "__fastdata_fastfs") {
      const args = field(fc, "args");
      if (typeof args !== "string") throw new Error("no args");
      let raw: Uint8Array;
      try {
        raw = base64.decode(args);
      } catch {
        throw new Error("args base64: invalid");
      }
      return simpleContent(raw);
    }
  }
  throw new Error("no __fastdata_fastfs call in the transaction");
}
