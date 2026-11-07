/**
 * What the account page reads, all public: the API (/v1/accounts, /v1/programs and their feeds, /v1/events)
 * and, for numbers that must be current (right after a transaction) or when the API is busy, the same views
 * read by the browser from NEAR RPC. No read needs a key.
 */
import { get } from "@/lib/api";
import type { FeedRow, IndexedEvent, ProgramSummary } from "@/lib/api/types";
import { nearRpc } from "@/lib/near/client";
import { RpcError } from "@/lib/near/rpc";
import { rawOf } from "./amounts";
import { ANTIBODY, RELEASE, WNEAR, WNEAR_REGISTRATION_FALLBACK } from "./contracts";
import type { AccountData, Activity, ActivityRow, AgentRecord, AgentStatement, ContractState, Credit, History } from "./types";

type Ok<T> = { ok: true; data: T } | { ok: false; error: string };

const obj = (v: unknown): Record<string, unknown> | null => (typeof v === "object" && v !== null && !Array.isArray(v) ? (v as Record<string, unknown>) : null);
const str = (v: unknown): string | undefined => (typeof v === "string" ? v : typeof v === "number" || typeof v === "bigint" ? String(v) : undefined);
const num = (v: unknown): number | undefined => (typeof v === "number" && Number.isFinite(v) ? v : undefined);
const nums = (v: unknown): number[] | null => (Array.isArray(v) ? v.filter((x): x is number => typeof x === "number") : null);
const message = (e: unknown) => (e instanceof Error ? e.message : typeof e === "string" ? e : JSON.stringify(e));

// ------------------------------------------------------------------ account ids

/** NEAR's account id rules: 2 to 64 of a-z, 0-9 and `-_` separators, in dot-separated parts. */
export function validAccountId(id: string): boolean {
  return id.length >= 2 && id.length <= 64 && /^(([a-z\d]+[-_])*[a-z\d]+\.)*([a-z\d]+[-_])*[a-z\d]+$/.test(id);
}

/** An implicit account (a key's hex, or an Ethereum-style address): it cannot have sub-accounts. */
export const isImplicit = (id: string) => /^[0-9a-f]{64}$/.test(id) || /^0x[0-9a-f]{40}$/.test(id);

/** A testnet account: a `.testnet` name or an implicit account. `alice.near` is a mainnet name. */
export const isTestnetAccount = (id: string) => validAccountId(id) && (id.endsWith(".testnet") || isImplicit(id));

// ------------------------------------------------------------------ normalizers

function agentRecord(v: unknown): AgentRecord | null {
  const o = obj(v);
  if (!o) return null;
  return {
    owner: str(o.owner),
    bond_budget: str(o.bond_budget),
    per_commit_cap: str(o.per_commit_cap),
    bonds_locked: str(o.bonds_locked),
    bonds_lost: str(o.bonds_lost),
    bonds_returned: str(o.bonds_returned),
    paid: str(o.paid),
    verification_spent: str(o.verification_spent),
    commits: num(o.commits),
    containment_events: num(o.containment_events),
    programs: nums(o.programs),
  };
}

function credit(v: unknown): Credit | null {
  const o = obj(v);
  if (!o) return null;
  return { tokens: str(o.tokens) ?? null, near: str(o.near) ?? null, committed: o.committed === true, agent: agentRecord(o.agent) };
}

function statement(v: unknown): AgentStatement | null {
  const o = obj(v);
  if (!o) return null;
  return {
    agent: str(o.agent),
    registered: o.registered === true,
    owner: str(o.owner),
    bond_budget: str(o.bond_budget),
    per_commit_cap: str(o.per_commit_cap),
    bonds_locked: str(o.bonds_locked),
    bonds_lost: str(o.bonds_lost),
    bonds_returned: str(o.bonds_returned),
    paid: str(o.paid),
    verification_spent_yocto: str(o.verification_spent_yocto),
    net_tokens: str(o.net_tokens),
    commits: num(o.commits),
    containment_events: num(o.containment_events),
    key_permissions: str(o.key_permissions),
  };
}

// ------------------------------------------------------------------ reads

/** The three views, read by the browser from NEAR (optimistic: a transaction just executed shows). */
async function accountFromNear(id: string): Promise<AccountData> {
  const rpc = nearRpc();
  const [c, s, b] = await Promise.all([
    rpc.viewFunction(ANTIBODY, "get_account", { account_id: id }),
    rpc.viewFunction(ANTIBODY, "agent_statement", { agent: id }),
    rpc.viewFunction(WNEAR, "ft_balance_of", { account_id: id }),
  ]);
  return { account: id, credit: credit(c.json()), statement: statement(s.json()), tokenBalance: str(b.json()) ?? null, source: "near", at: Date.now() };
}

/**
 * The account's credit, agent statement and wNEAR balance: from /v1/accounts/{id}, or (`fresh`, or when the
 * API fails) from the same views on NEAR.
 */
export async function loadAccount(id: string, fresh = false): Promise<Ok<AccountData>> {
  let apiError: string | undefined;
  if (!fresh) {
    const r = await get<unknown>(`/v1/accounts/${encodeURIComponent(id)}`);
    if (r.ok) {
      const o = obj(r.data) ?? {};
      return {
        ok: true,
        data: { account: id, credit: credit(o.credit), statement: statement(o.agent), tokenBalance: str(o.token_balance_raw) ?? null, source: "api", at: r.at },
      };
    }
    apiError = r.error;
  }
  try {
    const data = await accountFromNear(id);
    return { ok: true, data: { ...data, apiError } };
  } catch (e) {
    return { ok: false, error: apiError ? `API: ${apiError}; NEAR: ${message(e)}` : message(e) };
  }
}

