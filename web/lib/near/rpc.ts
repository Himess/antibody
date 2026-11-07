/**
 * A small NEAR JSON-RPC client for the browser, after crates/cli/src/near.rs: an ordered list of
 * endpoints tried in turn (a network error, a timeout, HTTP 408, 429 or 5xx, an answer that is not JSON,
 * or a transient RPC error moves to the next, with exponential backoff), typed helpers for the calls a
 * practice try makes, and near.rs's outcome helpers (`failures`, `return_value`, `events`). Resending a
 * signed transaction is safe: its nonce lets it execute once.
 *
 * Endpoint URLs may carry keys: none ever appears in an error or a log (an endpoint is named by its
 * position, "rpc #2").
 */
import { base64 } from "@scure/base";

export type Finality = "optimistic" | "near-final" | "final";
export type WaitUntil = "NONE" | "INCLUDED" | "EXECUTED_OPTIMISTIC" | "INCLUDED_FINAL" | "EXECUTED" | "FINAL";

/** An RPC call that failed: the node's JSON-RPC `error` when it answered one (near.rs `{method}: {e}`). */
export class RpcError extends Error {
  readonly method: string;
  /** The JSON-RPC error object (`name`, `cause`, `code`, `message`, `data`), if the node sent one. */
  readonly error: unknown;

  constructor(method: string, message: string, error?: unknown) {
    super(message);
    this.name = "RpcError";
    this.method = method;
    this.error = error;
  }

  /** `error.cause.name`: UNKNOWN_ACCOUNT, TIMEOUT_ERROR, INVALID_TRANSACTION, … */
  get causeName(): string | undefined {
    const n = get(get(this.error, "cause"), "name");
    return typeof n === "string" ? n : undefined;
  }
}

/** near.rs `call`: RPC errors worth another attempt (FastNEAR answers a rate limit as -429). */
const TRANSIENT = ["TIMEOUT_ERROR", "Too Many Requests", "-429", "Rate limit", "UNKNOWN_BLOCK"];

export interface NearRpcOptions {
  /** Per-request timeout (default 30 s). */
  timeoutMs?: number;
  /** Attempts per call across the endpoints (default 6). */
  attempts?: number;
  /** The first retry's pause, doubled each retry (default 250 ms) up to `maxBackoffMs` (default 8 s). */
  backoffMs?: number;
  maxBackoffMs?: number;
  /** For tests: the fetch and sleep to use. */
  fetch?: (url: string, init: RequestInit) => Promise<Response>;
  sleep?: (ms: number) => Promise<void>;
}

type Attempt = { ok: true; result: unknown } | { ok: false; retry: boolean; error: RpcError };

/** `call_function`'s result: the raw return bytes, with helpers. */
export interface ViewResult {
  bytes: Uint8Array;
  logs: string[];
  blockHeight: number;
  blockHash: string;
  /** The return value as UTF-8 JSON (near.rs `view`). */
  json<T = unknown>(): T;
}

export interface AccessKeyView {
  /** u64; refused if the RPC's JSON number is past 2^53 (it would have lost precision). */
  nonce: bigint;
  /** The block the view read: a recent hash a transaction may name. */
  blockHash: string;
  blockHeight: number;
  permission: unknown;
}

export interface AccountView {
  amount: string;
  locked: string;
  code_hash: string;
  storage_usage: number;
  block_height: number;
  block_hash: string;
}

export interface BlockView {
  header: { height: number; hash: string; prev_hash: string; timestamp_nanosec: string };
  [field: string]: unknown;
}

/** An execution status: `{ SuccessValue }`, `{ SuccessReceiptId }`, `{ Failure }`, or a word ("NotStarted"). */
export type ExecutionStatus = { SuccessValue?: string; SuccessReceiptId?: string; Failure?: unknown } | string;

export interface ExecutionOutcomeView {
  id: string;
  outcome: { logs: string[]; status: ExecutionStatus; receipt_ids?: string[]; gas_burnt?: number; executor_id?: string };
}

/** A `send_tx` / `tx` result; with `wait_until` NONE or INCLUDED only `final_execution_status` is set. */
export interface TxOutcome {
  final_execution_status?: string;
  status?: ExecutionStatus;
  transaction?: { hash: string; signer_id: string; receiver_id: string; nonce: number; [field: string]: unknown };
  transaction_outcome?: ExecutionOutcomeView;
  receipts_outcome?: ExecutionOutcomeView[];
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** serde_json's `v.get(k)`: the field of an object, else undefined (a present null stays null). */
function get(v: unknown, k: string): unknown {
  return isObject(v) && Object.prototype.hasOwnProperty.call(v, k) ? v[k] : undefined;
}

function list(v: unknown): unknown[] {
  return Array.isArray(v) ? v : [];
}

const utf8 = new TextEncoder();
const strictUtf8 = new TextDecoder("utf-8", { fatal: true });

export class NearRpc {
  private readonly endpoints: string[];
  private readonly timeoutMs: number;
  private readonly attempts: number;
  private readonly backoffMs: number;
  private readonly maxBackoffMs: number;
  private readonly doFetch: (url: string, init: RequestInit) => Promise<Response>;
  private readonly sleep: (ms: number) => Promise<void>;

