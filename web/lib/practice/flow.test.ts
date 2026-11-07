// The browser try's state machine (flow.ts) against a simulated chain. The RPC answers are built from the
// real shapes (web/tests/fixtures/api: program 6, its meta, commits and runs; golden/web-vectors.json:
// the public vend key and the practice program; fixtures/web/practice-base.bin: the pinned base). Every
// transaction is decoded and its signature checked, and the contract rules the try depends on are
// simulated: the hourly vend limit, commit order, the challenge window, close and reclaim. Nothing here
// touches a network.
import { readFileSync } from "node:fs";
import { ed25519 } from "@noble/curves/ed25519.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { base58, base64 } from "@scure/base";
import { describe, expect, it } from "vitest";
import { RpcError, type TxOutcome } from "../near/rpc";
import { parseBase, vaultAddress } from "./base";
import { ByteReader, concat, fromHex, toHex, u128le, u32le, utf8 } from "./bytes";
import { buildPracticeEnvelope } from "./envelope";
import { contentFromTx, parseLocation, simpleArgs, simpleContent } from "./fastfs";
import {
  commitStateLabel,
  type OpenStream,
  paidOf,
  pinnedBaseLocation,
  readableFailure,
  type RpcLike,
  runState,
  type SavedTry,
  type StreamEvent,
  type StreamHandlers,
  suggestName,
  type Timing,
  TryFlow,
  type TryStorage,
} from "./flow";
import { borshString, checkName, keyFromSeed, publicKeyString, publicVendSeed, TGAS, vendArgs } from "./near-tx";
import { commitment, openWithKey, practiceContentKey, type Rng } from "./seal";
import { nextHourMs, parseVendingConfigFull, parseVendingHour, vendsLeft } from "./vending";

type Json = Record<string, unknown>;

const repo = (rel: string) => new URL(`../../../${rel}`, import.meta.url);
const fixture = <T>(name: string): T => JSON.parse(readFileSync(new URL(`../../tests/fixtures/api/${name}`, import.meta.url), "utf8")) as T;
const web = JSON.parse(readFileSync(repo("golden/web-vectors.json"), "utf8")) as {
  public_vend_key: string;
  practice: { program_key: string; sealing_pk: string; base: { sha256: string }; tries: { vault_index: number; vault: string }[] };
};
const baseBytes = new Uint8Array(readFileSync(repo("fixtures/web/practice-base.bin")));
const base = parseBase(baseBytes, web.practice.base.sha256);
const program6 = fixture<{ program: Json; meta: Json }>("program-6.json");

const VENDING = "practice.antibody.testnet";
const ANTIBODY = "bounty.antibody.testnet";
const RELAYER = "relayer.antibody.testnet";
const KEEPER = "keeper.antibody.testnet";
const BASE_TX = "Hzdxw2Ai4xCwEmMgfL3ktdvkNTMG5Ym6yC644wzYLZmq";
const PAY_RAW = "1000000000000000000000";
const TAG = "cc52c19a12305290fa0b15741d08f4ec325bacf4242cdef37f4b153e13c33e51";
const vendPk = publicKeyString(keyFromSeed(publicVendSeed()).publicKey);
const decoder = new TextDecoder();
const tgas = (n: number) => BigInt(n) * TGAS;

class Panic extends Error {}

interface Sent {
  signer: string;
  publicKey: string;
  nonce: bigint;
  receiver: string;
  blockHash: string;
  method: string;
  args: Uint8Array;
  gas: bigint;
  deposit: bigint;
  hash: string;
  /** The chain's clock when it arrived. */
  at: number;
}

/** borsh(SignedTransaction) back to its fields; the signature must verify over SHA-256(tx). */
function decodeSigned(b64: string, at: number): Sent {
  const bytes = base64.decode(b64);
  const r = new ByteReader(bytes);
  const str = () => decoder.decode(r.take(r.u32le()));
  const signer = str();
  expect(r.u8()).toBe(0);
  const pk = r.take(32).slice();
  const nonce = r.u64le();
  const receiver = str();
  const blockHash = base58.encode(r.take(32));
  expect(r.u32le()).toBe(1);
  expect(r.u8()).toBe(2); // FunctionCall
  const method = str();
  const args = r.take(r.u32le()).slice();
  const gas = r.u64le();
  const dep = r.take(16);
  let deposit = BigInt(0);
  for (let i = 15; i >= 0; i--) deposit = (deposit << BigInt(8)) | BigInt(dep[i]);
  const tx = bytes.slice(0, r.offset);
  expect(r.u8()).toBe(0); // ED25519
  const sig = r.take(64);
  expect(r.remaining).toBe(0);
  const digest = sha256(tx);
  expect(ed25519.verify(sig, digest, pk)).toBe(true);
  return { signer, publicKey: publicKeyString(pk), nonce, receiver, blockHash, method, args, gas, deposit, hash: base58.encode(digest), at };
}

/** A node's refusal of a transaction, as NEAR RPCs answer it (near.rs matches the same words). */
function rpcErr(invalidTx: unknown): RpcError {
  const err = { name: "HANDLER_ERROR", cause: { name: "INVALID_TRANSACTION", info: {} }, code: -32000, message: "Server error", data: { TxExecutionError: { InvalidTxError: invalidTx } } };
  return new RpcError("send_tx", `send_tx: ${JSON.stringify(err)}`, err);
}

const ev = (standard: string, event: string, data: Json) => `EVENT_JSON:${JSON.stringify({ standard, version: "1.0.0", event, data: [data] })}`;

/** near-antibody, the vending contract and the practice accounts, as far as a try touches them. */
class FakeChain {
  t = Date.UTC(2026, 10, 7, 12, 0, 0);
  height = 276_600_000;
  latest = "";
  readonly blocks = new Map<string, number>();
  /** account → public key → nonce. */
  readonly keys = new Map<string, Map<string, bigint>>();
  readonly vended = new Map<string, { vault: number; done: boolean; at: number }>();
  vendedCount = 4;
  perHour = 6;
  private hour = -1;
  private inHour = 0;
  readonly program: Json = structuredClone(program6.program);
  readonly meta: Json = structuredClone(program6.meta);
  readonly commits = new Map<number, Json>();
  readonly order: number[] = [];
  readonly runs = new Map<number, Json>();
  nextCommit = 24;
  nextRun = 58;
  readonly sent: Sent[] = [];
  readonly attempts: Sent[] = [];
  readonly rejects: { method: string; make: (tx: Sent) => Error }[] = [];
  readonly panics: { method: string; message: string }[] = [];
  t1 = true;
  answerAfter = 14_000;
  t1After = 11_000;
  answerInReveal = false;
  /** Withholds the answer's time: no block time on the stream's run_answered, no Answered.at from get_run. */
  answerTimeHidden = false;
  readonly timers: { at: number; fn: () => void }[] = [];
  readonly log: StreamEvent[] = [];
  seq = 3000;
  private handlers?: StreamHandlers;
  readonly streamUrls: string[] = [];
  readonly views: string[] = [];