/** The account's NEAR balance (liquid and staked), or null when the account does not exist on testnet. */
export async function loadNearBalance(id: string): Promise<Ok<{ amount: string; locked: string } | null>> {
  try {
    const a = await nearRpc().viewAccount(id);
    return { ok: true, data: a ? { amount: a.amount, locked: a.locked } : null };
  } catch (e) {
    return { ok: false, error: message(e) };
  }
}

/** Whether wrap.testnet holds a storage registration for the account, and what one costs. */
export async function loadTokenRegistration(id: string): Promise<Ok<{ registered: boolean; min: bigint }>> {
  try {
    const rpc = nearRpc();
    const [bal, bounds] = await Promise.all([
      rpc.viewFunction(WNEAR, "storage_balance_of", { account_id: id }),
      rpc.viewFunction(WNEAR, "storage_balance_bounds", {}).catch(() => null),
    ]);
    const min = rawOf(obj(bounds?.json())?.min) ?? WNEAR_REGISTRATION_FALLBACK;
    return { ok: true, data: { registered: bal.json() !== null, min } };
  } catch (e) {
    return { ok: false, error: message(e) };
  }
}

/** The account's commits, as hunter or as beneficiary, across every program's feed (read in parallel). */
export async function loadActivity(id: string): Promise<Ok<Activity>> {
  const list = await get<{ programs: ProgramSummary[] }>("/v1/programs");
  if (!list.ok) return { ok: false, error: list.error };
  const programs = (Array.isArray(list.data?.programs) ? list.data.programs : [])
    .filter((p) => typeof p?.program_id === "number")
    .map((p) => ({ id: p.program_id, name: p.name ?? `Program ${p.program_id}`, project: p.project ?? "", phase: p.phase ?? "" }));
  const feeds = await Promise.all(programs.map((p) => get<{ submissions: FeedRow[] }>(`/v1/programs/${p.id}/feed`)));
  const rows: ActivityRow[] = [];
  const failed: number[] = [];
  feeds.forEach((f, i) => {
    const p = programs[i];
    if (!f.ok) {
      failed.push(p.id);
      return;
    }
    for (const r of Array.isArray(f.data?.submissions) ? f.data.submissions : []) {
      if (r && (r.hunter === id || r.beneficiary === id)) rows.push({ ...r, program_id: r.program_id ?? p.id, programName: p.name });
    }
  });
  rows.sort((a, b) => (b.commit_id ?? 0) - (a.commit_id ?? 0));
  return { ok: true, data: { rows, programs, failed } };
}

/**
 * The agents this account registered (the latest `agent_registered` event per agent) and its withdrawals,
 * from the API's event log, read in pages of 1,000 (at most 20 pages).
 */
export async function loadHistory(id: string): Promise<Ok<History>> {
  const agents = new Map<string, History["agents"][number]>();
  const withdrawals: History["withdrawals"] = [];
  let after = 0;
  for (let page = 0; page < 20; page++) {
    const r = await get<{ events: IndexedEvent[] }>(`/v1/events?scope=antibody&after=${after}&limit=1000`);
    if (!r.ok) return { ok: false, error: r.error };
    const events = Array.isArray(r.data?.events) ? r.data.events : [];
    for (const e of events) {
      const d = obj(e?.data) ?? {};
      const at = typeof e?.block_ts_ns === "number" ? e.block_ts_ns : null;
      if (e?.event === "agent_registered" && d.owner === id && typeof d.agent === "string") {
        agents.set(d.agent, { agent: d.agent, bondBudget: str(d.bond_budget) ?? null, perCommitCap: str(d.per_commit_cap) ?? null, programs: nums(d.programs), tx: e.tx ?? null, at });
      } else if (e?.event === "withdraw" && d.account === id) {
        withdrawals.push({ amount: str(d.amount) ?? null, tx: e.tx ?? null, at });
      }
    }
    if (events.length < 1000) return { ok: true, data: { agents: [...agents.values()], withdrawals: withdrawals.reverse(), truncated: false } };
    after = (events[events.length - 1]?.seq ?? after) + 1;
  }
  return { ok: true, data: { agents: [...agents.values()], withdrawals: withdrawals.reverse(), truncated: true } };
}

/**
 * The code at the two contracts, against the release record: near-antibody's code hash and its full-access
 * keys (none: nobody can change it), and wrap.testnet's code hash (its owner may upgrade it).
 */
export async function checkContracts(): Promise<ContractState[]> {
  const rpc = nearRpc();
  const one = async (c: { account: string; codeHash: string; role: string }, keys: boolean): Promise<ContractState> => {
    const base: ContractState = { account: c.account, role: c.role, expected: c.codeHash, actual: null, matches: null, fullAccessKeys: null };
    try {
      const a = await rpc.viewAccount(c.account, "final");
      if (!a) return { ...base, error: "no such account" };
      let full: number | null = null;
      if (keys) {
        const list = obj(await rpc.call("query", { request_type: "view_access_key_list", finality: "final", account_id: c.account }));
        const k = Array.isArray(list?.keys) ? list.keys : [];
        full = k.filter((x) => obj(obj(x)?.access_key)?.permission === "FullAccess").length;
      }
      return { ...base, actual: a.code_hash, matches: a.code_hash === c.codeHash, fullAccessKeys: full };
    } catch (e) {
      return { ...base, error: e instanceof RpcError ? e.message : message(e) };
    }
  };
  return Promise.all([one(RELEASE.antibody, true), one(RELEASE.token, false)]);
}
