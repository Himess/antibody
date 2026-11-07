/**
 * The commit and run pages' reading of the API (docs/api.md): pure functions, no rendering. The API may
 * omit a field or send an unexpected type, so every reader here tolerates both and falls back to "—".
 */
import type { Expect } from "@/components/ui/LiveCheck";
import type { FeedRow, IndexedEvent, KnownIssue, ProgramDetail, Severity, T2Panel, T2Record, VerdictKey } from "@/lib/api";
import type { Result } from "@/lib/api";
import { cap, fixtureAgentLetter, provenance, wnear } from "@/lib/format";

/** The near-antibody account, when /health cannot be read (the locked testnet deployment, README). */
export const DEFAULT_ANTIBODY = "bounty.antibody.testnet";

/** On this deployment, programs 0 to 4 are earlier runs' (JUDGES.md); 5, 6 and 7 are the recorded run's. */
const EARLIER_RUN_MAX_PROGRAM = 4;

export const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

export function numOf(v: unknown): number | null {
  if (typeof v === "number" && Number.isFinite(v)) return v;
  if (typeof v === "string" && /^\d{1,16}$/.test(v)) return Number(v);
  return null;
}

export const strOf = (v: unknown): string | null => (typeof v === "string" && v !== "" ? v : null);

/** A route id: a non-negative integer, or null (the page answers 404). */
export function parseId(s: string): number | null {
  if (!/^\d{1,15}$/.test(s)) return null;
  const n = Number(s);
  return Number.isSafeInteger(n) ? n : null;
}

/** The API answers an unknown id with "commit 99 does not exist on …" (HTTP 502). */
export const unknownId = (r: Result<unknown>) => !r.ok && /does not exist/i.test(r.error);

/** A program name as served, with a double-encoded dash repaired ("â€”" is "—" read as Latin-1). */
export const fixText = (s: string) => s.replace(/â€”/g, "—").replace(/â€“/g, "–").replace(/â€™/g, "’");

// ------------------------------------------------------------------ commit state

/** The contract's state as the feed names it (cmd::state_label): "Holder", "Done:Paid", … */
export function stateLabelOf(raw: unknown): string {
  if (typeof raw === "string") return raw;
  if (!isObj(raw)) return "";
  const [k, v] = Object.entries(raw)[0] ?? [];
  if (!k) return "";
  if (k === "Done") {
    if (typeof v === "string") return `Done:${v}`;
    if (isObj(v)) return `Done:${Object.keys(v)[0] ?? ""}`;
    return "Done:";
  }
  return k;
}

/** The fields of the state's variant: `{amount, instant, at}` for Done:Paid, `{of, …}` for Duplicate, … */
export function stateData(raw: unknown): Record<string, unknown> | null {
  if (!isObj(raw)) return null;
  const [k, v] = Object.entries(raw)[0] ?? [];
  if (k === "Done") {
    if (!isObj(v)) return null;
    const inner = Object.values(v)[0];
    return isObj(inner) ? inner : null;
  }
  return isObj(v) ? v : null;
}

/** One or two words for titles: "Paid", "Duplicate", "Known issue". */
export function shortState(label: string): string {
  const m: Record<string, string> = {
    "Done:Paid": "Paid",
    "Done:Forfeited": "Bond forfeited",
    "Done:DuplicateConfirmed": "Duplicate",
    Duplicate: "Duplicate",
    "Done:Known": "Known issue",
    "Done:Refunded": "Refunded",
    "Done:Expired": "Expired",
    "Done:Exhausted": "Budget spent",
    "Done:GapOpen": "Gap claim",
    "Done:Defaulted": "Defaulted",
    Holder: "Verified",
    Verifying: "Verifying",
    Committed: "Committed",
    GapClaim: "Gap claim",
    Promoted: "Promoted",
  };
  return m[label] ?? (label.startsWith("Done:") ? label.slice(5) : label || "Unknown state");
}

export type BondFate = "returned" | "forfeited" | "held" | "unknown";

/** Spec §7.3, §7.7, §12.2: a bond is refunded unless the proof broke nothing or was never revealed. */
export function bondFate(label: string): BondFate {
  if (!label) return "unknown";
  if (label === "Committed" || label === "Verifying" || label === "Revealed") return "held";
  if (label === "Done:Forfeited" || label === "Done:Expired") return "forfeited";
  return "returned";
}