  constructor() {
    // Program 6's three earlier commits (indexes 0 to 2), settled; its cursor is past them.
    for (const [i, id] of [20, 21, 23].entries()) {
      this.commits.set(id, { id, index: i, program: 6, round: 0, hunter: `judge-${i + 1}-p6.${VENDING}`, beneficiary: VENDING, committed_at: 1793809238 + i * 48, run: 53 + i, state: { Done: { Paid: { amount: PAY_RAW, at: 1793809271, instant: true } } } });
      this.order.push(id);
    }
    expect(this.program.cursor).toBe(3);
    this.keys.set(VENDING, new Map([[vendPk, BigInt(199_000_000_123)]]));
    this.newBlock();
  }

  get nowS(): number {
    return Math.floor(this.t / 1000);
  }

  newBlock(): string {
    this.height += 1;
    const hash = base58.encode(sha256(utf8(`block ${this.height}`)));
    this.blocks.set(hash, this.t);
    this.latest = hash;
    return hash;
  }

  advance(ms: number): void {
    const end = this.t + ms;
    for (;;) {
      this.timers.sort((a, b) => a.at - b.at);
      const next = this.timers[0];
      if (!next || next.at > end) break;
      this.timers.shift();
      this.t = Math.max(this.t, next.at);
      next.fn();
    }
    this.t = end;
    this.newBlock();
  }

  emit(event: string, data: Json, tx: string, signer: string, standard = "antibody"): void {
    const e: StreamEvent = { seq: this.seq++, standard, event, data, tx, signer, block_ts_ns: this.answerTimeHidden && event === "run_answered" ? undefined : this.t * 1e6 };
    this.log.push(e);
    this.handlers?.event(e);
  }

  /** The API's stream: replays from `after` (inclusive), then each new event. */
  readonly openStream: OpenStream = (url, on) => {
    this.streamUrls.push(url);
    const after = Number(new URL(url).searchParams.get("after") ?? "0");
    this.handlers = on;
    queueMicrotask(() => {
      on.open();
      for (const e of [...this.log]) if ((e.seq ?? 0) >= after) on.event(e);
    });
    return () => {
      if (this.handlers === on) this.handlers = undefined;
    };
  };

  /** An earlier commit of program 6 (the test of a held queue). */
  addCommit(c: Json): number {
    const id = this.nextCommit++;
    this.commits.set(id, { id, index: this.order.length, program: 6, round: 0, beneficiary: VENDING, run: null, ...c });
    this.order.push(id);
    this.program.commits = this.order.length;
    return id;
  }

  /** The keeper: expires an unrevealed commit once its reveal window has passed. */
  keeperExpires(commitId: number, at: number): void {
    this.timers.push({
      at,
      fn: () => {
        const c = this.commits.get(commitId);
        const reveal = ((this.program.params as Json).windows as Json).reveal as number;
        if (c && c.state === "Committed" && this.nowS > (c.committed_at as number) + reveal) {
          c.state = { Done: "Expired" };
          this.newBlock();
          this.emit("expired", { commit_id: commitId }, base58.encode(sha256(utf8(`expire ${commitId}`))), KEEPER);
        }
      },
    });
  }

  rpc(): RpcLike {
    return {
      viewFunction: async (contract, method, args) => {
        const raw = args instanceof Uint8Array ? args : utf8(JSON.stringify(args ?? {}));
        this.views.push(`${contract}.${method}`);
        const bytes = this.view(contract, method, raw);
        return {
          bytes,
          logs: [],
          blockHeight: this.height,
          blockHash: this.latest,
          json<T = unknown>(): T {
            return JSON.parse(decoder.decode(bytes)) as T;
          },
        };
      },
      viewAccessKey: async (account, publicKey) => {
        const n = this.keys.get(account)?.get(publicKey);
        if (n === undefined) throw new RpcError("query", `access key ${publicKey} of ${account}: does not exist`);
        return { nonce: n, blockHash: this.latest, blockHeight: this.height, permission: "FunctionCall" };
      },
      viewAccount: async (account) => {
        if (!this.keys.has(account)) return null;
        return { amount: "280000000000000000000000", locked: "0", code_hash: "11111111111111111111111111111111", storage_usage: 300, block_height: this.height, block_hash: this.latest };
      },
      block: async () => ({ header: { height: this.height, hash: this.latest, prev_hash: "", timestamp_nanosec: (BigInt(this.t) * BigInt(1_000_000)).toString() } }),
      sendTx: async (signed) => this.process(signed),
      txStatus: async () => {
        throw new RpcError("tx", "tx: the regular RPC is not asked for old transactions");
      },
      call: async <T = unknown>(method: string, params: unknown): Promise<T> => {
        if (method !== "block") throw new Error(`unexpected RPC ${method}`);
        const id = (params as { block_id: string }).block_id;
        const t = this.blocks.get(id);
        if (t === undefined) throw new RpcError("block", "block: UNKNOWN_BLOCK");
        return { header: { hash: id, height: 0, timestamp_nanosec: (BigInt(t) * BigInt(1_000_000)).toString() } } as T;
      },
    };
  }

  /** The archival RPC: the base witness's FastFS upload. */
  archival(): RpcLike {
    const none = async (): Promise<never> => {
      throw new Error("the archival RPC is asked for old transactions only");
    };
    return {
      viewFunction: none,
      viewAccessKey: none,
      viewAccount: none,
      block: none,
      sendTx: none,
      call: none,
      async txStatus(hash, signer, waitUntil) {
        expect(waitUntil).toBe("INCLUDED_FINAL");
        if (hash !== BASE_TX || signer !== "antibody.testnet") throw new RpcError("tx", "tx: UNKNOWN_TRANSACTION");
        const args = base64.encode(simpleArgs(`antibody/base/${web.practice.base.sha256}.bin`, "application/octet-stream", baseBytes));
        return {
          final_execution_status: "FINAL",
          transaction: { hash, signer_id: signer, receiver_id: ANTIBODY, nonce: 1, actions: [{ FunctionCall: { method_name: "__fastdata_fastfs", args, gas: 1, deposit: "0" } }] },
        };
      },
    };
  }

  private view(contract: string, method: string, raw: Uint8Array): Uint8Array {
    const json = (v: unknown) => utf8(JSON.stringify(v));
    if (contract === VENDING && method === "get_config") {
      expect(raw.length).toBe(0); // the CLI reads it with empty arguments
      return concat(
        borshString("antibody.testnet"),
        borshString(ANTIBODY),
        borshString("wrap.testnet"),
        u32le(6),
        u32le(24),
        u128le(BigInt("280000000000000000000000")),
        u128le(BigInt("400000000000000000000000")),
        u128le(BigInt("10000000000000000000000")),
        u128le(BigInt("100000000000000000000")),
        u32le(this.perHour),
        u32le(this.vendedCount),
      );
    }
    if (contract === VENDING && method === "vault_of") {
      const r = new ByteReader(raw);
      const v = this.vended.get(decoder.decode(r.take(r.u32le())));
      return v ? concat(Uint8Array.of(1), u32le(v.vault)) : Uint8Array.of(0);
    }
    if (contract === ANTIBODY) {
      const a = JSON.parse(decoder.decode(raw)) as Json;
      switch (method) {
        case "get_program":
          return json(a.program_id === 6 ? this.program : null);
        case "get_program_meta":
          return json(a.program_id === 6 ? this.meta : null);
        case "get_account":
          return json({ agent: this.vended.has(String(a.account_id)) ? { owner: VENDING, budget: "10000000000000000000000", per_commit_cap: "1000000000000000000000" } : null, committed: false, near: "0", tokens: "0" });
        case "get_commit":
          return json(this.commits.get(Number(a.commit_id)) ?? null);
        case "get_run": {
          const run = this.runs.get(Number(a.run_id)) ?? null;
          if (!run || !this.answerTimeHidden) return json(run);
          // A copy: the chain's own record keeps the time, for the test to compare against.
          const copy = structuredClone(run);
          const answered = (copy.outcome as Json | undefined)?.Answered as Json | undefined;
          if (answered) delete answered.at;
          return json(copy);
        }
        case "list_commits": {
          const from = Number(a.from ?? 0);
          return json(this.order.slice(from, from + Number(a.limit ?? 20)).map((id) => this.commits.get(id)));
        }
      }
    }
    throw new RpcError("query", `view ${contract}.${method}: MethodNotFound`);
  }