  /** `endpoints`: RPC URLs in order of preference (the first is tried first). */
  constructor(endpoints: string[], opts: NearRpcOptions = {}) {
    if (endpoints.length === 0) throw new Error("NearRpc: no endpoint");
    this.endpoints = [...endpoints];
    this.timeoutMs = opts.timeoutMs ?? 30_000;
    this.attempts = Math.max(1, opts.attempts ?? 6);
    this.backoffMs = opts.backoffMs ?? 250;
    this.maxBackoffMs = opts.maxBackoffMs ?? 8_000;
    // Called as a plain function: a browser's fetch refuses any `this` but the global object.
    const f = opts.fetch ?? ((url: string, init: RequestInit) => fetch(url, init));
    this.doFetch = (url, init) => f(url, init);
    this.sleep = opts.sleep ?? ((ms) => new Promise((resolve) => setTimeout(resolve, ms)));
  }

  /** One JSON-RPC call; failed attempts move to the next endpoint, pausing 250 ms, 500 ms, 1 s, … */
  async call<T = unknown>(method: string, params: unknown): Promise<T> {
    const body = JSON.stringify({ jsonrpc: "2.0", id: "antibody", method, params });
    let last = new RpcError(method, `${method}: no attempt`);
    for (let attempt = 0; attempt < this.attempts; attempt++) {
      if (attempt > 0) await this.sleep(Math.min(this.maxBackoffMs, this.backoffMs * 2 ** (attempt - 1)));
      const r = await this.once(method, body, attempt % this.endpoints.length);
      if (r.ok) return r.result as T;
      if (!r.retry) throw r.error;
      last = r.error;
    }
    throw last;
  }

  private async once(method: string, body: string, i: number): Promise<Attempt> {
    const at = `rpc #${i + 1}`;
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), this.timeoutMs);
    let status: number;
    let text: string;
    try {
      const res = await this.doFetch(this.endpoints[i], {
        method: "POST",
        headers: { "content-type": "application/json" },
        body,
        signal: ctl.signal,
      });
      status = res.status;
      text = await res.text();
    } catch {
      // The error's own message is dropped: it may quote the URL.
      const why = ctl.signal.aborted ? `timed out after ${this.timeoutMs} ms` : "network error";
      return { ok: false, retry: true, error: new RpcError(method, `${method}: ${why} at ${at}`) };
    } finally {
      clearTimeout(timer);
    }
    const busy = status === 408 || status === 429 || status >= 500;
    let json: unknown;
    try {
      json = JSON.parse(text);
    } catch {
      json = undefined;
    }
    const err = get(json, "error");
    if (err !== undefined && err !== null) {
      const s = JSON.stringify(err);
      const retry = busy || TRANSIENT.some((t) => s.includes(t));
      return { ok: false, retry, error: new RpcError(method, `${method}: ${s}`, err) };
    }
    if (status < 200 || status >= 300 || !isObject(json) || !("result" in json)) {
      // An answer that is not JSON at all (a proxy's HTML page, say) is the endpoint failing, as a 5xx is.
      return { ok: false, retry: busy || !isObject(json), error: new RpcError(method, `${method}: HTTP ${status} at ${at}`) };
    }
    return { ok: true, result: json.result };
  }

  /**
   * `query` / `call_function` (near.rs `view_raw`): `args` as JSON (base64 of its UTF-8) or raw bytes
   * (Borsh; `new Uint8Array(0)` for none, as near.rs reads `get_config`). Reads optimistic state by
   * default, so a view sees the transaction just executed.
   */
  async viewFunction(contract: string, method: string, args: object | Uint8Array = {}, finality: Finality = "optimistic"): Promise<ViewResult> {
    const raw = args instanceof Uint8Array ? args : utf8.encode(JSON.stringify(args));
    const r = await this.call("query", {
      request_type: "call_function",
      finality,
      account_id: contract,
      method_name: method,
      args_base64: base64.encode(raw),
    });
    // Older nodes answer a failed view inside the result.
    const inner = get(r, "error");
    if (inner !== undefined) throw new RpcError("query", `view ${contract}.${method}: ${JSON.stringify(inner)}`, inner);
    const result = get(r, "result");
    if (!Array.isArray(result)) throw new RpcError("query", `view ${contract}.${method}: no result`);
    // near.rs: `b.as_u64().unwrap_or(0) as u8`.
    const bytes = Uint8Array.from(list(result), (b) => (typeof b === "number" ? b & 0xff : 0));
    const logs = list(get(r, "logs")).filter((l): l is string => typeof l === "string");
    const height = get(r, "block_height");
    const hash = get(r, "block_hash");
    return {
      bytes,
      logs,
      blockHeight: typeof height === "number" ? height : 0,
      blockHash: typeof hash === "string" ? hash : "",
      json<T = unknown>(): T {
        return JSON.parse(strictUtf8.decode(bytes)) as T;
      },
    };
  }

  /** `query` / `view_access_key`: the key's nonce and the block the view read. */
  async viewAccessKey(account: string, publicKey: string, finality: Finality = "optimistic"): Promise<AccessKeyView> {
    const r = await this.call("query", { request_type: "view_access_key", finality, account_id: account, public_key: publicKey });
    const inner = get(r, "error");
    if (inner !== undefined) throw new RpcError("query", `access key ${publicKey} of ${account}: ${JSON.stringify(inner)}`, inner);
    const nonce = get(r, "nonce");
    const hash = get(r, "block_hash");
    const height = get(r, "block_height");
    if (typeof nonce !== "number" || !Number.isSafeInteger(nonce) || nonce < 0 || typeof hash !== "string") {
      throw new RpcError("query", `access key ${publicKey} of ${account}: unexpected answer`);
    }
    return { nonce: BigInt(nonce), blockHash: hash, blockHeight: typeof height === "number" ? height : 0, permission: get(r, "permission") };
  }

  /** `query` / `view_account` (near.rs `account`): null when the account does not exist. */
  async viewAccount(account: string, finality: Finality = "optimistic"): Promise<AccountView | null> {
    try {
      const r = await this.call("query", { request_type: "view_account", finality, account_id: account });
      if (!isObject(r)) throw new RpcError("query", `account ${account}: unexpected answer`);
      return r as unknown as AccountView;
    } catch (e) {
      if (e instanceof RpcError && (e.causeName === "UNKNOWN_ACCOUNT" || e.message.includes("UNKNOWN_ACCOUNT") || e.message.includes("does not exist"))) {
        return null;
      }
      throw e;
    }
  }

  /** `block`: the latest block at `finality` (a transaction names a final block's hash, as near.rs `send`). */
  async block(finality: Finality = "final"): Promise<BlockView> {
    const r = await this.call("block", { finality });
    const header = get(r, "header");
    if (typeof get(header, "hash") !== "string" || typeof get(header, "height") !== "number") {
      throw new RpcError("block", "block: unexpected answer");
    }
    return r as BlockView;
  }

  /** `send_tx`: sends a signed transaction (base64) and waits for `waitUntil`. */
  async sendTx(signedBase64: string, waitUntil: WaitUntil = "EXECUTED_OPTIMISTIC"): Promise<TxOutcome> {
    return outcome("send_tx", await this.call("send_tx", { signed_tx_base64: signedBase64, wait_until: waitUntil }));
  }

  /** `tx`: a transaction's status by hash and signer (near.rs `tx_status`). */
  async txStatus(hash: string, sender: string, waitUntil: WaitUntil = "NONE"): Promise<TxOutcome> {
    return outcome("tx", await this.call("tx", { tx_hash: hash, sender_account_id: sender, wait_until: waitUntil }));
  }
}