export const kindWords = (k: string | null | undefined): string =>
  !k
    ? "Pending"
    : k === "NoViolation"
      ? "No violation"
      : k === "InvalidPoC"
        ? "Invalid proof"
        : k === "ProgramMisconfigured"
          ? "Program misconfigured"
          : k === "InfraError"
            ? "Infrastructure error"
            : k;

// ------------------------------------------------------------------ who

/** "our fixture agent A", "a practice try", … (null for an account that is not ours). */
export function who(account: string | null | undefined): string | null {
  if (!account) return null;
  const p = provenance(account);
  if (p === "fixture-agent") return `our fixture agent ${fixtureAgentLetter(account) ?? ""}`.trim();
  if (p === "practice") return account === "practice.antibody.testnet" ? "the practice vending contract" : "a practice try";
  if (p === "ours") return "our account";
  return null;
}

/** A short name for lists: "agent A", "judge-1-p6", else the account. */
export function shortWho(account: string | null | undefined): string {
  if (!account) return "—";
  const p = provenance(account);
  if (p === "fixture-agent") return `agent ${fixtureAgentLetter(account) ?? ""}`.trim();
  if (p === "practice") return account.split(".")[0];
  return account;
}

/** The subject of a sentence: "Our fixture agent A", else the account. */
export const subject = (account: string | null | undefined) => {
  const w = who(account);
  return w ? cap(w) : (account ?? "The hunter");
};

// ------------------------------------------------------------------ programs and rules

export interface ProgramInfo {
  id: number;
  name: string | null;
  practice: boolean;
  earlier: boolean;
  ours: boolean;
}

export function programInfo(id: number, p: ProgramDetail | null | undefined): ProgramInfo {
  const name = p?.meta?.name ? fixText(String(p.meta.name)) : null;
  return {
    id,
    name,
    practice: name ? /practice program/i.test(name) : id === 6 || id === 7,
    earlier: id <= EARLIER_RUN_MAX_PROGRAM,
    ours: p?.program?.project ? provenance(String(p.program.project)) !== "other" : true,
  };
}

const SEVERITIES: Severity[] = ["critical", "high", "medium", "low"];
const isSeverity = (s: unknown): s is Severity => typeof s === "string" && (SEVERITIES as string[]).includes(s);

/** A slot's name and severity: the program's rules, its metadata, then its params (a round may add slots). */
export function slotInfo(p: ProgramDetail | null | undefined, slot: number): { name: string | null; severity: Severity | null; added: boolean } {
  const r = p?.rules?.slots?.find((s) => s?.bit === slot);
  if (r) return { name: r.name ?? null, severity: isSeverity(r.severity) ? r.severity : null, added: false };
  const m = p?.meta?.slots?.find((s) => Array.isArray(s) && s[0] === slot);
  if (m) return { name: m[1] ?? null, severity: isSeverity(m[2]) ? m[2] : null, added: false };
  const ps = p?.program?.params?.slots;
  if (Array.isArray(ps)) {
    const x = ps.find((s) => isObj(s) && numOf(s.slot) === slot);
    if (isObj(x)) return { name: null, severity: isSeverity(x.severity) ? x.severity : null, added: true };
  }
  return { name: null, severity: null, added: false };
}

/** The highest severity among the keys' slots (VerdictStamp, the lede). */
export function topSeverity(p: ProgramDetail | null | undefined, keys: VerdictKey[]): Severity | null {
  let best: Severity | null = null;
  for (const k of keys) {
    const s = slotInfo(p, k.slot).severity;
    if (s && (best === null || SEVERITIES.indexOf(s) < SEVERITIES.indexOf(best))) best = s;
  }
  return best;
}

/** "slot 0 (solvency)". */
export function slotName(p: ProgramDetail | null | undefined, slot: number): string {
  const i = slotInfo(p, slot);
  return i.name ? `slot ${slot} (${i.name})` : `slot ${slot}`;
}

/** Keys from any source, dropping malformed entries. */
export function cleanKeys(v: unknown): VerdictKey[] {
  if (!Array.isArray(v)) return [];
  return v.flatMap((k) => (isObj(k) && numOf(k.slot) !== null && typeof k.tag === "string" ? [{ slot: numOf(k.slot) as number, tag: k.tag }] : []));
}

export const sameKey = (a: VerdictKey, b: VerdictKey) => a.slot === b.slot && a.tag.toLowerCase() === b.tag.toLowerCase();