  private async process(signed: string): Promise<TxOutcome> {
    const tx = decodeSigned(signed, this.t);
    this.attempts.push(tx);
    const rej = this.rejects.findIndex((r) => r.method === tx.method);
    if (rej >= 0) throw this.rejects.splice(rej, 1)[0].make(tx);
    const keys = this.keys.get(tx.signer);
    const ak = keys?.get(tx.publicKey);
    if (!keys || ak === undefined) throw rpcErr({ InvalidAccessKeyError: { AccessKeyNotFound: { account_id: tx.signer, public_key: tx.publicKey } } });
    if (!this.blocks.has(tx.blockHash)) throw rpcErr("Expired");
    if (tx.nonce <= ak) throw rpcErr({ InvalidNonce: { ak_nonce: Number(ak), tx_nonce: Number(tx.nonce) } });
    // The access keys' permissions: the vend key calls vend and reclaim on the vending contract; a practice
    // key only its own account's methods.
    if (tx.signer === VENDING) {
      expect(tx.publicKey).toBe(vendPk);
      expect(tx.receiver).toBe(VENDING);
      expect(["vend", "reclaim"]).toContain(tx.method);
    } else {
      expect(tx.receiver).toBe(tx.signer);
      expect(["commit", "reveal", "retry", "allocate", "pay", "close"]).toContain(tx.method);
    }
    expect(tx.deposit).toBe(BigInt(0));
    keys.set(tx.publicKey, tx.nonce);
    this.sent.push(tx);
    const txBlock = this.newBlock();
    this.advance(600);
    const receipts: Json[] = [];
    const receipt = (logs: string[], status: Json = { SuccessValue: "" }) => {
      receipts.push({ id: `${tx.hash}-${receipts.length}`, block_hash: this.newBlock(), outcome: { logs, status, executor_id: tx.receiver } });
    };
    let status: Json;
    try {
      const ret = this.execute(tx, receipt);
      status = { SuccessValue: ret === undefined ? "" : base64.encode(utf8(JSON.stringify(ret))) };
    } catch (e) {
      if (!(e instanceof Panic)) throw e;
      const failure = { ActionError: { index: 0, kind: { FunctionCallError: { ExecutionError: `Smart contract panicked: ${e.message}` } } } };
      receipt([], { Failure: failure });
      status = { Failure: failure };
    }
    this.advance(600);
    return {
      final_execution_status: "EXECUTED_OPTIMISTIC",
      status,
      transaction: { hash: tx.hash, signer_id: tx.signer, receiver_id: tx.receiver, nonce: Number(tx.nonce) },
      transaction_outcome: { id: tx.hash, block_hash: txBlock, outcome: { logs: [], status: { SuccessReceiptId: "r" } } },
      receipts_outcome: receipts,
    } as unknown as TxOutcome;
  }

  private isFinal(c: Json): boolean {
    const r = this.runs.get(c.run as number);
    const a = (r?.outcome as Json | undefined)?.Answered as Json | undefined;
    if (!a) return false;
    return a.tier === "T1" || this.nowS > (a.at as number) + 60;
  }

  private answer(runId: number, tx: string, signer: string): Json {
    const r = this.runs.get(runId) as Json;
    const keys = [{ slot: 0, tag: TAG }];
    r.outcome = {
      Answered: { at: this.nowS, facts: { extra: "None", impact_usd_micro: "0", keys, kind: "Violation", protected_harm_usd_micro: "0", reason: null, receipt_sha256: "e5".repeat(32) }, output_sha256: "84".repeat(32), tier: "T0" },
    };
    const data = { commit_id: r.commit, keys, kind: "Violation", mode: "verify", receipt_sha256: "e5".repeat(32), run_id: runId, tier: "T0" };
    this.emit("run_answered", data, tx, signer);
    if (this.t1) {
      this.timers.push({
        at: this.t + this.t1After,
        fn: () => {
          ((r.outcome as Json).Answered as Json).tier = "T1";
          this.newBlock();
          this.emit("t1_confirmed", { commit_id: r.commit, mode: "verify", run_id: runId }, base58.encode(sha256(utf8(`t1 ${runId}`))), RELAYER);
        },
      });
    }
    return data;
  }

