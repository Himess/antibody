/**
 * The browser's practice try: crates/cli/src/practice.rs `try_it`, step by step, as a UI-independent
 * async state machine. Every dependency is injected (the NEAR RPCs, the archival RPC, the API's event
 * stream, the clock, the random bytes, the tab's storage), so the tests drive it with mocked answers.
 *
 * The transactions are the CLI's, in the CLI's order, with the CLI's arguments and gas:
 *   vend (the public vend key) → commit → reveal → allocate → pay → close (the practice key, through
 *   its own account) → reclaim (the public vend key).
 * Nonces and block hashes follow near.rs `send`: the nonce from the access key (optimistic), the block
 * hash from a final block, and on `InvalidNonce` or `Expired` a new attempt (the public vend key is shared
 * by every visitor). Each outcome is checked for failures (cmd.rs `ensure_ok`). Once the practice account
 * may exist, any failure still closes it, as `try_it` does.
 *
 * The key never leaves the tab: it is kept in memory and in the tab's storage (so a reload can resume
 * or at least close the account), and only signatures are sent.
 */
import { sha256 } from "@noble/hashes/sha2.js";
import type { AccessKeyView, AccountView, BlockView, Finality, TxOutcome, ViewResult, WaitUntil } from "../near/rpc";
import { outcomeFailures, returnValue, RpcError, txHash } from "../near/rpc";
import { parseBase, vaultAddress, type BaseWitness, type Manifest } from "./base";
import { fromHex, fromHexN, fromUtf8, toHex } from "./bytes";
import { buildPracticeEnvelope, tutorialBundle } from "./envelope";
import { contentFromTx, parseLocation } from "./fastfs";
import {
  ALLOCATE_TGAS,
  borshString,
  checkName,
  CLOSE_TGAS,
  encodeTx,
  type FunctionCall,
  jsonArgs,
  keyFromSeed,
  type KeyPair,
  PAY_TGAS,
  parseVaultOf,
  publicKeyString,
  publicVendSeed,
  RECLAIM_TGAS,
  signTx,
  TGAS,
  VEND_TGAS,
  vendArgs,
} from "./near-tx";
import { commitment, cryptoRng, practiceContentKey, type Rng, sealPractice } from "./seal";
import { parseVendingConfigFull } from "./vending";

// ------------------------------------------------------------------------------------------ constants

/** deployments/near-testnet.json `practice.vending`: the CLI's default vending contract. */
export const VENDING = "practice.antibody.testnet";
/** cmd.rs `commit`: near-antibody burns about 11 Tgas on a commit. */
export const COMMIT_TGAS = BigInt(25);
/** cmd.rs `RUN_TGAS`: OutLayer's 100 and the answer's callback 60 ride on the reveal. */
export const RUN_TGAS = BigInt(200);
/** cmd.rs `MAX_INLINE_BLOB`: a proof sent through a practice account travels inline. */
export const MAX_INLINE_BLOB = 15 * 1024;

/** The waits, as the CLI's (cmd.rs `wait_verdict`, practice.rs `wait_t1` and `settle`). */
export interface Timing {
  /** cmd.rs `wait_verdict(…, 240)`. */
  verdictSecs: number;
  /**
   * The longest wait for T1 when the answer's time is unknown (practice.rs `wait_t1(…, 300)`). With it known,
   * the try waits for T1 only until a T0 payout is allowed: the answer, the challenge window and 5 s.
   */
  t1Secs: number;
  /** practice.rs `settle`: views read a moment behind the allocation just executed. */
  settleSecs: number;
  /** The CLI polls every 2 s; the browser a little slower (public RPCs rate-limit). */
  pollMs: number;
  /** With the event stream live, polling is only a safety net. */
  pollLiveMs: number;
  /** How long to keep following a queue an earlier commit holds before closing the account. */
  heldMaxSecs: number;
}

export const TIMING: Timing = { verdictSecs: 240, t1Secs: 300, settleSecs: 20, pollMs: 2500, pollLiveMs: 10_000, heldMaxSecs: 900 };

// ------------------------------------------------------------------------------------------ types

/** The NEAR RPC calls a try makes: web/lib/near/rpc.ts `NearRpc` has them all. */
export interface RpcLike {
  viewFunction(contract: string, method: string, args?: object | Uint8Array, finality?: Finality): Promise<ViewResult>;
  viewAccessKey(account: string, publicKey: string, finality?: Finality): Promise<AccessKeyView>;
  viewAccount(account: string, finality?: Finality): Promise<AccountView | null>;
  block(finality?: Finality): Promise<BlockView>;
  sendTx(signedBase64: string, waitUntil?: WaitUntil): Promise<TxOutcome>;
  txStatus(hash: string, sender: string, waitUntil?: WaitUntil): Promise<TxOutcome>;
  call<T = unknown>(method: string, params: unknown): Promise<T>;
}

/** One event of the API's stream (`event: antibody`, the indexed event's JSON). */
export interface StreamEvent {
  seq?: number;
  standard?: string;
  event?: string;
  data?: unknown;
  tx?: string;
  block_ts_ns?: number;
  signer?: string;
}

export interface StreamHandlers {
  open(): void;
  event(e: StreamEvent): void;
  /** `closed`: the stream gave up (the browser's EventSource retries otherwise). */
  error(closed: boolean): void;
}

/** Opens the event stream at `url`; returns its close function. */
export type OpenStream = (url: string, on: StreamHandlers) => () => void;

/** What the tab keeps so a reload can resume the try, or at least close the account. */
export interface SavedTry {
  v: 1;
  name: string;
  account: string;
  vending: string;
  /** The throwaway key's 32-byte seed, hex. It stays in this tab. */
  seedHex: string;
  startedAt: number;
  programId?: number;
  /** The vend was sent: from here the account may exist, and a reload must close it. */
  vendSent?: boolean;
  vault?: number;
  vendTx?: string;
  vendAt?: number;
  blobHex?: string;
  saltHex?: string;
  commitId?: number;
  commitTx?: string;
  commitAt?: number;
  commitExact?: boolean;
  runId?: number;
  revealTx?: string;
  closed?: boolean;
  finalState?: string;
}

export interface TryStorage {
  load(): SavedTry | null;
  save(t: SavedTry): void;
  clear(): void;
}

export interface FlowDeps {
  rpc: RpcLike;
  /** FastFS uploads (the base witness) live in old transactions. */
  archival: RpcLike;
  /** The Antibody API (the event stream). */
  apiBase: string;
  /** The stream's first event to replay (inclusive, as the API reads `after`). */
  streamAfter?: number;
  /** null or absent: no stream, the flow polls `get_run`. */
  openStream?: OpenStream | null;
  now(): number;
  sleep(ms: number): Promise<void>;
  random?: Rng;
  storage?: TryStorage | null;
  vending?: string;
  /** An https location (the browser reads `near-tx:` locations itself). */
  fetchUrl?: (url: string) => Promise<Uint8Array>;
  timing?: Partial<Timing>;
}

export type StepId = "name" | "key" | "program" | "vend" | "base" | "seal" | "commit" | "reveal" | "verdict" | "t1" | "allocate" | "pay" | "close" | "reclaim";

export const STEP_IDS: StepId[] = ["name", "key", "program", "base", "vend", "seal", "commit", "reveal", "verdict", "t1", "allocate", "pay", "close", "reclaim"];

export type StepStatus = "waiting" | "running" | "done" | "failed" | "skipped";

export interface StepState {
  id: StepId;
  status: StepStatus;
  /** One line: what happened, or why not. */
  detail?: string;
  /** The NEAR transaction of the step. */
  tx?: string;
  /** Who signed it. */
  signer?: string;
  /** When it happened (ms since the epoch): a block time when `exact`, else this tab's clock. */
  at?: number;
  exact?: boolean;
}

export interface VerdictKey {
  slot: number;
  tag: string;
}

export interface TryVerdict {
  kind: string | null;
  keys: VerdictKey[];
  tier: string | null;
  receipt?: string | null;
  reason?: string | null;
  /** The answer's block time, in seconds. */
  answeredAt?: number;
}

export interface SealedInfo {
  blobHex: string;
  sha256: string;
  commitment: string;
  bytes: number;
  envelopeHex: string;
  envelopeSha256: string;
  saltHex: string;
  contentKeyHex: string;
}

/** An earlier commit that holds the program's queue (claims are paid in commit order). */
export interface HeldQueue {
  commitId: number;
  state: string;
  committedAt?: number;
  /** Seconds: when its reveal window ends and the keeper may expire it. */
  expiresAt?: number;
  message: string;
}

export type FailKind = "name" | "capacity" | "faucet" | "rpc" | "program" | "chain" | "verdict" | "other";

export type FlowPhase = "idle" | "running" | "done" | "failed";