const HELD = new Set(["Done:Paid", "Holder", "Promoted", "Done:Exhausted", "Done:Defaulted"]);

export interface EarlierClaim {
  row: FeedRow;
  /** This commit's key and the earlier claim's equal key, with each key's position in its run's facts. */
  matches: { mine: VerdictKey; mineIndex: number; theirs: VerdictKey; theirsIndex: number }[];
}

/**
 * The claim a duplicate duplicates: the commit near-antibody names (`of`), else the earliest commit of the
 * program's feed that holds an equal key (same slot, same tag) and was paid or holds it.
 */
export function earlierClaim(rows: FeedRow[], selfId: number, keys: VerdictKey[], of: number | null): EarlierClaim | null {
  const named = of !== null ? rows.find((r) => r.commit_id === of) : undefined;
  const pool = named
    ? [named]
    : rows.filter((r) => r.commit_id !== selfId && r.commit_id < selfId && HELD.has(r.state)).sort((a, b) => a.commit_id - b.commit_id);
  for (const r of pool) {
    const theirs = cleanKeys(r.verdict?.keys);
    const matches = keys.flatMap((k, mineIndex) => {
      const theirsIndex = theirs.findIndex((t) => sameKey(t, k));
      return theirsIndex >= 0 ? [{ mine: k, mineIndex, theirs: theirs[theirsIndex], theirsIndex }] : [];
    });
    if (matches.length || named) return { row: r, matches };
  }
  return null;
}

/** The known issue (declared sealed before open) whose tag this commit's key equals. */
export function knownMatch(p: ProgramDetail | null | undefined, keys: VerdictKey[]): { issue: KnownIssue; index: number; tagIndex: number; key: VerdictKey; keyIndex: number } | null {
  const declared = p?.program?.known;
  const known = Array.isArray(declared) ? declared : [];
  for (const [keyIndex, key] of keys.entries()) {
    for (const [index, issue] of known.entries()) {
      const tags = Array.isArray(issue?.tags) ? issue.tags : [];
      const tagIndex = tags.findIndex((t) => typeof t === "string" && t.toLowerCase() === key.tag.toLowerCase());
      if (tagIndex >= 0) return { issue, index, tagIndex, key, keyIndex };
    }
  }
  return null;
}

// ------------------------------------------------------------------ events

export const evData = (e: IndexedEvent | null | undefined): Record<string, unknown> => (isObj(e?.data) ? (e.data as Record<string, unknown>) : {});

export const runOfEvent = (e: IndexedEvent) => numOf(evData(e).run_id);
export const modeOfEvent = (e: IndexedEvent) => strOf(evData(e).mode);

/** Events in log order (seq), malformed ones dropped. */
export function sortedEvents(events: unknown): IndexedEvent[] {
  if (!Array.isArray(events)) return [];
  return (events as IndexedEvent[])
    .filter((e) => isObj(e) && typeof e.event === "string" && typeof e.tx === "string")
    .sort((a, b) => (numOf(a.seq) ?? 0) - (numOf(b.seq) ?? 0));
}

/** The first event named `name` for run `run`. */
export const runEvent = (events: IndexedEvent[], name: string, run: number | null | undefined) =>
  run === null || run === undefined ? undefined : events.find((e) => e.event === name && runOfEvent(e) === run);

/** Seconds between two block times (ns; float precision is ample at 0.1 s). */
export function secondsBetween(fromNs: number | null | undefined, toNs: number | null | undefined): number | null {
  if (typeof fromNs !== "number" || typeof toNs !== "number" || !fromNs || !toNs) return null;
  return (toNs - fromNs) / 1e9;
}

/** "+18.5 s", "+11 min", "+45.5 h". */
export function offset(s: number | null | undefined): string {
  if (s === null || s === undefined || !Number.isFinite(s)) return "—";
  const a = Math.abs(s);
  const body = a >= 3600 ? `${(a / 3600).toFixed(1)} h` : a >= 600 ? `${Math.round(a / 60)} min` : `${a.toFixed(1)} s`;
  return `${s < 0 ? "−" : "+"}${body}`;
}

// ------------------------------------------------------------------ money