  private execute(tx: Sent, receipt: (logs: string[], status?: Json) => void): unknown {
    const p = this.panics.findIndex((x) => x.method === tx.method);
    if (p >= 0) throw new Panic(this.panics.splice(p, 1)[0].message);
    const text = () => decoder.decode(tx.args);
    switch (tx.method) {
      case "vend": {
        expect(tx.gas).toBe(tgas(60));
        const r = new ByteReader(tx.args);
        const name = decoder.decode(r.take(r.u32le()));
        expect(r.u32le()).toBe(33);
        expect(r.u8()).toBe(0);
        const pk = publicKeyString(r.take(32).slice());
        expect(r.remaining).toBe(0);
        const hour = Math.floor(this.t / 3_600_000);
        if (hour !== this.hour) {
          this.hour = hour;
          this.inHour = 0;
        }
        if (this.inHour >= this.perHour) throw new Panic("the hour's practice accounts are vended; try again later");
        this.inHour += 1;
        const account = `${name}.${VENDING}`;
        if (this.vended.has(account)) throw new Panic("that practice account exists");
        const vault = this.vendedCount % 24;
        this.vendedCount += 1;
        this.vended.set(account, { vault, done: false, at: this.nowS });
        this.keys.set(account, new Map([[pk, BigInt(this.height) * BigInt(1_000_000)]]));
        const data = { account, vault, program_id: 6 };
        receipt([ev("antibody-practice", "vended", data)]);
        this.emit("vended", data, tx.hash, VENDING, "antibody-practice");
        receipt([]);
        return undefined;
      }
      case "commit": {
        expect(tx.gas).toBe(tgas(25));
        const a = JSON.parse(text()) as Json;
        expect(Object.keys(a)).toEqual(["commitment", "program_id", "round", "terms_sha256"]);
        expect([a.program_id, a.round, a.terms_sha256]).toEqual([6, 0, this.meta.terms_sha256]);
        expect(String(a.commitment)).toMatch(/^[0-9a-f]{64}$/);
        const id = this.addCommit({ hunter: tx.signer, commitment: a.commitment, blob_sha256: null, bond: "100000000000000000000", committed_at: this.nowS, state: "Committed" });
        const data = { commit_id: id, hunter: tx.signer, index: this.order.length - 1, program_id: 6, round: 0 };
        receipt([]);
        receipt([ev("antibody", "commit", data)]);
        this.emit("commit", data, tx.hash, tx.signer);
        return id;
      }
      case "reveal": {
        expect(tx.gas).toBe(tgas(200));
        const a = JSON.parse(text()) as { commit_id: number; blob: { inline: string } };
        expect(Object.keys(a)).toEqual(["blob", "commit_id"]);
        const c = this.commits.get(a.commit_id);
        if (!c || c.state !== "Committed") throw new Panic("only a committed commit is revealed");
        const blob = fromHex(a.blob.inline);
        if (blob.length > 15 * 1024) throw new Panic("inline blobs are at most 15 KB; upload larger ones with FastFS");
        if (toHex(commitment(String(this.program.key), 0, String(c.hunter), sha256(blob))) !== c.commitment) throw new Panic("the blob does not match the commitment");
        const runId = this.nextRun++;
        this.runs.set(runId, { id: runId, commit: c.id, mode: "verify", program: 6, round: 0, payer: VENDING, signer: tx.signer, started_at: this.nowS, outcome: "Pending" });
        c.state = "Verifying";
        c.run = runId;
        c.blob_sha256 = toHex(sha256(blob));
        const requested = { commit_id: c.id, mode: "verify", payer: VENDING, program_id: 6, request_sha256: "0a".repeat(32), run_id: runId };
        receipt([]);
        receipt([ev("antibody", "run_requested", requested), ev("antibody", "reveal", { commit_id: c.id, run_id: runId })]);
        this.emit("run_requested", requested, tx.hash, tx.signer);
        this.emit("reveal", { commit_id: c.id, run_id: runId }, tx.hash, tx.signer);
        if (this.answerInReveal) {
          // OutLayer's answer resumes the reveal's own transaction tree (as on testnet).
          this.advance(this.answerAfter);
          receipt([ev("antibody", "run_answered", this.answer(runId, tx.hash, tx.signer))]);
        } else {
          this.timers.push({ at: this.t + this.answerAfter, fn: () => void this.answer(runId, tx.hash, tx.signer) });
        }
        return undefined;
      }
      case "allocate": {
        expect(tx.gas).toBe(tgas(40));
        const a = JSON.parse(text()) as Json;
        expect(Object.keys(a)).toEqual(["max", "program_id"]);
        expect(a.program_id).toBe(6);
        const logs: string[] = [];
        let n = 0;
        let cursor = this.program.cursor as number;
        while (n < (a.max as number) && cursor < this.order.length) {
          const c = this.commits.get(this.order[cursor]) as Json;
          if (c.state === "Committed") break;
          if (c.state === "Verifying") {
            if (!this.isFinal(c)) break;
            c.state = { Holder: { amount: PAY_RAW, reserved: PAY_RAW, debt: "0", new_keys: [TAG] } };
            const data = { amount: PAY_RAW, commit_id: c.id, debt: "0", keys: [TAG], reserved: PAY_RAW };
            logs.push(ev("antibody", "holder", data));
            this.emit("holder", data, tx.hash, tx.signer);
          }
          cursor += 1;
          n += 1;
        }
        this.program.cursor = cursor;
        if (n === 0 && logs.length === 0) throw new Panic("nothing to allocate");
        receipt([]);
        receipt(logs);
        return undefined;
      }
      case "pay": {
        expect(tx.gas).toBe(tgas(30));
        const a = JSON.parse(text()) as { commit_id: number };
        const c = this.commits.get(a.commit_id);
        if (!c || commitStateLabel(c.state) !== "Holder") throw new Panic("only a holder or a promoted claim is paid");
        const ans = ((this.runs.get(c.run as number) as Json).outcome as Json).Answered as Json;
        const instant = ans.tier === "T1";
        if (!instant && this.nowS <= (ans.at as number) + 60) throw new Panic("waits its window: no T1, above the cap, or the instant budget is spent");
        c.state = { Done: { Paid: { amount: PAY_RAW, at: this.nowS, instant } } };
        const data = { amount: PAY_RAW, beneficiary: VENDING, commit_id: c.id, fee: "0", instant, program_id: 6 };
        receipt([]);
        receipt([ev("antibody", "paid", data)]);
        this.emit("paid", data, tx.hash, tx.signer);
        return undefined;
      }
      case "close": {
        expect(tx.gas).toBe(tgas(15));
        expect(text()).toBe("{}");
        const v = this.vended.get(tx.signer);
        if (!v) throw new Panic("not a practice account vended here");
        v.done = true;
        receipt([]);
        receipt([]);
        return undefined;
      }
      case "reclaim": {
        expect(tx.gas).toBe(tgas(20));
        const r = new ByteReader(tx.args);
        const account = decoder.decode(r.take(r.u32le()));
        expect(r.remaining).toBe(0);
        const v = this.vended.get(account);
        if (!v) throw new Panic("not a practice account vended here");
        if (!v.done && this.nowS < v.at + 3600) throw new Panic("a practice account may be reclaimed once its try is done, or an hour after its vend");
        this.keys.delete(account); // deleted; the vending contract keeps its name
        receipt([]);
        receipt([]);
        return undefined;
      }
    }
    throw new Panic(`MethodNotFound: ${tx.method}`);
  }
}

/** Deterministic draws, recorded in order: the key's seed, the salt, then the seal's e, nonce_w, nonce_b. */
function counterRng(): { rng: Rng; draws: Uint8Array[] } {
  let i = 0;
  const draws: Uint8Array[] = [];
  const rng: Rng = (n) => {
    const b = sha256(utf8(`draw ${i++}`)).slice(0, n);
    draws.push(b);
    return b;
  };
  return { rng, draws };
}

function memoryStorage(): TryStorage & { saves: SavedTry[] } {
  const saves: SavedTry[] = [];
  return {
    saves,
    load: () => saves.at(-1) ?? null,
    save: (t) => void saves.push(structuredClone(t)),
    clear: () => void (saves.length = 0),
  };
}

function setup(o: { stream?: boolean; timing?: Partial<Timing>; chain?: FakeChain; sleep?: (ms: number) => Promise<void>; skewMs?: number } = {}) {
  const chain = o.chain ?? new FakeChain();
  const { rng, draws } = counterRng();
  const storage = memoryStorage();
  const flow = new TryFlow({
    rpc: chain.rpc(),
    archival: chain.archival(),
    apiBase: "https://api.example.test",
    streamAfter: chain.seq - 10,
    openStream: o.stream === false ? null : chain.openStream,
    now: () => chain.t + (o.skewMs ?? 0),
    sleep: o.sleep ?? (async (ms) => chain.advance(ms)),
    random: rng,
    storage,
    timing: o.timing,
  });
  return { chain, flow, draws, storage };
}

const methods = (c: FakeChain) => c.sent.map((t) => t.method);

// ------------------------------------------------------------------------------------------ helpers

