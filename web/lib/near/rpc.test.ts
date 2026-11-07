// The NEAR RPC client against a mocked fetch: request shapes, failover and backoff, timeouts, typed
// helpers, and near.rs's outcome helpers.
import { base64 } from "@scure/base";
import { describe, expect, it } from "vitest";
import { events, NearRpc, outcomeFailures, returnValue, RpcError, txHash } from "./rpc";

interface Sent {
  url: string;
  body: { jsonrpc: string; id: string; method: string; params: Record<string, unknown> };
}

type Reply = { status?: number; json?: unknown; text?: string } | "network" | "hang";

/** A fetch that answers each request with the next reply and records what was sent. */
function mock(replies: Reply[]) {
  const sent: Sent[] = [];
  const sleeps: number[] = [];
  const fetch = (url: string, init: RequestInit): Promise<Response> => {
    sent.push({ url, body: JSON.parse(String(init.body)) as Sent["body"] });
    const r = replies.shift();
    if (r === undefined) throw new Error("no reply left");
    if (r === "network") return Promise.reject(new TypeError(`fetch failed for ${url}`));
    if (r === "hang") {
      return new Promise((_, reject) => init.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError"))));
    }
    const text = r.text ?? JSON.stringify(r.json);
    return Promise.resolve(new Response(text, { status: r.status ?? 200, headers: { "content-type": "application/json" } }));
  };
  const sleep = async (ms: number) => {
    sleeps.push(ms);
  };
  return { sent, sleeps, fetch, sleep };
}

const ok = (result: unknown): Reply => ({ json: { jsonrpc: "2.0", id: "antibody", result } });
const rpcError = (name: string, cause: string, status = 200): Reply => ({
  status,
  json: { jsonrpc: "2.0", id: "antibody", error: { name, cause: { name: cause, info: {} }, code: -32000, message: "Server error" } },
});

const A = "https://rpc-a.example/?apiKey=SECRET-A";
const B = "https://rpc-b.example/SECRET-B";
const C = "https://rpc-c.example";

describe("NearRpc", () => {
  it("calls view functions with JSON or Borsh arguments", async () => {
    const m = mock([ok({ result: [...new TextEncoder().encode('{"a":1}')], logs: ["l"], block_height: 7, block_hash: "h" }), ok({ result: [1, 4, 0, 0, 0], logs: [] })]);
    const rpc = new NearRpc([A], m);
    const v = await rpc.viewFunction("bounty.antibody.testnet", "get_program", { program_id: 6 });
    expect(v.json()).toEqual({ a: 1 });
    expect([v.logs, v.blockHeight, v.blockHash]).toEqual([["l"], 7, "h"]);
    expect(m.sent[0].body.method).toBe("query");
    expect(m.sent[0].body.params).toEqual({
      request_type: "call_function",
      finality: "optimistic",
      account_id: "bounty.antibody.testnet",
      method_name: "get_program",
      args_base64: base64.encode(new TextEncoder().encode('{"program_id":6}')),
    });
    const raw = await rpc.viewFunction("practice.antibody.testnet", "vault_of", Uint8Array.of(3, 0, 0, 0, 97, 110, 110), "final");
    expect(raw.bytes).toEqual(Uint8Array.of(1, 4, 0, 0, 0));
    expect(m.sent[1].body.params.args_base64).toBe("AwAAAGFubg==");
    expect(m.sent[1].body.params.finality).toBe("final");
  });

  it("fails over on network errors, HTTP 429 and 5xx, backing off exponentially", async () => {
    const m = mock(["network", { status: 429, text: "Too Many Requests" }, { status: 503, text: "unavailable" }, ok({ header: { hash: "H", height: 9 } })]);
    const rpc = new NearRpc([A, B, C], m);
    const b = await rpc.block();
    expect(b.header.hash).toBe("H");
    expect(m.sent.map((s) => s.url)).toEqual([A, B, C, A]);
    expect(m.sent[0].body).toEqual({ jsonrpc: "2.0", id: "antibody", method: "block", params: { finality: "final" } });
    expect(m.sleeps).toEqual([250, 500, 1000]);
    // An answer that is not JSON-RPC (a proxy's page) moves on too; a 4xx JSON-RPC error does not.
    const h = mock([{ text: "<html>gateway</html>" }, ok({ header: { hash: "H", height: 9 } })]);
    await new NearRpc([A, B], h).block();
    expect(h.sent.map((s) => s.url)).toEqual([A, B]);
    const v = mock([rpcError("REQUEST_VALIDATION_ERROR", "PARSE_ERROR", 400)]);
    await expect(new NearRpc([A, B], v).block()).rejects.toThrow(/PARSE_ERROR/);
    expect(v.sent).toHaveLength(1);
  });

  it("retries transient RPC errors (near.rs) and stops at the first other one", async () => {
    const m = mock([rpcError("HANDLER_ERROR", "TIMEOUT_ERROR", 408), ok({ final_execution_status: "EXECUTED_OPTIMISTIC", status: { SuccessValue: "" } })]);
    const rpc = new NearRpc([A, B], m);
    const out = await rpc.sendTx("c2lnbmVk");
    expect(out.final_execution_status).toBe("EXECUTED_OPTIMISTIC");
    expect(m.sent.map((s) => [s.url, s.body.method, s.body.params])).toEqual([
      [A, "send_tx", { signed_tx_base64: "c2lnbmVk", wait_until: "EXECUTED_OPTIMISTIC" }],
      [B, "send_tx", { signed_tx_base64: "c2lnbmVk", wait_until: "EXECUTED_OPTIMISTIC" }],
    ]);

    const n = mock([rpcError("HANDLER_ERROR", "INVALID_TRANSACTION")]);
    const err = await new NearRpc([A, B], n).sendTx("eA==").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(RpcError);
    expect((err as RpcError).causeName).toBe("INVALID_TRANSACTION");
    expect(n.sent).toHaveLength(1);
  });

  it("times out a request and moves on", async () => {
    const m = mock(["hang", ok({ header: { hash: "H", height: 1 } })]);
    const rpc = new NearRpc([A, B], { ...m, timeoutMs: 20 });
    await expect(rpc.block("optimistic")).resolves.toMatchObject({ header: { hash: "H" } });
    expect(m.sent.map((s) => s.url)).toEqual([A, B]);
  });

  it("gives up after its attempts, never quoting an endpoint", async () => {
    const m = mock(["network", "hang", { status: 502, text: "bad gateway" }]);
    const rpc = new NearRpc([A, B], { ...m, attempts: 3, timeoutMs: 20 });
    const err = await rpc.block().catch((e: unknown) => e);
    expect(err).toBeInstanceOf(RpcError);
    const msg = (err as RpcError).message;
    expect(msg).toBe("block: HTTP 502 at rpc #1");
    for (const s of ["SECRET", "example", "https"]) expect(msg).not.toContain(s);
    expect(m.sent).toHaveLength(3);
    const net = await new NearRpc([A], { ...mock(["network"]), attempts: 1 }).block().catch((e: unknown) => e);
    expect((net as RpcError).message).toBe("block: network error at rpc #1");
  });

  it("reads access keys and accounts", async () => {
    const m = mock([
      ok({ nonce: 271000000001, permission: "FullAccess", block_height: 5, block_hash: "6URwbPipuA4MJLG7LCRRZuWnms3JZ9cRG3z9indXWz8G" }),
      rpcError("HANDLER_ERROR", "UNKNOWN_ACCOUNT"),
      ok({ amount: "1", locked: "0", code_hash: "11111111111111111111111111111111", storage_usage: 182, block_height: 5, block_hash: "h" }),
      ok({ nonce: 2 ** 60, permission: "FullAccess", block_height: 5, block_hash: "h" }),
    ]);
    const rpc = new NearRpc([A], m);
    const ak = await rpc.viewAccessKey("web-vector.practice.antibody.testnet", "ed25519:FnDw11RnMuVPfRYeo2h9aGj8siN4iWJTz5UwdLtKcfA4");
    expect(ak.nonce).toBe(BigInt(271000000001));
    expect(ak.blockHash).toBe("6URwbPipuA4MJLG7LCRRZuWnms3JZ9cRG3z9indXWz8G");
    expect(m.sent[0].body.params).toEqual({
      request_type: "view_access_key",
      finality: "optimistic",
      account_id: "web-vector.practice.antibody.testnet",
      public_key: "ed25519:FnDw11RnMuVPfRYeo2h9aGj8siN4iWJTz5UwdLtKcfA4",
    });
    expect(await rpc.viewAccount("gone.testnet")).toBeNull();
    expect((await rpc.viewAccount("ann.practice.antibody.testnet"))?.storage_usage).toBe(182);
    expect(m.sent[2].body.params).toEqual({ request_type: "view_account", finality: "optimistic", account_id: "ann.practice.antibody.testnet" });
    await expect(rpc.viewAccessKey("a.testnet", "ed25519:x")).rejects.toThrow(/unexpected answer/);
  });

  it("asks for a transaction's status", async () => {
    const m = mock([ok({ final_execution_status: "FINAL", transaction: { hash: "T" } })]);
    const out = await new NearRpc([A], m).txStatus("T", "web-vector.practice.antibody.testnet", "FINAL");
    expect(txHash(out)).toBe("T");
    expect(m.sent[0].body).toMatchObject({ method: "tx", params: { tx_hash: "T", sender_account_id: "web-vector.practice.antibody.testnet", wait_until: "FINAL" } });
  });

  it("surfaces a failed view", async () => {
    const m = mock([rpcError("HANDLER_ERROR", "CONTRACT_EXECUTION_ERROR"), ok({ error: "wasm execution failed", logs: [] })]);
    const rpc = new NearRpc([A, B], m);
    await expect(rpc.viewFunction("x.testnet", "boom")).rejects.toThrow(/CONTRACT_EXECUTION_ERROR/);
    await expect(rpc.viewFunction("x.testnet", "boom")).rejects.toThrow(/view x.testnet.boom: "wasm execution failed"/);
    expect(m.sent).toHaveLength(2);
  });
});

describe("outcome helpers (near.rs failures, return_value, events)", () => {
  const b64 = (s: string) => base64.encode(new TextEncoder().encode(s));
  const outcome = {
    status: { SuccessValue: b64("42") },
    transaction: { hash: "4mxBB1jD9DVx7FNCDx9D7vqz1zg1RHkZ7gPfEKGXPNiy" },
    receipts_outcome: [
      { id: "r1", outcome: { logs: ['EVENT_JSON:{"standard":"antibody","event":"commit","data":{"commit_id":3}}', "plain log", "EVENT_JSON:{not json"], status: { SuccessValue: "" } } },
      { id: "r2", outcome: { logs: [], status: { Failure: { ActionError: { index: 0, kind: { FunctionCallError: { ExecutionError: "Smart contract panicked: nothing to allocate" } } } } } } },
    ],
  };

  it("reads the return value", () => {
    expect(returnValue(outcome)).toBe(42);
    expect(returnValue({ status: { SuccessValue: "" } })).toBeNull();
    expect(returnValue({ status: { SuccessValue: b64("not json") } })).toBeUndefined();
    expect(returnValue({ status: { Failure: {} } })).toBeUndefined();
    expect(returnValue({})).toBeUndefined();
  });

  it("collects every failure and every event", () => {
    expect(outcomeFailures(outcome)).toEqual(['{"ActionError":{"index":0,"kind":{"FunctionCallError":{"ExecutionError":"Smart contract panicked: nothing to allocate"}}}}']);
    expect(outcomeFailures({ status: { Failure: { InvalidTxError: "Expired" } }, receipts_outcome: [] })).toEqual(['{"InvalidTxError":"Expired"}']);
    expect(outcomeFailures({ status: "NotStarted" })).toEqual([]);
    expect(events(outcome)).toEqual([{ standard: "antibody", event: "commit", data: { commit_id: 3 } }]);
    expect(events({})).toEqual([]);
    expect(txHash(outcome)).toBe("4mxBB1jD9DVx7FNCDx9D7vqz1zg1RHkZ7gPfEKGXPNiy");
  });
});