export interface TrySnapshot {
  phase: FlowPhase;
  name: string;
  account: string;
  vending: string;
  antibody?: string;
  publicKey?: string;
  programId?: number;
  programKey?: string;
  round?: number;
  perHour?: number;
  challengeSecs?: number;
  revealSecs?: number;
  vault?: number;
  vaultAddress?: string;
  base?: { location: string; sha256: string; bytes: number; anchorBlock: string; chainId: string };
  sealed?: SealedInfo;
  commitId?: number;
  runId?: number;
  verdict?: TryVerdict;
  tier?: string;
  /** The T0 path, said as the CLI says it. */
  t0Note?: string;
  /** When the T0 path pays (ms): the answer, the challenge window and 5 s. */
  t0PayAfter?: number;
  held?: HeldQueue;
  state?: string;
  paidRaw?: string;
  beneficiary?: string;
  closed?: boolean;
  /** The event stream: live, connecting, or the flow polls `get_run`. */
  stream: "off" | "connecting" | "live" | "polling";
  steps: Record<StepId, StepState>;
  log: { at: number; level: "info" | "warn" | "error"; text: string }[];
  /** The latest sentence, for a polite live region. */
  announce: string;
  error?: { kind: FailKind; message: string; closing?: string };
  /** Transactions signed in this tab. */
  signed: number;
  startedAt?: number;
  finishedAt?: number;
  resumed?: boolean;
  /** Following stopped at the visitor's request. */
  stopped?: boolean;
}

export class FlowError extends Error {
  readonly kind: FailKind;
  readonly tx?: string;

  constructor(kind: FailKind, message: string, tx?: string) {
    super(message);
    this.name = "FlowError";
    this.kind = kind;
    this.tx = tx;
  }
}

// ------------------------------------------------------------------------------------------ views

interface ProgramView {
  id?: number;
  key?: string;
  round?: number;
  phase?: unknown;
  sealing_pk?: string | null;
  manifest_sha256?: string;
  created_at?: number;
  cursor?: number;
  commits?: number;
  params?: { bond?: string; windows?: { challenge?: number; reveal?: number } };
}

interface LocationView {
  location?: string;
  sha256?: string;
}

interface MetaView {
  base?: LocationView | null;
  round_bases?: (LocationView | null)[];
  manifest_url?: string;
  terms_sha256?: string | null;
}

export interface CommitView {
  id?: number;
  index?: number;
  hunter?: string;
  beneficiary?: string;
  committed_at?: number;
  run?: number | null;
  state?: unknown;
}

function rec(v: unknown): Record<string, unknown> | undefined {
  return typeof v === "object" && v !== null && !Array.isArray(v) ? (v as Record<string, unknown>) : undefined;
}

const num = (v: unknown): number | undefined => (typeof v === "number" && Number.isFinite(v) ? v : undefined);
const str = (v: unknown): string | undefined => (typeof v === "string" ? v : undefined);

/** cmd.rs `state_label`: `Verifying`, `Holder`, `Done:Paid`, … */
export function commitStateLabel(s: unknown): string {
  if (typeof s === "string") return s;
  const o = rec(s);
  if (!o) return "";
  const first = Object.entries(o)[0];
  if (!first) return "";
  const [k, inner] = first;
  if (k === "Done" && rec(inner)) return `Done:${Object.keys(rec(inner) ?? {})[0] ?? ""}`;
  if (k === "Done" && typeof inner === "string") return `Done:${inner}`;
  return k;
}

/** cmd.rs `paid_of`: the raw amount a `Done:Paid` commit was paid ("0" otherwise). */
export function paidOf(c: CommitView | null | undefined): string {
  const a = rec(rec(rec(c?.state)?.Done)?.Paid)?.amount;
  return typeof a === "string" ? a : typeof a === "number" ? String(a) : "0";
}

function keysOf(v: unknown): VerdictKey[] {
  if (!Array.isArray(v)) return [];
  return v.flatMap((k) => {
    const o = rec(k);
    const slot = num(o?.slot);
    const tag = str(o?.tag);
    return slot !== undefined && tag !== undefined ? [{ slot, tag }] : [];
  });
}

/** A run's state, as cmd.rs `run_view` reads it: answered (kind, keys, tier), failed, or pending. */
export interface RunState {
  pending?: boolean;
  failed?: string;
  verdict?: TryVerdict;
}

export function runState(raw: unknown): RunState {
  const o = rec(raw)?.outcome;
  const answered = rec(rec(o)?.Answered);
  if (answered) {
    const f = rec(answered.facts) ?? {};
    return {
      verdict: {
        kind: str(f.kind) ?? null,
        keys: keysOf(f.keys),
        tier: str(answered.tier) ?? null,
        receipt: str(f.receipt_sha256) ?? null,
        reason: str(f.reason) ?? null,
        answeredAt: num(answered.at),
      },
    };
  }
  const failed = rec(rec(o)?.Failed);
  if (failed) return { failed: typeof failed.reason === "string" ? failed.reason : "failed" };
  return { pending: true };
}

/** cmd.rs `pinned_base_at`: the latest round base up to `round`, else the activation's base. */
export function pinnedBaseLocation(meta: MetaView, round: number): { location: string; sha256: string } {
  const rounds = Array.isArray(meta.round_bases) ? meta.round_bases : [];
  const loc = rounds.slice(0, round).reverse().find((b) => b !== null && b !== undefined) ?? meta.base ?? null;
  if (!loc || !loc.location) throw new FlowError("program", "the program has no pinned base witness");
  return { location: loc.location, sha256: (loc.sha256 ?? "").replace(/^0x/, "").toLowerCase() };
}

/** A practice name to suggest: `judge-` and four random characters. */
export function suggestName(random: Rng = cryptoRng): string {
  const abc = "abcdefghijklmnopqrstuvwxyz0123456789";
  return "judge-" + Array.from(random(4), (b) => abc[b % abc.length]).join("");
}

/** A transaction failure's words: the contract's panic message when there is one. */
export function readableFailure(f: string): string {
  const m = /"ExecutionError":"((?:[^"\\]|\\.)*)"/.exec(f);
  if (m) {
    try {
      return (JSON.parse(`"${m[1]}"`) as string).replace(/^Smart contract panicked: /, "");
    } catch {
      return m[1];
    }
  }
  return f.length > 240 ? `${f.slice(0, 240)}…` : f;
}

const errText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/** An RPC that did not answer (network, timeout, rate limit), as opposed to one that refused. */
function unavailable(e: unknown): boolean {
  return e instanceof RpcError && /network error|timed out|HTTP (408|429|5\d\d)|Too Many Requests|-429|Rate limit|TIMEOUT_ERROR/i.test(e.message);
}

function classify(e: unknown): FlowError {
  // near-antibody refuses a run or storage the vending contract's credit cannot pay: the faucet, not the visitor.
  if (e instanceof FlowError) return /not enough NEAR credit/.test(e.message) ? new FlowError("faucet", e.message, e.tx) : e;
  const m = errText(e);
  if (/NotEnoughAllowance|NotEnoughBalance|LackBalanceForState|BalanceExceeded|not enough NEAR credit/.test(m)) {
    return new FlowError("faucet", `the practice faucet cannot pay for this (${readableFailure(m)})`);
  }
  if (unavailable(e)) return new FlowError("rpc", `the public NEAR RPCs did not answer (${m}); they rate-limit, so try again in a minute`);
  return new FlowError("other", m);
}

/** The vend's own refusals: the hourly limit, a name vended a moment ago, an empty faucet. */
function vendFailure(e: unknown): FlowError {
  const m = errText(e);
  if (m.includes("the hour's practice accounts are vended")) {
    return new FlowError("capacity", "this hour's practice accounts are all vended: the vending contract allows a few per clock hour", e instanceof FlowError ? e.tx : undefined);
  }
  if (m.includes("that practice account exists")) return new FlowError("name", "that name was vended a moment ago: pick another");
  return classify(e);
}

/** The receipt that logged the NEP-297 event `name`: its data and its block. */
function eventIn(out: TxOutcome, name: string): { data: Record<string, unknown>; blockHash?: string } | undefined {
  const receipts = Array.isArray(out.receipts_outcome) ? out.receipts_outcome : [];
  for (const r of receipts) {
    const logs = rec(rec(r)?.outcome)?.logs;
    for (const l of Array.isArray(logs) ? logs : []) {
      if (typeof l !== "string" || !l.startsWith("EVENT_JSON:")) continue;
      let j: unknown;
      try {
        j = JSON.parse(l.slice("EVENT_JSON:".length));
      } catch {
        continue;
      }
      if (rec(j)?.event !== name) continue;
      const d = rec(j)?.data;
      return { data: rec(Array.isArray(d) ? d[0] : d) ?? {}, blockHash: str(rec(r)?.block_hash) };
    }
  }
  return undefined;
}

const txBlockHash = (out: TxOutcome): string | undefined => str(rec(out.transaction_outcome)?.block_hash);

function draw(rng: Rng, n: number): Uint8Array {
  const b = rng(n);
  if (b.length !== n) throw new Error(`rng returned ${b.length} bytes, not ${n}`);
  return b;
}

function parseManifestJson(bytes: Uint8Array): Manifest {
  const m: unknown = JSON.parse(fromUtf8(bytes));
  const targets = rec(m)?.targets;
  if (!Array.isArray(targets) || !targets.every((t) => typeof rec(t)?.name === "string" && typeof rec(t)?.address === "string")) {
    throw new FlowError("program", "the program's manifest has no targets");
  }
  return m as Manifest;
}

function freshSteps(): Record<StepId, StepState> {
  return Object.fromEntries(STEP_IDS.map((id) => [id, { id, status: "waiting" as StepStatus }])) as Record<StepId, StepState>;
}

// ------------------------------------------------------------------------------------------ the flow