describe("the helpers a try reads the chain with", () => {
  it("reads FastFS uploads as fastfs.rs does", () => {
    const args = simpleArgs("antibody/blob.bin", "application/octet-stream", utf8("hello"));
    expect(decoder.decode(simpleContent(args))).toBe("hello");
    const result = { transaction: { actions: [{ Transfer: {} }, { FunctionCall: { method_name: "__fastdata_fastfs", args: base64.encode(args) } }] } };
    expect(decoder.decode(contentFromTx(result))).toBe("hello");
    expect(parseLocation("near-tx:9xYz:alice.testnet")).toEqual({ hash: "9xYz", signer: "alice.testnet" });
    expect(parseLocation("https://x")).toBeNull();
    expect(parseLocation("near-tx::alice.testnet")).toBeNull();
    expect(() => simpleContent(Uint8Array.of(1))).toThrow(/only FastfsData::Simple/);
    expect(() => simpleContent(concat(Uint8Array.of(0), u32le(1), utf8("a"), Uint8Array.of(0)))).toThrow(/no content/);
    expect(() => simpleContent(args.subarray(0, args.length - 1))).toThrow(/truncated/);
    expect(() => contentFromTx({ transaction: { actions: [] } })).toThrow(/no __fastdata_fastfs call/);
    expect(() => contentFromTx({})).toThrow(/no transaction/);
  });

  it("reads the vending contract's whole get_config, and its program id as the CLI does", () => {
    const chain = new FakeChain();
    const raw = concat(borshString("antibody.testnet"), borshString(ANTIBODY), borshString("wrap.testnet"), u32le(6), u32le(24));
    const full = concat(raw, u128le(BigInt(280)), u128le(BigInt(400)), u128le(BigInt(10)), u128le(BigInt(1)), u32le(6), u32le(9));
    expect(parseVendingConfigFull(full)).toEqual({
      owner: "antibody.testnet",
      antibody: ANTIBODY,
      token: "wrap.testnet",
      programId: 6,
      vaults: 24,
      terms: { gasFunds: BigInt(280), allowance: BigInt(400), bondBudget: BigInt(10), perCommitCap: BigInt(1), perHour: 6 },
      vended: 9,
    });
    // An older, shorter config still names its program.
    const short = parseVendingConfigFull(raw);
    expect([short.programId, short.vaults, short.terms, short.vended]).toEqual([6, 24, undefined, undefined]);
    expect(chain.perHour).toBe(6);
    expect(nextHourMs(Date.UTC(2026, 10, 7, 12, 59, 59))).toBe(Date.UTC(2026, 10, 7, 13, 0, 0));
  });

  it("reads the hour's vend count from the vending contract's own state, as testnet holds it", () => {
    // The start of practice.antibody.testnet's STATE (view_state, block 276623227): get_config's fields, then
    // hour 498325 (2026-11-06 13:00 UTC) and in_hour 1, then the accounts map's prefix "a" and the account code.
    const state = base64.decode(
      "EAAAAGFudGlib2R5LnRlc3RuZXQXAAAAYm91bnR5LmFudGlib2R5LnRlc3RuZXQMAAAAd3JhcC50ZXN0bmV0BgAAABgAAAAAAAB/axCW1Eo7AAAAAAAAAAAA2iuFHwu0VAAAAAAAAAAAQLK6yeAZHgIAAAAAAAAAAKDexa3JNTYAAAAAAAAABgAAAAcAAACVmgcAAAAAAAEAAAABAAAAYUYJAAA=",
    );
    const cfg = parseVendingConfigFull(state);
    expect([cfg.antibody, cfg.programId, cfg.vaults, cfg.terms?.perHour, cfg.vended]).toEqual([ANTIBODY, 6, 24, 6, 7]);
    const h = parseVendingHour(state, cfg);
    expect(h).toEqual({ hour: 498325, inHour: 1 });
    if (!h) return;
    // The count is the hour's only while that hour lasts: vend starts a new one.
    expect(vendsLeft(6, h, Date.UTC(2026, 10, 6, 13, 59, 59))).toBe(5);
    expect(vendsLeft(6, h, Date.UTC(2026, 10, 6, 14, 0, 0))).toBe(6);
    expect(vendsLeft(6, { hour: 498325, inHour: 6 }, Date.UTC(2026, 10, 6, 13, 30))).toBe(0);
    // A tab clock a little behind the chain's hour still counts it.
    expect(vendsLeft(6, h, Date.UTC(2026, 10, 6, 12, 59, 58))).toBe(5);
    // Another program, a truncated state, or a count over the limit: unknown, never a guess.
    expect(parseVendingHour(state, { ...cfg, programId: 7 })).toBeUndefined();
    expect(parseVendingHour(state.subarray(0, 150), cfg)).toBeUndefined();
    const over = state.slice();
    over[151] = 7;
    expect(parseVendingHour(over, cfg)).toBeUndefined();
  });

  it("names commit states and runs as cmd.rs state_label and run_view do", () => {
    const c20 = fixture<{ commit: Json }>("commit-20.json").commit;
    expect(commitStateLabel(c20.state)).toBe("Done:Paid");
    expect(paidOf(c20)).toBe(PAY_RAW);
    expect(commitStateLabel("Verifying")).toBe("Verifying");
    expect(commitStateLabel({ Holder: { amount: "1" } })).toBe("Holder");
    expect(commitStateLabel({ Done: "Expired" })).toBe("Done:Expired");
    expect(commitStateLabel(null)).toBe("");
    expect(paidOf({ state: "Verifying" })).toBe("0");
    const r53 = runState(fixture<{ raw: Json }>("run-53.json").raw);
    expect(r53.verdict).toEqual({ kind: "Violation", keys: [{ slot: 0, tag: TAG }], tier: "T1", receipt: "e51759c61d379da5947f1d38291b0efa4bd0d90634a72d9411bd9eda4fd119df", reason: null, answeredAt: 1793809254 });
    expect(runState(fixture<{ raw: Json }>("run-57.json").raw).verdict?.tier).toBe("T0");
    expect(runState({ outcome: "Pending" })).toEqual({ pending: true });
    expect(runState({ outcome: { Failed: { at: 1, reason: "run timeout" } } })).toEqual({ failed: "run timeout" });
  });

  it("picks the pinned base as cmd.rs pinned_base_at does", () => {
    const meta = program6.meta as Parameters<typeof pinnedBaseLocation>[0];
    expect(pinnedBaseLocation(meta, 0)).toEqual({ location: `near-tx:${BASE_TX}:antibody.testnet`, sha256: web.practice.base.sha256 });
    const rounds = { ...meta, round_bases: [{ location: "near-tx:R1:a.testnet", sha256: "0xAB" }, null] };
    expect(pinnedBaseLocation(rounds, 2)).toEqual({ location: "near-tx:R1:a.testnet", sha256: "ab" });
    expect(pinnedBaseLocation(rounds, 0).location).toBe(`near-tx:${BASE_TX}:antibody.testnet`);
    expect(() => pinnedBaseLocation({ base: null }, 0)).toThrow(/no pinned base/);
  });

  it("suggests names the vending contract takes, and says failures in words", () => {
    for (let i = 0; i < 20; i++) {
      const n = suggestName();
      expect(n).toMatch(/^judge-[a-z0-9]{4}$/);
      expect(() => checkName(n)).not.toThrow();
    }
    expect(readableFailure('{"ActionError":{"index":0,"kind":{"FunctionCallError":{"ExecutionError":"Smart contract panicked: nothing to allocate"}}}}')).toBe("nothing to allocate");
    expect(readableFailure('{"InvalidTxError":"Expired"}')).toBe('{"InvalidTxError":"Expired"}');
  });
});

// ------------------------------------------------------------------------------------------ the try