/** Micro-USD (the verifier's unit) as dollars: "462931853867" → "$462,931.85". */
export function usd(micro: unknown): string {
  if (micro === null || micro === undefined || micro === "") return "—";
  let v: bigint;
  try {
    v = BigInt(typeof micro === "number" ? Math.trunc(micro) : String(micro));
  } catch {
    return "—";
  }
  const neg = v < BigInt(0);
  if (neg) v = -v;
  const million = BigInt(1_000_000);
  const dollars = v / million;
  const cents = (v % million) / BigInt(10_000);
  return `${neg ? "-" : ""}$${dollars.toLocaleString("en-US")}.${cents.toString().padStart(2, "0")}`;
}

/** What `get_commit` should answer for this state (the LiveCheck's expectation). */
export function stateExpect(label: string, raw: unknown): Expect | undefined {
  if (!label) return undefined;
  if (label === "Done:Paid") {
    const amount = strOf(stateData(raw)?.amount);
    if (amount) return { path: "state.Done.Paid.amount", equals: amount, says: `state Done:Paid, ${wnear(amount)}` };
  }
  if (label.startsWith("Done:") && isObj(raw) && typeof raw.Done === "string") {
    return { path: "state.Done", equals: raw.Done, says: `state ${label}` };
  }
  if (label === "Duplicate") {
    const of = numOf(stateData(raw)?.of);
    if (of !== null) return { path: "state.Duplicate.of", equals: of, says: `state Duplicate, of commit ${of}` };
  }
  const key = label.startsWith("Done:") ? label.slice(5) : label;
  return { path: "state", contains: key, says: `state ${label}` };
}

// ------------------------------------------------------------------ runs

export const MODE_WORDS: Record<string, string> = {
  activate: "Activation: the TEE derived the program's keys and output its sealing public key.",
  verify: "Verification: the TEE judged a sealed submission.",
  second: "A second, independent run of the same proof: above the instant cap, a payout needs one that agrees.",
  rerun: "A re-run of a paid proof, a duplicate or a gap claim at a newer round's code and rules.",
  check_round: "A round check: the TEE checked a proposed fix before its re-runs.",
  distinct: "A distinctness run (switched off in this deployment).",
  release_wrap: "A release wrap: after payment, the proof's content key wrapped to the project's own key.",
  disclose: "A disclosure run: the TEE released the disclosure keys of qualifying tags.",
};

export interface Answered {
  at: number | null;
  tier: string | null;
  kind: string | null;
  keys: VerdictKey[];
  impact: string | null;
  harm: string | null;
  receipt: string | null;
  reason: string | null;
  output: string | null;
  extra: unknown;
}

/** A run's outcome: `{Answered: {at, facts, output_sha256, tier}}`, `{Failed: {reason}}`, or still in flight. */
export function runOutcome(outcome: unknown): { status: "answered"; a: Answered } | { status: "failed"; reason: string } | { status: "pending" } {
  if (isObj(outcome) && isObj(outcome.Answered)) {
    const a = outcome.Answered;
    const f = isObj(a.facts) ? a.facts : {};
    return {
      status: "answered",
      a: {
        at: numOf(a.at),
        tier: strOf(a.tier),
        kind: strOf(f.kind),
        keys: cleanKeys(f.keys),
        impact: f.impact_usd_micro === undefined || f.impact_usd_micro === null ? null : String(f.impact_usd_micro),
        harm: f.protected_harm_usd_micro === undefined || f.protected_harm_usd_micro === null ? null : String(f.protected_harm_usd_micro),
        receipt: strOf(f.receipt_sha256),
        reason: strOf(f.reason),
        output: strOf(a.output_sha256),
        extra: f.extra,
      },
    };
  }
  if (isObj(outcome) && outcome.Failed !== undefined) {
    const f = outcome.Failed;
    const reason = isObj(f) ? (strOf(f.reason) ?? JSON.stringify(f)) : String(f);
    return { status: "failed", reason };
  }
  return { status: "pending" };
}

/** Whether the run's facts carry a disclosure wrap (`wrap_d`): only then can a disclosure key open the proof. */
export function hasWrapD(extra: unknown): boolean | null {
  if (extra === undefined) return null;
  if (!isObj(extra)) return false;
  return Object.keys(extra).some((k) => k.toLowerCase() === "wrapd") || Object.values(extra).some((v) => isObj(v) && "wrap_d" in v);
}

// ------------------------------------------------------------------ T2