interface Seen {
  answer?: { verdict: TryVerdict; tx?: string; at?: number };
  t1?: { tx?: string; at?: number };
  failed?: string;
  commitAt?: number;
  /** `holder` or `paid` for this try's commit, from the stream. */
  settled?: string;
  expired?: number;
}

export class TryFlow {
  private s: TrySnapshot;
  private readonly d: FlowDeps;
  private readonly t: Timing;
  private readonly random: Rng;
  private readonly vending: string;
  private readonly listeners = new Set<(s: TrySnapshot) => void>();
  private readonly vendKey: KeyPair = keyFromSeed(publicVendSeed());
  private readonly nonces = new Map<string, bigint>();
  private readonly blockTimes = new Map<string, number>();
  private readonly pending: Promise<void>[] = [];
  private key?: KeyPair;
  private program?: ProgramView;
  private meta?: MetaView;
  private base?: BaseWitness;
  private manifest?: Manifest;
  private blob?: Uint8Array;
  private saved?: SavedTry;
  private seen: Seen = {};
  private wake?: () => void;
  private closeStream?: () => void;
  private running = false;
  private stopRequested = false;
  /** The vend was sent: from here a failure still closes the account. */
  private accountMayExist = false;

  constructor(deps: FlowDeps) {
    this.d = deps;
    this.t = { ...TIMING, ...deps.timing };
    this.random = deps.random ?? cryptoRng;
    this.vending = deps.vending ?? VENDING;
    this.s = this.blank("");
  }

  get snapshot(): TrySnapshot {
    return this.s;
  }