describe("the browser try", () => {
  it("vends, proves, settles and closes: Done:Paid, every transaction as the CLI signs it", async () => {
    const { chain, flow, draws, storage } = setup();
    const name = "judge-ab12";
    const account = `${name}.${VENDING}`;
    const seen: string[] = [];
    flow.subscribe((s) => seen.push(s.phase));
    await flow.start(name);
    await flow.settled();
    const s = flow.snapshot;

    expect(s.error).toBeUndefined();
    expect(s.phase).toBe("done");
    expect(seen[0]).toBe("running");
    expect([s.state, s.paidRaw, s.beneficiary, s.tier, s.closed]).toEqual(["Done:Paid", PAY_RAW, VENDING, "T1", true]);
    for (const step of Object.values(s.steps)) expect([step.id, step.status]).toEqual([step.id, "done"]);
    expect([s.programId, s.programKey, s.round, s.perHour, s.challengeSecs, s.revealSecs]).toEqual([6, web.practice.program_key, 0, 6, 60, 600]);

    // The transactions, in the CLI's order, signed by the CLI's keys, to the CLI's receivers, with its gas.
    expect(methods(chain)).toEqual(["vend", "commit", "reveal", "allocate", "pay", "close", "reclaim"]);
    const key = keyFromSeed(draws[0]);
    const pk = publicKeyString(key.publicKey);
    expect(s.publicKey).toBe(pk);
    expect(chain.sent.map((t) => [t.signer, t.publicKey, t.receiver])).toEqual([
      [VENDING, web.public_vend_key, VENDING],
      [account, pk, account],
      [account, pk, account],
      [account, pk, account],
      [account, pk, account],
      [account, pk, account],
      [VENDING, web.public_vend_key, VENDING],
    ]);
    expect(chain.sent.map((t) => t.gas / TGAS)).toEqual([60, 25, 200, 40, 30, 15, 20].map(BigInt));
    expect(toHex(chain.sent[0].args)).toBe(toHex(vendArgs(name, key.publicKey)));
    expect(toHex(chain.sent[6].args)).toBe(toHex(borshString(account)));
    expect(s.signed).toBe(7);

    // The vault and the base: vault 4 of the base's manifest, the base checked against its pinned hash.
    expect(s.vault).toBe(4);
    expect(s.vaultAddress).toBe(web.practice.tries.find((t) => t.vault_index === 4)?.vault);
    expect(s.base).toMatchObject({ location: `near-tx:${BASE_TX}:antibody.testnet`, sha256: web.practice.base.sha256, bytes: baseBytes.length });

    // The proof: the tutorial bundle under the drawn salt, sealed with the public content key, committed.
    const reveal = JSON.parse(decoder.decode(chain.sent[2].args)) as { blob: { inline: string }; commit_id: number };
    const blob = fromHex(reveal.blob.inline);
    const payload = openWithKey(blob, practiceContentKey(web.practice.program_key, account), web.practice.program_key);
    expect(payload.hunter).toBe(account);
    expect(toHex(payload.envelope)).toBe(toHex(buildPracticeEnvelope(base, vaultAddress(base.manifest, 4), draws[1])));
    expect(payload.report.length).toBe(0);
    const commitArgs = decoder.decode(chain.sent[1].args);
    const cm = toHex(commitment(web.practice.program_key, 0, account, sha256(blob)));
    expect(commitArgs).toBe(`{"commitment":"${cm}","program_id":6,"round":0,"terms_sha256":"${String(chain.meta.terms_sha256)}"}`);
    expect(s.sealed).toMatchObject({ commitment: cm, sha256: toHex(sha256(blob)), bytes: blob.length, saltHex: toHex(draws[1]), envelopeHex: toHex(payload.envelope) });
    expect(s.sealed?.contentKeyHex).toBe(toHex(practiceContentKey(web.practice.program_key, account)));
    expect([s.commitId, s.runId, reveal.commit_id]).toEqual([24, 58, 24]);

    // Followed live: the stream from the server's cursor; times are block times, from the commit.
    expect(chain.streamUrls).toEqual([`https://api.example.test/v1/stream?scope=antibody&after=${3000 - 10}`]);
    expect(s.verdict).toMatchObject({ kind: "Violation", keys: [{ slot: 0, tag: TAG }], tier: "T1" });
    const commitAt = s.steps.commit.at ?? 0;
    expect(s.steps.commit.exact).toBe(true);
    expect(s.steps.verdict.exact).toBe(true);
    expect(s.steps.t1.exact).toBe(true);
    expect((s.steps.t1.at ?? 0) - (s.steps.verdict.at ?? 0)).toBeCloseTo(chain.t1After, -1);
    expect((s.steps.verdict.at ?? 0) - commitAt).toBeGreaterThan(chain.answerAfter);
    expect((s.steps.pay.at ?? 0) - commitAt).toBeGreaterThan((s.steps.t1.at ?? 0) - commitAt);
    expect(s.steps.vend.at ?? Infinity).toBeLessThan(commitAt);
    expect(s.steps.t1.tx).toBe(base58.encode(sha256(utf8("t1 58"))));
    expect(s.steps.pay.tx).toBe(chain.sent[4].hash);

    // The tab kept the key (for a reload) and now records the try as closed.
    const last = storage.saves.at(-1);
    expect(last).toMatchObject({ v: 1, name, account, vending: VENDING, seedHex: toHex(draws[0]), programId: 6, vault: 4, commitId: 24, runId: 58, closed: true, finalState: "Done:Paid" });
    expect(storage.saves[0].seedHex).toBe(toHex(draws[0]));
    expect(storage.saves[0].vault).toBeUndefined(); // kept before the vend, so a reload can close the account
    expect(storage.saves.some((t) => t.vendSent === true && t.vault === undefined)).toBe(true); // marked before the vend is sent
    expect(chain.keys.has(account)).toBe(false);
  });

  it("retries the shared vend key's nonce (InvalidNonce) and an expired block hash, as near.rs send does", async () => {
    const { chain, flow } = setup();
    // Another visitor used the public vend key meanwhile: the node answers with the key's nonce.
    chain.rejects.push({
      method: "vend",
      make: (tx) => {
        const k = chain.keys.get(VENDING) as Map<string, bigint>;
        const ak = (k.get(vendPk) as bigint) + BigInt(7);
        k.set(vendPk, ak);
        return rpcErr({ InvalidNonce: { ak_nonce: Number(ak), tx_nonce: Number(tx.nonce) } });
      },
    });
    chain.rejects.push({ method: "commit", make: () => rpcErr("Expired") });
    await flow.start("judge-nonce");
    const s = flow.snapshot;
    expect(s.error).toBeUndefined();
    expect(s.state).toBe("Done:Paid");
    const vends = chain.attempts.filter((t) => t.method === "vend");
    expect(vends).toHaveLength(2);
    expect(vends[1].nonce).toBe(vends[0].nonce + BigInt(7)); // ak_nonce + 1
    const commits = chain.attempts.filter((t) => t.method === "commit");
    expect(commits).toHaveLength(2);
    expect(commits[1].nonce).toBe(commits[0].nonce + BigInt(1));
    expect(methods(chain)).toEqual(["vend", "commit", "reveal", "allocate", "pay", "close", "reclaim"]);
  });

  it("takes the T0 path when T1 does not come: settles as soon as the challenge window allows", async () => {
    const { chain, flow } = setup();
    chain.t1 = false;
    await flow.start("judge-t0");
    const s = flow.snapshot;
    expect(s.error).toBeUndefined();
    expect(s.t0Note).toBe("not attested on NEAR within the 60 s challenge window: paid after it, without attestation");
    expect(s.steps.t1).toMatchObject({ status: "skipped", detail: s.t0Note });
    expect(s.log.some((l) => l.level === "warn" && l.text === s.t0Note)).toBe(true);
    expect([s.tier, s.state]).toEqual(["T0", "Done:Paid"]);
    const answered = (s.verdict?.answeredAt ?? 0) * 1000;
    const allocate = chain.sent.find((t) => t.method === "allocate");
    // Not before the contract allows a T0 payout, and not minutes later (the CLI's wait, crates/cli/src/payable.rs, is the same).
    expect(allocate?.at ?? 0).toBeGreaterThanOrEqual(answered + 65_000);
    expect((allocate?.at ?? 0) - answered).toBeLessThan(80_000);
    expect(s.t0PayAfter).toBe(answered + 65_000);
  });

  it("with a T1 wait (t1Secs) shorter than the challenge window, still pays only after the window", async () => {
    const { chain, flow } = setup({ timing: { t1Secs: 20 } });
    chain.t1 = false;
    await flow.start("judge-t0b");
    const s = flow.snapshot;
    expect(s.state).toBe("Done:Paid");
    expect(s.t0Note).toBe("not attested on NEAR within the 60 s challenge window: paid after it, without attestation");
    const answered = (s.verdict?.answeredAt ?? 0) * 1000;
    const allocate = chain.sent.find((t) => t.method === "allocate");
    expect(allocate?.at ?? 0).toBeGreaterThanOrEqual(answered + 65_000);
    expect((allocate?.at ?? 0) - answered).toBeLessThan(80_000);
  });

  it("with the answer's time unknown, waits t1Secs before the T0 path", async () => {
    // Neither the stream nor get_run says when the run was answered, so the try cannot time the window: it
    // waits t1Secs, set longer than the 60 s window (with a shorter wait it would allocate before the window ends).
    const { chain, flow } = setup({ timing: { t1Secs: 90 } });
    chain.t1 = false;
    chain.answerTimeHidden = true;
    await flow.start("judge-t0d");
    const s = flow.snapshot;
    expect(s.error).toBeUndefined();
    expect(s.state).toBe("Done:Paid");
    expect(s.verdict?.answeredAt).toBeUndefined();
    expect(s.t0Note).toBe("not attested on NEAR in the 90 s this try waits for it: paid after the 60 s challenge window, without attestation");
    // The chain's own record still holds the answer's time.
    const answered = ((((chain.runs.get(Number(s.runId)) as Json).outcome as Json).Answered as Json).at as number) * 1000;
    const allocates = chain.attempts.filter((t) => t.method === "allocate");
    expect(allocates).toHaveLength(1);
    expect(allocates[0].at).toBeGreaterThan(answered + 60_000);
    const note = s.log.find((l) => l.level === "warn" && l.text === s.t0Note);
    expect(note?.at ?? 0).toBeGreaterThanOrEqual(answered + 90_000);
  });

  it("times the T0 payout by the chain's clock: a tab clock 30 s fast waits for a final block past the window", async () => {
    const { chain, flow } = setup({ skewMs: 30_000 });
    chain.t1 = false;
    await flow.start("judge-t0c");
    const s = flow.snapshot;
    expect(s.error).toBeUndefined();
    expect([s.tier, s.state]).toEqual(["T0", "Done:Paid"]);
    const answered = (s.verdict?.answeredAt ?? 0) * 1000;
    const allocates = chain.sent.filter((t) => t.method === "allocate");
    // One allocate, on the chain's clock after the window: none sent early to read "nothing to allocate".
    expect(allocates).toHaveLength(1);
    expect(allocates[0].at).toBeGreaterThanOrEqual(answered + 61_000);
    expect(allocates[0].at - answered).toBeLessThan(80_000);
    expect(s.steps.allocate).toMatchObject({ status: "done" });
  });

  it("says which unrevealed commit holds the queue and when the keeper expires it, and keeps following", async () => {
    const chain = new FakeChain();
    const committedAt = chain.nowS - 30;
    const early = chain.addCommit({ hunter: `early-1.${VENDING}`, commitment: "00".repeat(32), committed_at: committedAt, state: "Committed" });
    chain.keeperExpires(early, (committedAt + 600 + 20) * 1000);
    const { flow } = setup({ chain });
    await flow.start("judge-held");
    const s = flow.snapshot;
    expect(s.error).toBeUndefined();
    expect(s.held).toMatchObject({ commitId: early, state: "Committed", committedAt, expiresAt: committedAt + 600 });
    expect(s.held?.message).toBe(
      `Claims are paid in commit order. Commit ${early}, committed earlier and not yet revealed, holds the queue until its 600 s reveal window ends; then the keeper expires it and yours is paid.`,
    );
    expect(s.log.some((l) => l.level === "warn" && l.text === s.held?.message)).toBe(true);
    // The first allocation finds nothing; after the keeper expires commit 24, the next one pays the queue.
    expect(methods(chain)).toEqual(["vend", "commit", "reveal", "allocate", "allocate", "pay", "close", "reclaim"]);
    const [first, second] = chain.sent.filter((t) => t.method === "allocate");
    expect(first.at).toBeLessThan((committedAt + 600) * 1000);
    expect(second.at).toBeGreaterThan((committedAt + 620) * 1000);
    expect(s.steps.allocate).toMatchObject({ status: "done", tx: second.hash });
    expect([s.state, s.closed]).toEqual(["Done:Paid", true]);
    expect(commitStateLabel(chain.commits.get(early)?.state)).toBe("Done:Expired");
  });

  it("still closes the account when a step fails after the vend", async () => {
    const { chain, flow } = setup();
    chain.panics.push({ method: "reveal", message: "inline blob: hex" });
    const account = `judge-fail.${VENDING}`;
    await flow.start("judge-fail");
    const s = flow.snapshot;
    expect(s.phase).toBe("failed");
    expect(s.error).toEqual({ kind: "chain", message: "reveal failed: inline blob: hex", closing: `${account} is closed, its NEAR back to ${VENDING}` });
    expect(s.steps.reveal).toMatchObject({ status: "failed", tx: chain.sent[2].hash });
    expect([s.steps.verdict.status, s.steps.allocate.status, s.steps.close.status, s.steps.reclaim.status]).toEqual(["waiting", "waiting", "done", "done"]);
    expect(methods(chain)).toEqual(["vend", "commit", "reveal", "close", "reclaim"]);
    expect(s.closed).toBe(true);
    expect(chain.keys.has(account)).toBe(false);
  });

  it("reads and checks the base witness before the vend: one that hashes otherwise costs the faucet nothing", async () => {
    const chain = new FakeChain();
    (chain.meta.base as Json).sha256 = "00".repeat(32);
    const { flow } = setup({ chain });
    await flow.start("judge-base");
    const s = flow.snapshot;
    expect(s.error?.kind).toBe("program");
    expect(s.error?.message).toBe(`the base witness hashes to ${web.practice.base.sha256}, not the pinned ${"00".repeat(32)}`);
    expect(s.error?.closing).toBeUndefined();
    expect([s.steps.base.status, s.steps.vend.status]).toEqual(["failed", "waiting"]);
    expect(chain.attempts).toHaveLength(0);
  });

  it("closes the account when a step after the vend fails (a vault the manifest does not name)", async () => {
    const chain = new FakeChain();
    const rpc = chain.rpc();
    // The vending contract answers vault 24; the manifest names vault-0 … vault-23.
    const wrapped: RpcLike = {
      ...rpc,
      viewFunction: async (contract, method, args, finality) => {
        const r = await rpc.viewFunction(contract, method, args, finality);
        return method === "vault_of" && r.bytes[0] === 1 ? { ...r, bytes: concat(Uint8Array.of(1), u32le(24)) } : r;
      },
    };
    const flow = new TryFlow({
      rpc: wrapped,
      archival: chain.archival(),
      apiBase: "https://api.example.test",
      openStream: chain.openStream,
      now: () => chain.t,
      sleep: async (ms) => chain.advance(ms),
      random: counterRng().rng,
    });
    await flow.start("judge-vault");
    const s = flow.snapshot;
    expect(s.error).toEqual({ kind: "program", message: "no vault-24 in the manifest", closing: `judge-vault.${VENDING} is closed, its NEAR back to ${VENDING}` });
    expect(s.steps.vend.status).toBe("failed");
    expect(methods(chain)).toEqual(["vend", "close", "reclaim"]);
  });

  it("refuses a taken name before any transaction (a closed account's name stays taken)", async () => {
    const { chain, flow } = setup();
    chain.keys.set(`taken.${VENDING}`, new Map());
    await flow.start("taken");
    expect(flow.snapshot.error).toEqual({ kind: "name", message: `taken.${VENDING} is taken: pick another name (a new one, lowercase)`, closing: undefined });
    chain.vended.set(`gone.${VENDING}`, { vault: 1, done: true, at: 0 });
    await flow.start("gone");
    expect(flow.snapshot.error?.message).toBe(`gone.${VENDING} is taken: pick another name (a new one, lowercase)`);
    await flow.start("Not.Plain");
    expect(flow.snapshot.error).toMatchObject({ kind: "name", message: "a practice name is 1 to 32 of a-z, 0-9, - and _" });
    expect(flow.snapshot.steps.name.status).toBe("failed");
    expect(chain.attempts).toHaveLength(0);
  });

  it("says when this hour's practice accounts are vended (nothing to close)", async () => {
    const { chain, flow } = setup();
    chain.perHour = 0;
    await flow.start("judge-late");
    const s = flow.snapshot;
    expect(s.error?.kind).toBe("capacity");
    expect(s.error?.closing).toBeUndefined();
    expect(s.steps.vend.status).toBe("failed");
    expect(methods(chain)).toEqual(["vend"]);
  });

  it("polls get_run when there is no stream, and reads an answer that arrives in the reveal's own outcome", async () => {
    const { chain, flow } = setup({ stream: false });
    chain.answerInReveal = true;
    const streams: string[] = [];
    flow.subscribe((s) => streams.push(s.stream));
    await flow.start("judge-poll");
    await flow.settled();
    const s = flow.snapshot;
    expect(s.error).toBeUndefined();
    expect(streams).toContain("polling");
    expect(chain.streamUrls).toEqual([]);
    expect(s.state).toBe("Done:Paid");
    expect(s.steps.verdict).toMatchObject({ status: "done", tx: chain.sent[2].hash, exact: true });
    // T1 seen by polling: this tab's clock, labelled as such.
    expect(s.steps.t1).toMatchObject({ status: "done", exact: false });
    expect(chain.views).toContain(`${ANTIBODY}.get_run`);
  });

  it("stops following on request: no settlement from the tab, and the account closes", async () => {
    const { chain, flow } = setup();
    chain.t1 = false;
    // Before the challenge window ends (the T0 path settles at the answer + 65 s).
    chain.timers.push({ at: chain.t + 40_000, fn: () => flow.stopFollowing() });
    await flow.start("judge-stop");
    const s = flow.snapshot;
    expect(s.error).toBeUndefined();
    expect(s.stopped).toBe(true);
    expect(s.steps.t1.status).toBe("skipped");
    expect([s.steps.allocate.status, s.steps.pay.status, s.steps.close.status, s.steps.reclaim.status]).toEqual(["skipped", "skipped", "done", "done"]);
    expect(methods(chain)).toEqual(["vend", "commit", "reveal", "close", "reclaim"]);
    expect(s.state).toBe("Verifying");
  });

  it("resumes after a reload from what the tab kept, without a second vend, commit or reveal", async () => {
    const chain = new FakeChain();
    let hang = false;
    const a = setup({
      chain,
      stream: false, // a closed tab hears nothing
      sleep: async (ms) => {
        if (hang) await new Promise<void>(() => undefined); // the tab is gone
        chain.advance(ms);
      },
    });
    a.flow.subscribe((s) => {
      if (s.runId !== undefined && s.steps.reveal.status === "done") hang = true;
    });
    void a.flow.start("judge-reload");
    for (let i = 0; i < 200 && !hang; i++) await new Promise((r) => setTimeout(r, 0));
    expect(hang).toBe(true);
    const saved = a.storage.saves.at(-1) as SavedTry;
    expect(saved).toMatchObject({ commitId: 24, runId: 58, vault: 4, programId: 6 });
    chain.advance(40_000); // the verdict and T1 arrive while the tab is away
    const sentBefore = chain.sent.length;

    const b = setup({ chain });
    await b.flow.resume(saved);
    const s = b.flow.snapshot;
    expect(s.error).toBeUndefined();
    expect(s.resumed).toBe(true);
    expect(s.steps.commit.detail).toBe("commit 24 (before the reload)");
    expect([s.state, s.tier, s.closed]).toEqual(["Done:Paid", "T1", true]);
    expect(chain.sent.slice(sentBefore).map((t) => t.method)).toEqual(["allocate", "pay", "close", "reclaim"]);
  });

  it("closes a saved try's account (close, then reclaim) with the key the tab kept", async () => {
    const chain = new FakeChain();
    const { rng } = counterRng();
    const seed = rng(32);
    const key = keyFromSeed(seed);
    const account = `judge-left.${VENDING}`;
    chain.vended.set(account, { vault: 7, done: false, at: chain.nowS });
    chain.keys.set(account, new Map([[publicKeyString(key.publicKey), BigInt(5)]]));
    const { flow } = setup({ chain });
    await flow.closeSaved({ v: 1, name: "judge-left", account, vending: VENDING, seedHex: toHex(seed), startedAt: chain.t, vault: 7 });
    const s = flow.snapshot;
    expect(s.phase).toBe("done");
    expect([s.steps.close.status, s.steps.reclaim.status, s.closed]).toEqual(["done", "done", true]);
    expect(methods(chain)).toEqual(["close", "reclaim"]);
    expect(chain.keys.has(account)).toBe(false);
    // Closing again finds nothing to do.
    await flow.closeSaved({ v: 1, name: "judge-left", account, vending: VENDING, seedHex: toHex(seed), startedAt: chain.t, vault: 7 });
    expect(flow.snapshot.steps.close).toMatchObject({ status: "skipped", detail: `${account} is already deleted` });
  });
});