export interface T2Find {
  record: T2Record | null;
  event: IndexedEvent | null;
  contract: string | null;
  /** near-t2's pinned verifier (its version string and wasm sha256), when /v1/t2 answers. */
  verifier: { version: string | null; wasm: string | null } | null;
  /** near-t2 is not on this deployment yet. */
  notYet: boolean;
  /** /v1/t2 could not be read (and no record came with the run). */
  error: string | null;
}

/** near-t2's record of a run: from the run's own read, else /v1/t2; with the transaction that recorded it. */
export function findT2(runId: number | null, runT2: T2Record | null | undefined, panel: Result<T2Panel>, notYet: boolean): T2Find {
  const records = panel.ok && Array.isArray(panel.data?.records) ? panel.data.records : [];
  const events = panel.ok ? sortedEvents(panel.data?.events) : [];
  const record = runT2 ?? (runId === null ? null : (records.find((r) => numOf(r?.run_id) === runId) ?? null));
  const event =
    runId === null ? null : (events.find((e) => (e.event === "t2_confirmed" || e.event === "t2_contradicts") && runOfEvent(e) === runId) ?? null);
  return {
    record: record ?? null,
    event,
    contract: panel.ok ? (strOf(panel.data?.contract) ?? null) : null,
    verifier:
      panel.ok && isObj(panel.data?.pins)
        ? { version: strOf(panel.data.pins.verifier), wasm: strOf(panel.data.pins.verifier_wasm) }
        : null,
    notYet: !record && notYet,
    error: !record && !panel.ok && !notYet ? panel.error : null,
  };
}

// ------------------------------------------------------------------ the recorded run

/**
 * What each commit was in the recorded run of 2026-11-04 (JUDGES.md, rows 1–7, 12, 14–16). Shown only
 * when the program and the hunter match, so another deployment's commit with the same id gets none.
 */
export const RECORDED: Record<number, { program: number; hunter: string; rows: number[]; text: string }> = {
  13: {
    program: 5,
    hunter: "agent-a.antibody.testnet",
    rows: [1, 8, 15, 16],
    text: "An AI agent's sealed proof, submitted over MCP: verified in the TEE, T1-confirmed and paid in seconds under the cap. Its verdict paused the fixture on both chains, the verdict guard froze on it, and the project read the proof through its release wrap.",
  },
  14: {
    program: 5,
    hunter: "agent-b.antibody.testnet",
    rows: [2],
    text: "A second planted bug (the unguarded fee sweep): verified and paid after its window.",
  },
  15: {
    program: 5,
    hunter: "agent-c.antibody.testnet",
    rows: [3],
    text: "C put a prompt injection in its report. The verifier never reads reports: the proof broke nothing, so it earns nothing, and its bond went to the program's escrow.",
  },
  16: {
    program: 5,
    hunter: "agent-d.antibody.testnet",
    rows: [4, 15, 16],
    text: "A duplicate of A's finding: recorded with its own proof, then confirmed a duplicate at round 1's fix. Reported to the verdict guard before A's, it did not freeze it, and near-antibody refused to wrap it for the project.",
  },
  17: {
    program: 5,
    hunter: "agent-e.antibody.testnet",
    rows: [5],
    text: "A known issue the project declared, sealed, before opening: verified, matched, and not paid again.",
  },
  18: {
    program: 5,
    hunter: "agent-f.antibody.testnet",
    rows: [6],
    text: "A distinct planted bug: paid.",
  },
  19: {
    program: 5,
    hunter: "agent-g.antibody.testnet",
    rows: [7],
    text: "The gap lane: harm to a protected user that no rule caught. Refunded and recorded; round 1 added the rule, and the claim was promoted and paid.",
  },
  20: {
    program: 6,
    hunter: "judge-1-p6.practice.antibody.testnet",
    rows: [12],
    text: "A practice try with no account: the tutorial proof, sealed against the pinned base, verified and paid to the vending contract.",
  },
  21: {
    program: 6,
    hunter: "judge-2-p6.practice.antibody.testnet",
    rows: [12],
    text: "A practice try with no account: the tutorial proof, sealed against the pinned base, verified and paid to the vending contract.",
  },
  22: {
    program: 7,
    hunter: "agent-c.antibody.testnet",
    rows: [14],
    text: "The practice program in fork mode: its vault exists only inside the verifier, so the program needs no staging deployment.",
  },
};

export function recordedNote(id: number, program: number, hunter: string | null | undefined) {
  const r = RECORDED[id];
  return r && r.program === program && r.hunter === hunter ? r : null;
}