  subscribe(fn: (s: TrySnapshot) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Stop waiting (for T1, or for a held queue): the flow settles what it can and closes the account. */
  stopFollowing(): void {
    if (!this.running || this.stopRequested) return;
    this.stopRequested = true;
    this.s = { ...this.s, stopped: true };
    this.note("info", "You stopped following: the account closes now; the keeper allocates and pays the claim when it is final.");
    this.wake?.();
  }

  /** A new try for `name`. */
  async start(name: string): Promise<TrySnapshot> {
    this.claim();
    this.s = { ...this.blank(name), phase: "running", startedAt: this.d.now() };
    this.emit();
    await this.run(false);
    return this.s;
  }

  /** Continues a try this tab saved before a reload (each step checks the chain before acting). */
  async resume(saved: SavedTry): Promise<TrySnapshot> {
    this.claim();
    this.restore(saved);
    this.note("info", `Resumed ${saved.account} after a reload: each step reads the chain before it acts.`);
    await this.run(true);
    return this.s;
  }

  /** Closes the account of a saved try (close, then reclaim): the least a reload owes the faucet. */
  async closeSaved(saved: SavedTry): Promise<TrySnapshot> {
    this.claim();
    this.restore(saved);
    this.accountMayExist = true;
    try {
      await this.stepClose();
      await this.finalRead();
      this.set({ phase: "done" });
    } catch (e) {
      const f = classify(e);
      this.set({ phase: "failed", error: { kind: f.kind, message: f.message } });
    } finally {
      this.finish();
    }
    return this.s;
  }

  // ---------------------------------------------------------------------------------- the pipeline

  private claim(): void {
    if (this.running) throw new Error("a try is already running in this tab");
    this.running = true;
    this.stopRequested = false;
    this.seen = {};
    this.accountMayExist = false;
    this.key = undefined;
    this.program = undefined;
    this.meta = undefined;
    this.base = undefined;
    this.manifest = undefined;
    this.blob = undefined;
    this.saved = undefined;
  }

  private blank(name: string): TrySnapshot {
    return {
      phase: "idle",
      name,
      account: name ? `${name}.${this.vending}` : "",
      vending: this.vending,
      stream: "off",
      steps: freshSteps(),
      log: [],
      announce: "",
      signed: 0,
    };
  }

  private restore(saved: SavedTry): void {
    this.saved = saved;
    this.key = keyFromSeed(fromHexN(saved.seedHex, 32, "saved key"));
    if (saved.blobHex) this.blob = fromHex(saved.blobHex);
    const s = this.blank(saved.name);
    s.phase = "running";
    s.resumed = true;
    s.startedAt = saved.startedAt;
    s.publicKey = publicKeyString(this.key.publicKey);
    s.programId = saved.programId;
    s.vault = saved.vault;
    s.commitId = saved.commitId;
    s.runId = saved.runId;
    const before = "before the reload";
    s.steps.name = { id: "name", status: "done", detail: `${saved.account} (${before})` };
    s.steps.key = { id: "key", status: "done", detail: `kept in this tab: ${s.publicKey}` };
    if (saved.vault !== undefined) s.steps.vend = { id: "vend", status: "done", detail: `vault ${saved.vault} (${before})`, tx: saved.vendTx, signer: this.vending, at: saved.vendAt };
    if (saved.commitId !== undefined) {
      s.steps.commit = { id: "commit", status: "done", detail: `commit ${saved.commitId} (${before})`, tx: saved.commitTx, signer: saved.account, at: saved.commitAt, exact: saved.commitExact };
    }
    if (saved.runId !== undefined) s.steps.reveal = { id: "reveal", status: "done", detail: `run ${saved.runId} (${before})`, tx: saved.revealTx, signer: saved.account };
    this.s = s;
    this.accountMayExist = saved.vault !== undefined;
    this.emit();
  }

  private async run(resuming: boolean): Promise<void> {
    try {
      if (!resuming) {
        await this.stepName();
        this.stepKey();
      }
      await this.stepProgram();
      // The base witness is the program's, not the account's: read and checked before the vend, so an
      // archival RPC that cannot answer stops the try before it costs the faucet anything.
      if (this.s.commitId === undefined && !this.blob) await this.stepBase();
      else this.skip("base", this.s.commitId !== undefined ? "not needed: the commit was made before the reload" : "not needed: the sealed proof was kept before the reload");
      await this.stepVend(resuming);
      if (this.s.commitId === undefined) {
        if (!this.blob) this.stepSeal();
        else this.sealedFromBlob();
        await this.stepCommit(resuming);
      } else {
        this.skip("seal", "not needed: the commit was made before the reload");
      }
      this.openStream();
      if (this.s.runId === undefined) await this.stepReveal(resuming);
      await this.stepVerdict();
      await this.stepT1();
      if (this.stopRequested && this.s.steps.verdict.status === "running") this.skip("verdict", "you stopped following before the verdict arrived");
      if (this.stopRequested && this.s.state === undefined) {
        this.skip("allocate", "you stopped following: the keeper allocates the claim once it is final");
        this.skip("pay", "you stopped following: the keeper pays it");
      } else {
        await this.stepSettle();
      }
      await this.stepClose();
      await this.finalRead();
      this.set({ phase: "done", announce: this.s.state === "Done:Paid" ? "Done: paid." : `The try is over: ${this.s.state ?? "state unknown"}.` });
    } catch (e) {
      const f = classify(e);
      const running = STEP_IDS.filter((id) => this.s.steps[id].status === "running");
      if (running[0]) this.patch(running[0], { status: "failed", detail: f.message, tx: f.tx ?? this.s.steps[running[0]].tx });
      for (const id of running.slice(1)) this.patch(id, { status: "waiting", detail: undefined });
      let closing: string | undefined;
      if (this.accountMayExist && !this.s.closed) closing = await this.closeAfterFailure();
      this.set({ phase: "failed", error: { kind: f.kind, message: f.message, closing }, announce: `The try stopped: ${f.message}` });
      this.note("error", closing ? `${f.message} · ${closing}` : f.message);
    } finally {
      this.finish();
    }
  }

  private finish(): void {
    this.closeStream?.();
    this.closeStream = undefined;
    if (this.s.stream !== "off") this.s = { ...this.s, stream: "off" };
    this.running = false;
    this.set({ finishedAt: this.d.now() });
    // Block times still on their way update the steps when they arrive.
    void Promise.allSettled(this.pending).then(() => this.emit());
  }

  /** For tests and the page: every block-time lookup this flow started has settled. */
  async settled(): Promise<void> {
    await Promise.allSettled(this.pending);
  }

  // ---------------------------------------------------------------------------------- the steps

  /** practice.rs `vend`: the name is plain and new (no account, and no vault assigned to it before). */
  private async stepName(): Promise<void> {
    const { name, account } = this.s;
    this.begin("name", `Checking ${account}`);
    try {
      checkName(name);
    } catch (e) {
      throw new FlowError("name", errText(e));
    }
    const [acc, vault] = await Promise.all([this.d.rpc.viewAccount(account), this.vaultOf(account)]);
    // A closed practice account is gone, but the vending contract keeps its name.
    if (acc !== null || vault !== null) throw new FlowError("name", `${account} is taken: pick another name (a new one, lowercase)`);
    this.done("name", `${account} is free`);
  }

  /** near.rs `Signer::generate`: a throwaway ed25519 key from the platform CSPRNG, kept in this tab. */
  private stepKey(): void {
    const seed = draw(this.random, 32);
    this.key = keyFromSeed(seed);
    const publicKey = publicKeyString(this.key.publicKey);
    this.saved = { v: 1, name: this.s.name, account: this.s.account, vending: this.vending, seedHex: toHex(seed), startedAt: this.s.startedAt ?? this.d.now() };
    this.persist({});
    this.s = { ...this.s, publicKey };
    this.done("key", `ed25519, made in this tab and never sent: ${publicKey}`);
  }

  /** practice.rs `target` / `vending_program`: the program from the vending contract, never a constant. */
  private async stepProgram(): Promise<void> {
    this.begin("program", `Reading the practice program from ${this.vending}`);
    const cfg = parseVendingConfigFull((await this.d.rpc.viewFunction(this.vending, "get_config", new Uint8Array(0))).bytes);
    // A resumed try stays with the program it committed to (the vending contract may be pointed elsewhere since).
    const programId = this.saved?.programId ?? cfg.programId;
    const antibody = cfg.antibody;
    const p = (await this.d.rpc.viewFunction(antibody, "get_program", { program_id: programId })).json<ProgramView | null>();
    if (!p) throw new FlowError("program", `program ${programId} does not exist on ${antibody}`);
    if (!p.sealing_pk) throw new FlowError("program", `program ${programId} is not activated yet: it has no sealing key`);
    const phase = typeof p.phase === "string" ? p.phase : Object.keys(rec(p.phase) ?? {})[0];
    if (phase && phase !== "Open" && this.s.commitId === undefined) throw new FlowError("program", `program ${programId} is ${phase}, not open for commits`);
    const m = (await this.d.rpc.viewFunction(antibody, "get_program_meta", { program_id: programId })).json<MetaView | null>();
    if (!m) throw new FlowError("program", `program ${programId} does not exist on ${antibody}`);
    this.program = p;
    this.meta = m;
    this.persist({ programId });
    this.set({
      antibody,
      programId,
      programKey: p.key ?? `${antibody}/${programId}`,
      round: p.round ?? 0,
      perHour: cfg.terms?.perHour,
      challengeSecs: p.params?.windows?.challenge,
      revealSecs: p.params?.windows?.reveal,
    });
    this.done("program", `program ${programId} (${this.s.programKey}), round ${this.s.round}, from ${this.vending}'s get_config`);
  }

  /** practice.rs `vend`: the public vend key calls `vend(name, public_key)`; the contract assigns a vault. */
  private async stepVend(resuming: boolean): Promise<void> {
    if (this.s.vault !== undefined) return;
    const { name, account } = this.s;
    const key = this.requireKey();
    this.begin("vend", `The public vend key asks ${this.vending} for ${account}`);
    if (resuming) {
      // A vend sent before the reload: the account and its vault are on chain.
      if ((await this.d.rpc.viewAccount(account)) !== null) {
        const vault = await this.vaultOf(account);
        if (vault !== null) {
          this.accountMayExist = true;
          this.persist({ vault });
          this.set({ vault, vaultAddress: this.vaultAddressOf(vault) });
          this.done("vend", `${account}, vault ${vault} (vended before the reload)`);
          return;
        }
      }
    }
    this.accountMayExist = true;
    this.persist({ vendSent: true });
    const call: FunctionCall = { method: "vend", args: vendArgs(name, key.publicKey), gas: VEND_TGAS * TGAS, deposit: BigInt(0) };
    let out: TxOutcome;
    try {
      out = await this.send(this.vendKey, this.vending, this.vending, call);
      this.ensureOk(out, "vend");
    } catch (e) {
      throw vendFailure(e);
    }
    // A load-balanced RPC may answer from a node a block behind the vend: ask again briefly.
    const vault = await this.reread(() => this.vaultOf(account), (v) => v !== null);
    if (vault === null) throw new FlowError("chain", `${this.vending} assigned no vault to ${account}`, txHash(out));
    const tx = txHash(out);
    const address = this.vaultAddressOf(vault);
    this.persist({ vault, vendTx: tx, vendAt: this.d.now() });
    this.set({ vault, vaultAddress: address });
    this.done("vend", `${account}, vault ${vault}${address ? ` (${address})` : ""}: its key may call only its own account`, { tx, signer: `${this.vending} (public vend key)` });
    this.timeStep("vend", out, undefined, (at) => this.persist({ vendAt: at }));
  }

  /** cmd.rs `pinned_base_at`: the program's pinned base witness, from NEAR, checked against its hash. */
  private async stepBase(): Promise<void> {
    const { meta, program } = this.requireProgram();
    this.begin("base", "Fetching the program's pinned base witness from NEAR (archival RPC)");
    const loc = pinnedBaseLocation(meta, this.s.round ?? 0);
    const bytes = await this.fetchLocation(loc.location);
    let base: BaseWitness;
    try {
      base = parseBase(bytes, loc.sha256);
    } catch (e) {
      throw new FlowError("program", errText(e));
    }
    // practice.rs `vault_address` reads target `vault-<i>` of the program's manifest (round 0's, whose hash
    // the program commits). The base carries it; another manifest is fetched and checked the same way.
    const want = (program.manifest_sha256 ?? "").replace(/^0x/, "").toLowerCase();
    this.manifest = toHex(base.manifestSha256) === want ? base.manifest : await this.roundZeroManifest(want);
    this.base = base;
    this.set({
      base: { location: loc.location, sha256: toHex(base.sha256), bytes: bytes.length, anchorBlock: base.anchorBlock.toString(), chainId: base.chainId.toString() },
    });
    this.done("base", `${bytes.length.toLocaleString("en-US")} bytes, sha256 as the program pins it (anchor block ${base.anchorBlock}, chain ${base.chainId})`);
  }

  /** practice.rs `vault_address`: the address of the vault the vending contract assigned. */
  private vaultAddressOf(vault: number): string | undefined {
    if (!this.manifest) return undefined;
    try {
      return vaultAddress(this.manifest, vault);
    } catch (e) {
      throw new FlowError("program", errText(e));
    }
  }

  /** cmd.rs `commit`'s sealing: the tutorial proof under a fresh salt, sealed with the public content key. */
  private stepSeal(): void {
    const { program } = this.requireProgram();
    const base = this.base;
    const vault = this.s.vaultAddress ?? (this.s.vault !== undefined ? this.vaultAddressOf(this.s.vault) : undefined);
    if (!base || !vault) throw new FlowError("other", "no base witness to build the proof on");
    if (!this.s.vaultAddress) this.set({ vaultAddress: vault });
    this.begin("seal", "Building and sealing the tutorial proof in this tab");
    const salt = draw(this.random, 32);
    const envelope = buildPracticeEnvelope(base, vault, salt);
    const programKey = this.s.programKey ?? "";
    const sealed = sealPractice({ sealingPk: fromHexN(program.sealing_pk ?? "", 32, "sealing key"), programKey, hunter: this.s.account, envelope, rng: this.random });
    if (sealed.blob.length > MAX_INLINE_BLOB) {
      throw new FlowError("other", `a proof sent through a practice account travels inline: this one is ${sealed.blob.length} bytes (at most ${MAX_INLINE_BLOB})`);
    }
    this.blob = sealed.blob;
    const cm = commitment(programKey, this.s.round ?? 0, this.s.account, sealed.sha256);
    this.persist({ blobHex: toHex(sealed.blob), saltHex: toHex(salt) });
    this.set({
      sealed: {
        blobHex: toHex(sealed.blob),
        sha256: toHex(sealed.sha256),
        commitment: toHex(cm),
        bytes: sealed.blob.length,
        envelopeHex: toHex(envelope),
        envelopeSha256: toHex(sha256(envelope)),
        saltHex: toHex(salt),
        contentKeyHex: toHex(sealed.contentKey),
      },
    });
    this.done("seal", `ABSEAL1, ${sealed.blob.length} bytes, sealed to the program's key; commitment ${toHex(cm).slice(0, 12)}…`);
  }

  /** The sealed proof a reload kept: its hash and commitment again (the blob itself is unchanged). */
  private sealedFromBlob(): void {
    const blob = this.blob;
    if (!blob) return;
    const digest = sha256(blob);
    const cm = commitment(this.s.programKey ?? "", this.s.round ?? 0, this.s.account, digest);
    this.set({
      sealed: {
        blobHex: toHex(blob),
        sha256: toHex(digest),
        commitment: toHex(cm),
        bytes: blob.length,
        envelopeHex: "",
        envelopeSha256: "",
        saltHex: this.saved?.saltHex ?? "",
        contentKeyHex: toHex(practiceContentKey(this.s.programKey ?? "", this.s.account)),
      },
    });
    this.done("seal", `ABSEAL1, ${blob.length} bytes (sealed before the reload)`);
  }

  /** cmd.rs `commit` for an agent: through the practice account, the bond from the vending contract's credit. */
  private async stepCommit(resuming: boolean): Promise<void> {
    const { meta } = this.requireProgram();
    const key = this.requireKey();
    const sealed = this.s.sealed;
    if (!sealed) throw new FlowError("other", "nothing sealed to commit");
    const { account } = this.s;
    const antibody = this.s.antibody ?? "";
    this.begin("commit", "Committing the commitment (the proof stays sealed)");
    if (resuming) {
      const found = await this.commitOfHunter();
      if (found !== undefined) {
        this.persist({ commitId: found });
        this.set({ commitId: found });
        this.done("commit", `commit ${found} (sent before the reload)`);
        return;
      }
    }
    const me = (await this.d.rpc.viewFunction(antibody, "get_account", { account_id: account })).json<{ agent?: unknown } | null>();
    if (!me || me.agent === null || me.agent === undefined) {
      throw new FlowError("chain", `${account} is not an agent of ${this.vending}: its bond cannot come from the vending contract's credit`);
    }
    const args = jsonArgs({ program_id: this.s.programId, round: this.s.round ?? 0, commitment: sealed.commitment, terms_sha256: meta.terms_sha256 ?? null });
    const out = await this.send(key, account, account, { method: "commit", args, gas: COMMIT_TGAS * TGAS, deposit: BigInt(0) });
    this.ensureOk(out, "commit");
    const id = returnValue(out);
    if (typeof id !== "number") throw new FlowError("chain", "containment: the commit was refused (the bond is over this agent's cap or remaining budget)", txHash(out));
    const tx = txHash(out);
    this.persist({ commitId: id, commitTx: tx, commitAt: this.d.now(), commitExact: false });
    this.set({ commitId: id });
    this.done("commit", `commit ${id}: the commitment and the bond are on chain; the proof is still sealed`, { tx, signer: account });
    this.timeStep("commit", out, "commit", (at) => this.persist({ commitAt: at, commitExact: true }));
  }

  /** cmd.rs `reveal`: the sealed blob inline; near-antibody asks OutLayer to verify it (a run). */
  private async stepReveal(resuming: boolean): Promise<void> {
    const key = this.requireKey();
    const id = this.s.commitId;
    const { account } = this.s;
    if (id === undefined) throw new FlowError("other", "no commit to reveal");
    this.begin("reveal", "Revealing the sealed blob: near-antibody asks OutLayer's TEE to verify it");
    if (resuming) {
      const c = await this.getCommit();
      if (commitStateLabel(c?.state) !== "Committed" && typeof c?.run === "number") {
        this.persist({ runId: c.run });
        this.set({ runId: c.run });
        this.done("reveal", `run ${c.run} (revealed before the reload)`);
        return;
      }
    }
    const blob = this.blob;
    if (!blob) throw new FlowError("other", "the sealed proof was not kept in this tab, so it cannot be revealed");
    const args = jsonArgs({ commit_id: id, blob: { inline: toHex(blob) } });
    let settled = false;
    const sending = this.send(key, account, account, { method: "reveal", args, gas: RUN_TGAS * TGAS, deposit: BigInt(0) }).finally(() => {
      settled = true;
      this.wake?.();
    });
    sending.catch(() => undefined);
    // The RPC answers once the whole transaction tree ran, and OutLayer's answer is part of it (about 15 s).
    // near-antibody's state names the run within seconds: say so meanwhile.
    let early: number | undefined;
    while (!settled && early === undefined) {
      await this.nap(1500, true);
      if (settled) break;
      try {
        const c = await this.getCommit();
        if (typeof c?.run === "number" && commitStateLabel(c.state) !== "Committed") early = c.run;
      } catch {
        // the outcome decides
      }
    }
    if (early !== undefined) {
      this.set({ runId: early });
      this.patch("reveal", { detail: `included: run ${early} requested; the RPC answers once the TEE's verdict is in` });
      this.begin("verdict", "OutLayer's TEE is re-running the proof on the pinned base");
    }
    let out: TxOutcome;
    try {
      out = await sending;
    } catch (e) {
      if (early === undefined) {
        const c = await this.getCommit().catch(() => null);
        if (typeof c?.run === "number" && commitStateLabel(c.state) !== "Committed") early = c.run;
      }
      if (early !== undefined && unavailable(e)) {
        // The RPC stopped waiting for the tree; near-antibody has the run.
        this.persist({ runId: early });
        this.set({ runId: early });
        this.done("reveal", `run ${early} requested (the RPC stopped waiting for the whole transaction; near-antibody records the run)`, { signer: account });
        return;
      }
      throw e;
    }
    try {
      this.ensureOk(out, "reveal");
    } catch (e) {
      // Only this account reveals its commit: when near-antibody records a run for it, the reveal went
      // through (an earlier attempt of the same transaction), whatever this outcome says.
      const c = await this.getCommit().catch(() => null);
      if (typeof c?.run === "number" && commitStateLabel(c.state) !== "Committed") {
        this.note("warn", `${errText(e)}; near-antibody records run ${c.run} for commit ${id}, so the reveal went through`);
        this.persist({ runId: c.run });
        this.set({ runId: c.run });
        this.done("reveal", `run ${c.run} requested`, { signer: account });
        return;
      }
      throw e;
    }
    const requested = eventIn(out, "run_requested");
    const runId = num(requested?.data.run_id) ?? early;
    if (runId === undefined) throw new FlowError("chain", "the transaction started no run", txHash(out));
    const tx = txHash(out);
    this.persist({ runId, revealTx: tx });
    this.set({ runId });
    this.done("reveal", `run ${runId}: the sealed blob is on chain, under the try's public content key, so anyone, T2's prover included, can open it`, { tx, signer: account });
    this.timeStep("reveal", out, "run_requested");
    // The answer may be in the same outcome.
    const answered = eventIn(out, "run_answered");
    if (answered && num(answered.data.run_id) === runId) {
      this.seen.answer = {
        verdict: { kind: str(answered.data.kind) ?? null, keys: keysOf(answered.data.keys), tier: str(answered.data.tier) ?? null, receipt: str(answered.data.receipt_sha256) ?? null },
        tx,
      };
      if (answered.blockHash) {
        const p = this.blockTime(answered.blockHash)
          .then((ms) => {
            if (this.seen.answer && this.seen.answer.at === undefined) {
              this.seen.answer.at = ms;
              this.seen.answer.verdict.answeredAt ??= Math.floor(ms / 1000);
            }
            if (this.s.steps.verdict.status === "done" && !this.s.steps.verdict.exact) {
              this.patch("verdict", { at: ms, exact: true });
              if (this.s.verdict && this.s.verdict.answeredAt === undefined) this.set({ verdict: { ...this.s.verdict, answeredAt: Math.floor(ms / 1000) } });
            }
          })
          .catch(() => undefined);
        this.pending.push(p);
      }
    }
  }

  /** cmd.rs `wait_verdict` (240 s): the stream's `run_answered`, else `get_run`. */
  private async stepVerdict(): Promise<void> {
    if (this.s.steps.verdict.status === "done") return;
    if (this.s.steps.verdict.status !== "running") this.begin("verdict", "OutLayer's TEE is re-running the proof on the pinned base");
    const t0 = this.d.now();
    let lastPoll = -Infinity;
    for (;;) {
      if (this.seen.failed) throw new FlowError("verdict", `run ${this.s.runId} failed: ${this.seen.failed}`);
      if (this.seen.answer) return this.verdictIn(this.seen.answer.verdict, this.seen.answer.tx, this.seen.answer.at, true);
      if (this.d.now() - lastPoll >= this.pollEvery()) {
        lastPoll = this.d.now();
        const r = await this.pollRun();
        if (r?.failed) throw new FlowError("verdict", `run ${this.s.runId} failed: ${r.failed}`);
        if (r?.verdict) {
          const at = r.verdict.answeredAt !== undefined ? r.verdict.answeredAt * 1000 : undefined;
          return this.verdictIn(r.verdict, undefined, at, at !== undefined);
        }
      }
      if (this.d.now() - t0 > this.t.verdictSecs * 1000) {
        this.note("warn", `No verdict within ${this.t.verdictSecs} s: still verifying. The try keeps waiting for it with T1.`);
        this.patch("verdict", { detail: `still verifying after ${this.t.verdictSecs} s` });
        return;
      }
      if (this.stopRequested) return;
      await this.nap(this.pollEvery());
    }
  }

  private verdictIn(v: TryVerdict, tx: string | undefined, at: number | undefined, exact: boolean): void {
    if (v.kind === "InfraError") throw new FlowError("verdict", `run ${this.s.runId} failed: InfraError${v.reason ? ` (${v.reason})` : ""}`);
    const tier = v.tier ?? "T0";
    this.set({ verdict: v, tier });
    const slots = v.keys.map((k) => k.slot);
    const words = v.kind === "Violation" ? `Violation of slot ${slots.join(", ") || "?"}` : v.kind === "NoViolation" ? "No violation" : (v.kind ?? "answered");
    const how = tier === "T1" ? "OutLayer's answer, its TDX quote already verified on NEAR" : "OutLayer's answer (T0 until its TDX quote is verified)";
    this.done("verdict", `${words}: ${how}`, { tx, at, exact: at !== undefined && exact });
  }

  /**
   * Wait for T1, the run's TDX quote verified on NEAR, until a T0 payout is allowed: the answer, the
   * program's challenge window and 5 s. T1 first: settled at once, under the instant cap. No T1 by then: the
   * verdict stays T0 and is settled then, as the contract allows (practice.rs `try_it` takes the same T0
   * path, after waiting 300 s for T1; a visitor waits only as long as the contract makes them).
   */
  private async stepT1(): Promise<void> {
    if (this.s.tier === "T1" && this.s.steps.t1.status === "done") return;
    this.begin("t1", "Waiting for T1: the run's TDX quote, verified on NEAR");
    const t0 = this.d.now();
    const window = await this.challengeWindow();
    let lastPoll = -Infinity;
    for (;;) {
      if (this.seen.failed) throw new FlowError("verdict", `run ${this.s.runId} failed: ${this.seen.failed}`);
      if (this.seen.answer && this.s.steps.verdict.status !== "done") this.verdictIn(this.seen.answer.verdict, this.seen.answer.tx, this.seen.answer.at, true);
      if (this.seen.t1 || this.s.tier === "T1") {
        this.set({ tier: "T1", verdict: this.s.verdict ? { ...this.s.verdict, tier: "T1" } : this.s.verdict });
        this.done("t1", "T1: the TDX quote of the run, verified on NEAR (relayed by our T1 relayer)", {
          tx: this.seen.t1?.tx,
          at: this.seen.t1?.at ?? this.d.now(),
          exact: this.seen.t1?.at !== undefined,
        });
        return;
      }
      if (this.d.now() - lastPoll >= this.pollEvery()) {
        lastPoll = this.d.now();
        const r = await this.pollRun();
        if (r?.failed) throw new FlowError("verdict", `run ${this.s.runId} failed: ${r.failed}`);
        if (r?.verdict) {
          if (this.s.steps.verdict.status !== "done") {
            const at = r.verdict.answeredAt !== undefined ? r.verdict.answeredAt * 1000 : undefined;
            this.verdictIn(r.verdict, undefined, at, at !== undefined);
          }
          if (this.s.verdict && r.verdict.answeredAt !== undefined && this.s.verdict.answeredAt === undefined)
            this.set({ verdict: { ...this.s.verdict, answeredAt: r.verdict.answeredAt } });
          if (r.verdict.tier === "T1") {
            this.set({ tier: "T1" });
            continue;
          }
        }
      }
      if (this.stopRequested) {
        this.skip("t1", "you stopped following before T1");
        return;
      }
      const answered = this.s.verdict?.answeredAt;
      const payAfter = answered !== undefined ? (answered + window + 5) * 1000 : undefined;
      if (payAfter !== undefined && this.d.now() >= payAfter) return this.t0Path(window, payAfter);
      if (payAfter === undefined && this.d.now() - t0 > this.t.t1Secs * 1000) return this.t0Path(window, undefined);
      if (payAfter !== undefined && this.s.t0PayAfter !== payAfter) this.set({ t0PayAfter: payAfter });
      await this.nap(Math.min(this.pollEvery(), payAfter !== undefined ? Math.max(250, payAfter - this.d.now()) : this.pollEvery()));
    }
  }

  /** The program's challenge window, in seconds (60 on the practice program). */
  private async challengeWindow(): Promise<number> {
    let window = this.s.challengeSecs ?? 60;
    try {
      const p = (await this.d.rpc.viewFunction(this.s.antibody ?? "", "get_program", { program_id: this.s.programId })).json<ProgramView | null>();
      window = p?.params?.windows?.challenge ?? window;
    } catch {
      // the window read with the program
    }
    return window;
  }

  /** No T1 before a T0 payout is allowed: the verdict stays T0 and is paid after the challenge window. */
  private async t0Path(window: number, known: number | undefined): Promise<void> {
    let payAfter = known;
    if (payAfter === undefined) {
      const r = await this.pollRun();
      const answered = r?.verdict?.answeredAt ?? this.s.verdict?.answeredAt ?? 0;
      if (this.s.verdict && r?.verdict?.answeredAt !== undefined) this.set({ verdict: { ...this.s.verdict, answeredAt: r.verdict.answeredAt } });
      payAfter = (answered + window + 5) * 1000;
    }
    const note =
      known === undefined
        ? `no T1 in the ${this.t.t1Secs} s this try waits for it: the verdict stays T0 and is paid after the ${window} s challenge window`
        : `no T1 within the ${window} s challenge window: the verdict stays T0 and is paid after it`;
    this.set({ t0Note: note, t0PayAfter: payAfter, tier: this.s.tier ?? "T0" });
    this.note("warn", note);
    this.skip("t1", note);
    while (this.d.now() < payAfter && !this.stopRequested) await this.nap(Math.min(5000, payAfter - this.d.now()));
    const answered = this.s.verdict?.answeredAt;
    if (answered !== undefined) await this.untilChainPast(answered + window + 1);
  }

  /**
   * The challenge window ends on NEAR's clock, not this tab's: wait until a final block is past `endSecs`.
   * A tab clock ahead of the chain would otherwise send `allocate` before the claim is final, and read
   * "nothing to allocate". Without a block read, the tab's clock stands.
   */
  private async untilChainPast(endSecs: number): Promise<void> {
    for (let i = 0; i < 40 && !this.stopRequested; i++) {
      let chainMs: number;
      try {
        const ns = (await this.d.rpc.block("final")).header.timestamp_nanosec;
        chainMs = Number(BigInt(ns) / BigInt(1_000_000));
      } catch {
        return;
      }
      if (!(chainMs > 0) || chainMs >= endSecs * 1000) return;
      await this.nap(Math.min(5000, endSecs * 1000 - chainMs + 1000));
    }
  }

  /** practice.rs `settle`: `allocate` the program's claims, then `pay` this one (both permissionless). */
  private async stepSettle(): Promise<void> {
    const key = this.requireKey();
    const { account } = this.s;
    const id = this.s.commitId;
    if (id === undefined) throw new FlowError("other", "no commit to settle");
    this.begin("allocate", "allocate: the program's claims, in commit order (permissionless)");
    const allocated = await this.allocate(key);
    if (allocated.tx && allocated.ok) {
      // Views read a moment behind the allocation just executed.
      const t0 = this.d.now();
      while (commitStateLabel((await this.getCommit())?.state) === "Verifying" && this.d.now() - t0 < this.t.settleSecs * 1000) await this.nap(1000, true);
    }
    let c = await this.getCommit();
    let state = commitStateLabel(c?.state);
    if (allocated.ok) this.done("allocate", "the program's claims were allocated", { tx: allocated.tx, signer: account });
    else {
      const why = state === "Verifying" ? "nothing to allocate yet: the claim ahead of yours is not final" : "nothing to allocate: the keeper had allocated it already";
      this.skip("allocate", why, allocated.tx);
    }
    if (state === "Verifying") {
      await this.followHeld(key);
      c = await this.getCommit();
      state = commitStateLabel(c?.state);
    }
    this.set({ state });
    if (state === "Holder") {
      await this.pay(key, id);
      return;
    }
    if (state === "Done:Paid") this.skip("pay", "the keeper paid it already");
    else if (state === "Verifying") this.skip("pay", "not yet: the claim is paid once the queue ahead of it moves (the keeper runs allocate and pay)");
    else this.skip("pay", `nothing to pay: the commit is ${state || "unknown"}`);
  }

  private async allocate(key: KeyPair): Promise<{ ok: boolean; tx?: string }> {
    const { account } = this.s;
    try {
      const out = await this.send(key, account, account, { method: "allocate", args: jsonArgs({ program_id: this.s.programId, max: 20 }), gas: ALLOCATE_TGAS * TGAS, deposit: BigInt(0) });
      this.ensureOk(out, "allocate");
      this.timeStep("allocate", out, "holder");
      return { ok: true, tx: txHash(out) };
    } catch (e) {
      if (errText(e).includes("nothing to allocate")) return { ok: false, tx: e instanceof FlowError ? e.tx : undefined };
      throw e;
    }
  }

  private async pay(key: KeyPair, id: number): Promise<void> {
    const { account } = this.s;
    this.begin("pay", "pay: the claim, to the beneficiary (permissionless)");
    let out: TxOutcome;
    try {
      out = await this.send(key, account, account, { method: "pay", args: jsonArgs({ commit_id: id }), gas: PAY_TGAS * TGAS, deposit: BigInt(0) });
      this.ensureOk(out, "pay");
    } catch (e) {
      // The keeper may pay first: that is the same payout.
      const c = await this.getCommit().catch(() => null);
      if (commitStateLabel(c?.state) === "Done:Paid") {
        this.set({ state: "Done:Paid", paidRaw: paidOf(c), beneficiary: c?.beneficiary });
        this.skip("pay", "the keeper paid it a moment earlier", e instanceof FlowError ? e.tx : undefined);
        return;
      }
      throw e;
    }
    const paid = eventIn(out, "paid");
    const amount = str(paid?.data.amount);
    const beneficiary = str(paid?.data.beneficiary);
    this.set({ state: "Done:Paid", paidRaw: amount ?? this.s.paidRaw, beneficiary: beneficiary ?? this.s.beneficiary });
    this.done("pay", `Done:Paid${beneficiary ? `, to ${beneficiary}` : ""}${paid?.data.instant === true ? ", instantly (T1, under the cap)" : ""}`, { tx: txHash(out), signer: account });
    this.timeStep("pay", out, "paid");
  }

  /** Claims are paid in commit order: say which commit holds the queue and when the keeper expires it; keep following. */
  private async followHeld(key: KeyPair): Promise<void> {
    const held = await this.findHolder();
    const message = held?.message ?? "Verified; its payout follows once the program's earlier commits are settled (claims go in commit order; the keeper runs allocate and pay).";
    if (held) this.set({ held });
    this.note("warn", message);
    this.patch("allocate", { detail: message });
    const now = this.d.now();
    const expiresMs = held?.expiresAt !== undefined ? held.expiresAt * 1000 : undefined;
    const deadline = Math.min(now + this.t.heldMaxSecs * 1000, Math.max(now + 120_000, (expiresMs ?? now) + 180_000));
    let ahead = held;
    let lastAllocate = now;
    while (this.d.now() < deadline && !this.stopRequested) {
      await this.nap(Math.max(5000, this.pollEvery()));
      const state = commitStateLabel((await this.getCommit())?.state);
      if (state !== "Verifying") return;
      // Once the commit ahead moved (the keeper expired it, or it settled), allocation moves on: ask again.
      let moved = !ahead && this.d.now() - lastAllocate >= 30_000;
      if (ahead) {
        const was = ahead.commitId;
        const now2 = commitStateLabel((await this.getCommitById(was))?.state);
        moved = this.seen.expired === was || (now2 !== "" && now2 !== ahead.state);
      }
      if (moved && this.d.now() - lastAllocate >= 5000) {
        lastAllocate = this.d.now();
        try {
          const a = await this.allocate(key);
          if (a.ok) this.patch("allocate", { status: "done", detail: "the queue moved: the program's claims were allocated", tx: a.tx });
        } catch (e) {
          this.note("warn", `allocate again: ${errText(e)}`);
        }
        // Another commit may hold the queue next.
        ahead = await this.findHolder();
        if (ahead) {
          this.set({ held: ahead });
          this.note("warn", ahead.message);
        }
      }
    }
    this.note("warn", "Still verifying when following ended: the keeper allocates and pays it once the queue moves. The commit page shows when.");
  }

  /** The commit at the program's cursor, when it is not this try's: the one that holds the queue. */
  private async findHolder(): Promise<HeldQueue | undefined> {
    try {
      const antibody = this.s.antibody ?? "";
      const p = (await this.d.rpc.viewFunction(antibody, "get_program", { program_id: this.s.programId })).json<ProgramView | null>();
      const cursor = p?.cursor;
      if (cursor === undefined) return undefined;
      const list = (await this.d.rpc.viewFunction(antibody, "list_commits", { program_id: this.s.programId, from: cursor, limit: 1 })).json<CommitView[]>();
      const c = Array.isArray(list) ? list[0] : undefined;
      if (!c || c.id === undefined || c.id === this.s.commitId) return undefined;
      const state = commitStateLabel(c.state);
      const reveal = p?.params?.windows?.reveal ?? this.s.revealSecs ?? 600;
      if (state === "Committed" && c.committed_at !== undefined) {
        const expiresAt = c.committed_at + reveal;
        return {
          commitId: c.id,
          state,
          committedAt: c.committed_at,
          expiresAt,
          message: `Claims are paid in commit order. Commit ${c.id}, committed earlier and not yet revealed, holds the queue until its ${reveal} s reveal window ends; then the keeper expires it and yours is paid.`,
        };
      }
      return {
        commitId: c.id,
        state,
        committedAt: c.committed_at,
        message: `Claims are paid in commit order: commit ${c.id}, ahead of yours, is not final yet (${state}). Yours is paid once it settles.`,
      };
    } catch {
      return undefined;
    }
  }

  /** practice.rs `try_it`'s close: the practice key calls `close`, then the public vend key `reclaim`. */
  private async stepClose(): Promise<void> {
    const key = this.requireKey();
    const { account } = this.s;
    this.begin("close", `${account} tells ${this.vending} its try is done`);
    if ((await this.d.rpc.viewAccount(account)) === null) {
      const vended = this.s.steps.vend.status === "done";
      this.skip("close", vended ? `${account} is already deleted` : `${account} does not exist: nothing to close`);
      this.skip("reclaim", vended ? `its NEAR is already back with ${this.vending}` : "nothing to reclaim");
      this.set({ closed: true });
      this.persist({ closed: true });
      return;
    }
    const out = await this.send(key, account, account, { method: "close", args: jsonArgs({}), gas: CLOSE_TGAS * TGAS, deposit: BigInt(0) });
    this.ensureOk(out, "close");
    this.done("close", `the try is marked done at ${this.vending}`, { tx: txHash(out), signer: account });
    this.timeStep("close", out);
    this.begin("reclaim", `The public vend key asks ${this.vending} to delete ${account}`);
    const out2 = await this.send(this.vendKey, this.vending, this.vending, { method: "reclaim", args: borshString(account), gas: RECLAIM_TGAS * TGAS, deposit: BigInt(0) });
    this.ensureOk(out2, "reclaim");
    const closed = (await this.reread(() => this.d.rpc.viewAccount(account), (a) => a === null, 3)) === null;
    this.set({ closed });
    this.persist({ closed });
    this.done("reclaim", closed ? `${account} is deleted: its NEAR is back with ${this.vending}` : `reclaim ran, and ${account} still shows: NEAR views may lag a block`, {
      tx: txHash(out2),
      signer: `${this.vending} (public vend key)`,
    });
    this.timeStep("reclaim", out2);
  }

  /** After a failure: close the account if it exists (`try_it` closes it, done or not). */
  private async closeAfterFailure(): Promise<string | undefined> {
    const { account } = this.s;
    try {
      if ((await this.d.rpc.viewAccount(account)) === null) return this.s.steps.vend.status === "done" ? `${account} is already deleted` : undefined;
      await this.stepClose();
      return this.s.closed ? `${account} is closed, its NEAR back to ${this.vending}` : `${account} is marked done; its reclaim ran`;
    } catch (c) {
      const running = (["close", "reclaim"] as StepId[]).find((id) => this.s.steps[id].status === "running");
      if (running) this.patch(running, { status: "failed", detail: errText(c) });
      return `${account} stays open (${errText(c)}); ${this.vending}'s reclaim closes it an hour after its vend`;
    }
  }

  /** The commit as the chain has it now (practice.rs `try_it`'s summary). */
  private async finalRead(): Promise<void> {
    if (this.s.commitId === undefined) return;
    try {
      const c = await this.getCommit();
      if (!c) return;
      const state = commitStateLabel(c.state);
      this.set({ state, paidRaw: state === "Done:Paid" ? paidOf(c) : this.s.paidRaw, beneficiary: c.beneficiary ?? this.s.beneficiary });
      this.persist({ finalState: state });
    } catch {
      // the state read during the try stays
    }
  }

  // ---------------------------------------------------------------------------------- NEAR

  /**
   * near.rs `send`: the nonce from the key's latest state, the block hash from a final block; on
   * `InvalidNonce` the node's `ak_nonce` + 1, on `Expired` the access key view's block hash, then a fresh
   * final one. Four attempts.
   */
  private async send(key: KeyPair, signer: string, receiver: string, call: FunctionCall): Promise<TxOutcome> {
    const pk = publicKeyString(key.publicKey);
    const id = `${signer}:${pk}`;
    // The practice key was added by the vend a moment ago: a node a block behind may not know it yet.
    let ak: AccessKeyView | undefined;
    for (let i = 0; ak === undefined; i++) {
      try {
        ak = await this.d.rpc.viewAccessKey(signer, pk);
      } catch (e) {
        if (i >= 4 || !/does not exist|UNKNOWN_ACCESS_KEY|UNKNOWN_ACCOUNT/.test(errText(e))) throw e;
        await this.nap(1000, true);
      }
    }
    let blockHash = (await this.d.rpc.block("final")).header.hash;
    let nonce = ak.nonce + BigInt(1);
    for (let attempt = 0; attempt < 4; attempt++) {
      const last = this.nonces.get(id);
      if (last !== undefined && nonce <= last) nonce = last + BigInt(1);
      this.nonces.set(id, nonce);
      const signed = signTx(encodeTx(signer, key.publicKey, nonce, receiver, blockHash, [call]), key.seed);
      try {
        const out = await this.d.rpc.sendTx(signed.signedBase64, "EXECUTED_OPTIMISTIC");
        this.set({ signed: this.s.signed + 1 });
        return out;
      } catch (e) {
        const m = errText(e);
        if (attempt < 3 && m.includes("InvalidNonce")) {
          // The node tells the key's current nonce: continue from there.
          const n = /"ak_nonce":\s*(\d+)/.exec(m);
          nonce = n ? BigInt(n[1]) + BigInt(1) : nonce + BigInt(1);
          continue;
        }
        if (attempt < 3 && m.includes('"Expired"')) {
          blockHash = attempt === 0 ? ak.blockHash : (await this.d.rpc.block("final")).header.hash;
          continue;
        }
        throw e;
      }
    }
    throw new FlowError("chain", `send_tx: nonce conflict for ${signer}`);
  }

  /** cmd.rs `ensure_ok`: any failure in the transaction tree, said once, with the transaction. */
  private ensureOk(out: TxOutcome, what: string): void {
    const f = [...new Set(outcomeFailures(out))];
    if (f.length) throw new FlowError("chain", `${what} failed: ${f.map(readableFailure).join("; ")}`, txHash(out) || undefined);
  }

  /** A view right after a write: asked again (1 s apart, `tries` in all) until `ok`, then the last answer. */
  private async reread<T>(read: () => Promise<T>, ok: (v: T) => boolean, tries = 5): Promise<T> {
    let v = await read();
    for (let i = 1; i < tries && !ok(v); i++) {
      await this.nap(1000, true);
      v = await read();
    }
    return v;
  }

  private async vaultOf(account: string): Promise<number | null> {
    return parseVaultOf((await this.d.rpc.viewFunction(this.vending, "vault_of", borshString(account))).bytes);
  }

  private async getCommit(): Promise<CommitView | null> {
    return this.getCommitById(this.s.commitId);
  }

  private async getCommitById(id: number | undefined): Promise<CommitView | null> {
    return (await this.d.rpc.viewFunction(this.s.antibody ?? "", "get_commit", { commit_id: id })).json<CommitView | null>();
  }

  private async pollRun(): Promise<RunState | undefined> {
    if (this.s.runId === undefined) return undefined;
    try {
      const raw = (await this.d.rpc.viewFunction(this.s.antibody ?? "", "get_run", { run_id: this.s.runId })).json<unknown>();
      return raw === null ? undefined : runState(raw);
    } catch (e) {
      // A busy RPC is not a failed run: the next poll (or the stream) answers.
      this.note("warn", `get_run: ${errText(e)}`);
      return undefined;
    }
  }

  /** A commit this account made before a reload: the program's latest commits, by hunter. */
  private async commitOfHunter(): Promise<number | undefined> {
    const antibody = this.s.antibody ?? "";
    const p = (await this.d.rpc.viewFunction(antibody, "get_program", { program_id: this.s.programId })).json<ProgramView | null>();
    const n = p?.commits ?? 0;
    const from = Math.max(0, n - 50);
    const list = (await this.d.rpc.viewFunction(antibody, "list_commits", { program_id: this.s.programId, from, limit: 50 })).json<CommitView[]>();
    return (Array.isArray(list) ? list : []).find((c) => c.hunter === this.s.account)?.id;
  }

  private async fetchLocation(url: string): Promise<Uint8Array> {
    const loc = parseLocation(url);
    if (loc) return contentFromTx(await this.d.archival.txStatus(loc.hash, loc.signer, "INCLUDED_FINAL"));
    if (url.startsWith("https://") && this.d.fetchUrl) return this.d.fetchUrl(url);
    throw new FlowError("program", `this page reads near-tx: locations, not ${url}: use the CLI`);
  }

  /** cmd.rs `program_manifest_at(…, 0)`: round 0's manifest from its committed location, checked against its hash. */
  private async roundZeroManifest(want: string): Promise<Manifest> {
    const url = this.meta?.manifest_url ?? "";
    const bytes = await this.fetchLocation(url);
    const got = toHex(sha256(bytes));
    if (got !== want) throw new FlowError("program", `round 0 manifest at ${url} hashes to ${got}, not the committed ${want}`);
    return parseManifestJson(bytes);
  }

  private async blockTime(hash: string): Promise<number> {
    const hit = this.blockTimes.get(hash);
    if (hit !== undefined) return hit;
    const b = await this.d.rpc.call<{ header?: { timestamp_nanosec?: string; timestamp?: number } }>("block", { block_id: hash });
    const ns = b?.header?.timestamp_nanosec;
    const ms = ns ? Number(BigInt(ns) / BigInt(1_000_000)) : (b?.header?.timestamp ?? 0) / 1e6;
    if (!ms) throw new Error("block: no timestamp");
    this.blockTimes.set(hash, ms);
    return ms;
  }

  /** The step's time: this tab's clock now, then its receipt's block time when the RPC answers. */
  private timeStep(step: StepId, out: TxOutcome, event?: string, onExact?: (at: number) => void): void {
    if (this.s.steps[step].at === undefined) this.patch(step, { at: this.d.now(), exact: false });
    const hash = (event ? eventIn(out, event)?.blockHash : undefined) ?? txBlockHash(out);
    if (!hash) return;
    const p = this.blockTime(hash)
      .then((ms) => {
        // The stream's indexed block time, when it came first, is the same block.
        if (!(step === "commit" && this.seen.commitAt !== undefined)) this.patch(step, { at: ms, exact: true });
        onExact?.(ms);
      })
      .catch(() => undefined);
    this.pending.push(p);
  }

  // ---------------------------------------------------------------------------------- the stream

  private openStream(): void {
    if (this.closeStream) return;
    if (!this.d.openStream) {
      this.set({ stream: "polling" });
      return;
    }
    const after = this.d.streamAfter;
    const url = `${this.d.apiBase}/v1/stream?scope=antibody${after !== undefined ? `&after=${after}` : ""}`;
    this.set({ stream: "connecting" });
    try {
      this.closeStream = this.d.openStream(url, {
        open: () => this.set({ stream: "live" }),
        event: (e) => this.onStreamEvent(e),
        error: (closed) => {
          if (closed) {
            this.closeStream?.();
            this.closeStream = undefined;
            if (this.running) {
              this.set({ stream: "polling" });
              this.note("warn", "The live stream closed: the try polls NEAR (get_run) instead.");
            }
          } else if (this.s.stream === "live") this.set({ stream: "connecting" });
        },
      });
    } catch {
      this.closeStream = undefined;
      this.set({ stream: "polling" });
    }
  }

  private onStreamEvent(e: StreamEvent): void {
    const data = rec(e.data) ?? {};
    const at = typeof e.block_ts_ns === "number" ? e.block_ts_ns / 1e6 : undefined;
    const runId = this.s.runId;
    const commitId = this.s.commitId;
    const ofRun = runId !== undefined && data.run_id === runId;
    const ofCommit = commitId !== undefined && data.commit_id === commitId;
    switch (e.event) {
      case "commit":
        if (ofCommit && at !== undefined) {
          this.seen.commitAt = at;
          this.patch("commit", { at, exact: true });
          this.persist({ commitAt: at, commitExact: true });
        }
        break;
      case "run_answered":
        if (ofRun || (ofCommit && data.mode === "verify" && runId === undefined)) {
          this.seen.answer = {
            verdict: {
              kind: str(data.kind) ?? null,
              keys: keysOf(data.keys),
              tier: str(data.tier) ?? null,
              receipt: str(data.receipt_sha256) ?? null,
              // near-antibody records the answer's block time in seconds (`Answered.at`).
              answeredAt: at !== undefined ? Math.floor(at / 1000) : undefined,
            },
            tx: e.tx,
            at,
          };
          // Polling may have seen the answer first, by this tab's clock: the indexed block time is exact.
          const v = this.s.steps.verdict;
          if (v.status === "done" && !v.exact && at !== undefined) this.patch("verdict", { at, exact: true, tx: v.tx ?? e.tx });
        }
        break;
      case "t1_confirmed":
        if (ofRun) {
          this.seen.t1 = { tx: e.tx, at };
          const t = this.s.steps.t1;
          if (t.status === "done" && !t.exact && at !== undefined) this.patch("t1", { at, exact: true, tx: t.tx ?? e.tx });
        }
        break;
      case "run_failed":
        if (ofRun) this.seen.failed = str(data.reason) ?? "failed";
        break;
      case "holder":
      case "paid":
        if (ofCommit) this.seen.settled = e.event;
        break;
      case "expired":
        if (num(data.commit_id) !== undefined && data.commit_id === this.s.held?.commitId) this.seen.expired = num(data.commit_id);
        break;
      default:
        return;
    }
    this.wake?.();
  }

  // ---------------------------------------------------------------------------------- state

  private pollEvery(): number {
    return this.s.stream === "live" ? this.t.pollLiveMs : this.t.pollMs;
  }

  /** Sleeps `ms`, or until the stream brings news. `always`: even after a stop (a step's own short wait). */
  private async nap(ms: number, always = false): Promise<void> {
    if (this.stopRequested && !always) return;
    let wake: () => void = () => undefined;
    const woken = new Promise<void>((r) => (wake = r));
    this.wake = wake;
    await Promise.race([this.d.sleep(ms), woken]);
    this.wake = undefined;
  }

  private requireKey(): KeyPair {
    if (!this.key) throw new FlowError("other", "no practice key in this tab");
    return this.key;
  }

  private requireProgram(): { program: ProgramView; meta: MetaView } {
    if (!this.program || !this.meta) throw new FlowError("other", "the program was not read");
    return { program: this.program, meta: this.meta };
  }

  private persist(patch: Partial<SavedTry>): void {
    if (!this.saved) return;
    this.saved = { ...this.saved, ...patch };
    try {
      this.d.storage?.save(this.saved);
    } catch {
      // storage is a convenience: the try goes on without it
    }
  }

  private set(patch: Partial<TrySnapshot>): void {
    this.s = { ...this.s, ...patch };
    this.emit();
  }

  private patch(id: StepId, patch: Partial<StepState>): void {
    this.s = { ...this.s, steps: { ...this.s.steps, [id]: { ...this.s.steps[id], ...patch } } };
    this.emit();
  }

  private begin(id: StepId, detail: string): void {
    this.patch(id, { status: "running", detail });
    this.set({ announce: detail });
  }

  private done(id: StepId, detail: string, extra: Partial<StepState> = {}): void {
    this.patch(id, { status: "done", detail, ...extra });
    this.set({ announce: detail });
  }

  private skip(id: StepId, detail: string, tx?: string): void {
    this.patch(id, { status: "skipped", detail, ...(tx ? { tx } : {}) });
  }

  private note(level: "info" | "warn" | "error", text: string): void {
    this.s = { ...this.s, log: [...this.s.log, { at: this.d.now(), level, text }] };
    this.emit();
  }

  private emit(): void {
    for (const fn of this.listeners) {
      try {
        fn(this.s);
      } catch {
        // a listener's failure is its own
      }
    }
  }
}

/** The tutorial bundle the proof carries, for the page to show (practice.rs `tutorial_bundle`). */
export const bundleOf = tutorialBundle;