function outcome(method: string, r: unknown): TxOutcome {
  if (!isObject(r)) throw new RpcError(method, `${method}: unexpected answer`);
  return r as TxOutcome;
}

/** near.rs `tx_hash`: the outcome's transaction hash, or "". */
export function txHash(o: unknown): string {
  const h = get(get(o, "transaction"), "hash");
  return typeof h === "string" ? h : "";
}

/** near.rs `failures`: every failure in the transaction tree (its status, then each receipt's), as JSON. */
export function outcomeFailures(o: unknown): string[] {
  const out: string[] = [];
  const f = get(get(o, "status"), "Failure");
  if (f !== undefined) out.push(JSON.stringify(f));
  for (const r of list(get(o, "receipts_outcome"))) {
    const rf = get(get(get(r, "outcome"), "status"), "Failure");
    if (rf !== undefined) out.push(JSON.stringify(rf));
  }
  return out;
}

/**
 * near.rs `return_value`: the transaction's `SuccessValue` decoded as JSON; null for an empty return,
 * undefined when there is none or it is not JSON.
 */
export function returnValue(o: unknown): unknown {
  const b64 = get(get(o, "status"), "SuccessValue");
  if (typeof b64 !== "string") return undefined;
  try {
    const bytes = base64.decode(b64);
    return bytes.length === 0 ? null : (JSON.parse(strictUtf8.decode(bytes)) as unknown);
  } catch {
    return undefined;
  }
}

/** near.rs `events`: the NEP-297 events (`EVENT_JSON:` logs) of every receipt; logs that are not JSON are skipped. */
export function events(o: unknown): unknown[] {
  const out: unknown[] = [];
  for (const r of list(get(o, "receipts_outcome"))) {
    for (const l of list(get(get(r, "outcome"), "logs"))) {
      if (typeof l !== "string" || !l.startsWith("EVENT_JSON:")) continue;
      try {
        out.push(JSON.parse(l.slice("EVENT_JSON:".length)) as unknown);
      } catch {
        // not JSON: skipped, as near.rs
      }
    }
  }
  return out;
}
