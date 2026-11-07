/**
 * Pure helpers for the programs directory and the program page: no React, no fetch, so the server and
 * the browser can both use them. Everything here reads the API's payloads defensively (fields may be
 * missing) and labels provenance the way the rest of the site does.
 */
import { explorers } from "@/lib/config";
import { fixtureAgentLetter, provenance, type Tone } from "@/lib/format";
import type { FeedRow, IndexedEvent, ProgramDetail, Severity } from "@/lib/api/types";

/* ------------------------------------------------------------------ what a program is */

export type ProgramKind = "fixture" | "practice" | "fork" | "program";

/** Acme Lend is our fixture; the practice programs are tutorials; fork mode is named in the program's name. */
export function programKind(name?: string | null, description?: string | null): ProgramKind {
  const n = (name ?? "").toLowerCase();
  const d = (description ?? "").toLowerCase();
  if (n.includes("fork mode") || d.includes("fork mode")) return "fork";
  if (n.startsWith("practice") || d.includes("tutorial target")) return "practice";
  if (n.includes("acme") || d.includes("deliberately vulnerable")) return "fixture";
  return "program";
}

export const KIND_TEXT: Record<ProgramKind, { short: string; long: string }> = {
  fixture: { short: "Acme Lend fixture", long: "Acme Lend: our deliberately vulnerable fixture" },
  practice: { short: "Practice", long: "Practice program: a tutorial" },
  fork: { short: "Fork mode", long: "Practice program in fork mode: a tutorial" },
  program: { short: "Program", long: "Program" },
};

/**
 * The recorded run (docs/results/testnet-2026-11-04.json; JUDGES.md: "This run's programs are 5 (Acme Lend),
 * 6 (practice) and 7 (fork mode); the others are earlier runs' on the same deployment").
 */
export const RECORDED_RUN = {
  results_file: "docs/results/testnet-2026-11-04.json",
  date: "2026-11-04",
  programs: [5, 6, 7] as readonly number[],
};

/**
 * The staging run, on the same contracts before the lock (README; its results file lists programs 0–3 and
 * records near-antibody `locked: false`). Program 4 came after it and before the recorded run.
 */
export const STAGING_RUN = {
  results_file: "docs/results/testnet-staging-2026-11-03.json",
  date: "2026-11-03",
  programs: [0, 1, 2, 3] as readonly number[],
};

export type RunGroup = "this-run" | "earlier" | "later";

/** This run's programs, the earlier runs' (0–4: the staging run, then program 4), or later ones. */
export function runGroup(id: number): RunGroup {
  if (RECORDED_RUN.programs.includes(id)) return "this-run";
  return id < Math.min(...RECORDED_RUN.programs) ? "earlier" : "later";
}

/** Among the earlier runs: the staging run's programs, before the lock. */
export const isStagingRun = (id: number) => STAGING_RUN.programs.includes(id);

/** The label of a try button: a try always goes to the program the practice vending contract names. */
export function tryLabel(id: number, practiceProgram: number | null): string {
  if (practiceProgram === id) return "Try it, no account";
  return practiceProgram !== null ? `Try the practice program (${practiceProgram})` : "Try the practice program";
}

/** A route segment as a program id: a canonical u32, else null (a 404). */
export function parseProgramId(s: string): number | null {
  if (!/^(0|[1-9]\d{0,9})$/.test(s)) return null;
  const n = Number(s);
  return Number.isSafeInteger(n) && n <= 0xffffffff ? n : null;
}

/* ------------------------------------------------------------------ phase and anchor */

const PHASES: Record<string, { text: string; tone: Tone }> = {
  Pending: { text: "Pending: anchor check", tone: "pending" },
  Setup: { text: "Setup: activation", tone: "pending" },
  Open: { text: "Open", tone: "ok" },
  Closed: { text: "Closed", tone: "neutral" },
  Exited: { text: "Exited", tone: "neutral" },
  Rejected: { text: "Rejected: anchor contradicted", tone: "bad" },
};

export function phaseOf(phase?: string | null): { text: string; tone: Tone } {
  if (!phase) return { text: "phase unknown", tone: "neutral" };
  return PHASES[phase] ?? { text: phase, tone: "neutral" };
}

/** rules.anchor.check: "light-client" when client-eth2 confirmed the anchor at creation, else project-attested. */
export const anchorConfirmed = (check?: string | null) => check === "light-client";

/** An explorer page for an EVM block, from the same explorers the site uses for addresses. */
export function evmBlockUrl(chainId: number, block: number): string | undefined {
  const base = explorers.evmAddress(chainId, "0x0")?.replace(/\/address\/0x0$/, "");
  return base ? `${base}/block/${block}` : undefined;
}

/* ------------------------------------------------------------------ amounts */

const big = (v: unknown): bigint | null => {
  if (typeof v !== "string" && typeof v !== "number") return null;
  try {
    return BigInt(typeof v === "number" ? Math.trunc(v) : v);
  } catch {
    return null;
  }
};

/** The largest payout in the table (raw units), as the API computes escrow coverage. */
export function largestPayout(payouts?: Partial<Record<Severity, string>> | null): string | null {
  let best: bigint | null = null;
  for (const v of Object.values(payouts ?? {})) {
    const b = big(v);
    if (b !== null && (best === null || b > best)) best = b;
  }
  return best === null ? null : best.toString();
}

/** rules.escrow_coverage (available ÷ largest payout), or the same ratio computed here. */
export function coverageOf(cov: number | null | undefined, available?: string | null, largest?: string | null): number | null {
  if (typeof cov === "number" && Number.isFinite(cov)) return cov;
  const a = big(available);
  const l = big(largest);
  if (a === null || l === null || l <= BigInt(0)) return null;
  return Number((a * BigInt(10000)) / l) / 10000;
}

/** 1.61 → "1.61×", 27.000000000000004 → "27×". */
export const coverageText = (c: number) => `${Number(c.toFixed(2)).toLocaleString("en-US")}×`;

export const isZero = (raw?: string | null) => {
  const b = big(raw);
  return b === null || b === BigInt(0);
};

/** A window's length in whole units where it divides: 3600 → "1 h", 600 → "10 min", 60 → "60 s". */
export function duration(s: number | null | undefined): string {
  if (typeof s !== "number" || !Number.isFinite(s)) return "—";
  if (s >= 3600 && s % 3600 === 0) return `${s / 3600} h`;
  if (s >= 120 && s % 60 === 0) return `${s / 60} min`;
  return `${s} s`;
}

/** Basis points as a percentage: 5000 → "50%". */
export const bps = (v: unknown) => (typeof v === "number" ? `${(v / 100).toLocaleString("en-US", { maximumFractionDigits: 2 })}%` : "—");

/** The verifier's impact unit (micro-USD) as dollars. */
export function usdMicro(v: unknown): string {
  const b = big(v);
  if (b === null) return "—";
  return `$${(Number(b) / 1e6).toLocaleString("en-US", { maximumFractionDigits: 2 })}`;
}

/* ------------------------------------------------------------------ locations */

export type Location =
  | { kind: "https"; url: string }
  | { kind: "near-tx"; tx: string; signer: string | null }
  | { kind: "other"; text: string };

/** A manifest or witness location: an https URL, or `near-tx:<hash>:<signer>` (an upload carried by a NEAR transaction). */
export function parseLocation(loc?: string | null): Location | null {
  if (!loc) return null;
  if (/^https:\/\//i.test(loc)) return { kind: "https", url: loc };
  const m = /^near-tx:([1-9A-HJ-NP-Za-km-z]{32,44})(?::([a-z0-9._-]+))?$/.exec(loc);
  if (m) return { kind: "near-tx", tx: m[1], signer: m[2] ?? null };
  return { kind: "other", text: loc };
}

/* ------------------------------------------------------------------ the rules' source */

export interface RuleSource {
  path: string;
  line: number;
  contract: string;
  /** The rule in plain words, from its doc comment in the Solidity source. */
  says: string;
}

const ACME_RULES = "contracts/evm/src/acme/AcmeInvariants.sol";
const PRACTICE_RULES = "contracts/evm/src/practice/Practice.sol";

const ACME: Record<string, RuleSource> = {
  solvency: { path: ACME_RULES, line: 13, contract: "AcmeInvariants", says: "Lender claims are covered at the committed reference price." },
  fees_to_treasury: { path: ACME_RULES, line: 16, contract: "AcmeInvariants", says: "Accrued fees plus the treasury balance never decrease." },
  share_price: { path: ACME_RULES, line: 18, contract: "AcmeInvariants", says: "The vault's share price never falls for the remaining holders." },
  protected_balances: {
    path: ACME_RULES,
    line: 75,
    contract: "AcmeInvariantsV2",
    says: "The protected saver's acUSD never falls in a transaction the saver did not send.",
  },
};

const PRACTICE: Record<string, RuleSource> = {
  vault_keeps_its_ether: { path: PRACTICE_RULES, line: 63, contract: "PracticeInvariants", says: "No practice vault loses its ether." },
};

/** Where a rule is written, for our own fixtures (a program by anyone else has no entry). */
export function ruleSource(kind: ProgramKind, name: string): RuleSource | null {
  if (kind === "fixture") return ACME[name] ?? null;
  if (kind === "practice" || kind === "fork") return PRACTICE[name] ?? null;
  return null;
}

/**
 * Acme's round 1 adds bit 3 (AcmeInvariantsV2, docs/rounds.md); the API names the round-0 slots only. The
 * commit and run pages name it from here too, so a round's slot reads the same on every page.
 */
export const ACME_ADDED: Record<number, string> = { 3: "protected_balances" };

export interface SlotView {
  bit: number;
  name: string | null;
  severity: string;
  payout_raw: string | null;
  budget_raw: string | null;
  /** Added by a round: the round-0 manifest does not name it. */
  added: boolean;
  pauses: boolean;
}

/** Every slot the program prices now: the manifest's (rules.slots), then any a round added (params.slots). */
export function slotsOf(d: ProgramDetail, kind: ProgramKind): SlotView[] {
  const params: Record<string, unknown> = d.program?.params ?? {};
  const budgets = Array.isArray(params.slots) ? (params.slots as { slot?: number; severity?: string; budget?: string }[]) : [];
  const pause = Array.isArray(params.pause_slots) ? (params.pause_slots as number[]) : [];
  const payouts = (d.rules?.payouts_raw ?? {}) as Partial<Record<string, string>>;
  const out: SlotView[] = (d.rules?.slots ?? []).map((s) => ({
    bit: s.bit,
    name: s.name ?? null,
    severity: s.severity,
    payout_raw: s.payout_raw ?? payouts[s.severity] ?? null,
    budget_raw: budgets.find((b) => b.slot === s.bit)?.budget ?? null,
    added: false,
    pauses: pause.includes(s.bit),
  }));
  for (const b of budgets) {
    if (typeof b.slot !== "number" || out.some((s) => s.bit === b.slot)) continue;
    const severity = b.severity ?? "low";
    out.push({
      bit: b.slot,
      name: kind === "fixture" ? (ACME_ADDED[b.slot] ?? null) : null,
      severity,
      payout_raw: payouts[severity] ?? null,
      budget_raw: b.budget ?? null,
      added: true,
      pauses: pause.includes(b.slot),
    });
  }
  return out.sort((a, b) => a.bit - b.bit);
}

export function slotName(slots: SlotView[], bit: number): string {
  const s = slots.find((x) => x.bit === bit);
  return s?.name ?? `slot ${bit}`;
}

/* ------------------------------------------------------------------ who */

/** The practice vending contract: it creates every practice account and receives every try's payout. */
const VENDING = "practice.antibody.testnet";

/** Our own practice accounts on this run's programs: the recorded run's scripted tries (JUDGES.md row 12), our rehearsal and our checks. */
const OUR_TRIES: Record<string, string> = {
  "judge-1-p6.practice.antibody.testnet": "the recorded run's practice try",
  "judge-2-p6.practice.antibody.testnet": "the recorded run's practice try",
  "pcs-rehearsal.practice.antibody.testnet": "our rehearsal",
  // Our checks of 2026-11-07 (docs/results/byo-2026-11-07.json and the try of the deployed site).
  "site-check.practice.antibody.testnet": "our check of the deployed site",
  "cli-check-t0.practice.antibody.testnet": "our check of the CLI",
  "byo-alpha.practice.antibody.testnet": "our bring-your-own-proof check",
  "byo-beta.practice.antibody.testnet": "our bring-your-own-proof check",
  "byo-wrong.practice.antibody.testnet": "our bring-your-own-proof check",
  "byo-invalid.practice.antibody.testnet": "our bring-your-own-proof check",
  "rel-linux-check.practice.antibody.testnet": "our check of the release binary",
  "scrn-k7q2-1107.practice.antibody.testnet": "our screener dry run",
};

/** A practice try (a practice account other than the vending contract). */
export const isPracticeTry = (account?: string | null) => provenance(account) === "practice" && account !== VENDING;

/**
 * Whose practice try: ours (the recorded run's, our rehearsal, or any try on an earlier run's program, 0–4),
 * else a visitor's.
 */
export function practiceTry(account: string, program?: number | null): string {
  return OUR_TRIES[account] ?? (typeof program === "number" && runGroup(program) === "earlier" ? "an earlier run's practice try" : "a visitor's practice try");
}

/** Our fixture agents, the practice tries and our own accounts, labelled wherever they appear. */
export function whoLabel(account?: string | null, program?: number | null): string | null {
  const a = account ?? "";
  const p = provenance(a);
  if (p === "fixture-agent") return `our fixture agent ${fixtureAgentLetter(a) ?? ""}`.trim();
  if (p === "practice") return a === VENDING ? "the practice vending contract" : practiceTry(a, program);
  if (p === "ours") return "ours";
  return null;
}

/**
 * Whether a program's feed is "the tries": its rows are practice tries, or, with none yet, it is the
 * program the vending contract sends tries to. Our fixture agent's commit in fork mode is not a try.
 */
export function feedIsTries(rows: FeedRow[], kind: ProgramKind, vending: boolean): boolean {
  if (rows.length) return rows.some((r) => isPracticeTry(r.hunter));
  return kind === "practice" && vending;
}

/** A short name for a feed row's hunter: "A" for our fixture agent A, else the account. */
export function hunterShort(account?: string | null): string {
  const a = account ?? "";
  if (provenance(a) === "fixture-agent") return fixtureAgentLetter(a) ?? a;
  return a || "—";
}

/* ------------------------------------------------------------------ events and keys */

export function findEvent(d: ProgramDetail, event: string, match?: (data: Record<string, unknown>) => boolean): IndexedEvent | undefined {
  return (d.events ?? []).find((e) => e.event === event && (!match || match(e.data ?? {})));
}

/** Every feed row whose verdict holds a tag, in commit order. */
export function rowsWithTag(rows: FeedRow[], tag: string): FeedRow[] {
  return rows.filter((r) => (r.verdict?.keys ?? []).some((k) => k.tag === tag)).sort((a, b) => a.index - b.index);
}

export const normHex = (h?: string | null) => (h ?? "").toLowerCase().replace(/^0x/, "");

/** A number field of the program record the API passes through (`activation_run`, `round_started_at`…). */
export function num(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

export function str(v: unknown): string | null {
  return typeof v === "string" && v ? v : null;
}

/* ------------------------------------------------------------------ the windows */

/** The program's windows (spec §3), each in plain words. All are seconds except the deadline. */
export const WINDOWS: { key: string; label: string; says: string }[] = [
  { key: "reveal", label: "Reveal", says: "A commit is revealed within this, or it expires and its bond goes to the escrow." },
  {
    key: "challenge",
    label: "Challenge window",
    says: "A verdict that is not paid instantly is final after this; anyone may challenge it within it, with the challenge bond.",
  },
  { key: "resolve", label: "Resolve", says: "A challenge with no attested result within this voids the verdict; both bonds are refunded." },
  { key: "run_timeout", label: "Run timeout", says: "A run that never answered is void after this." },
  { key: "infra_grace", label: "Infrastructure grace", says: "An infrastructure failure is retried this long, then the commit closes with a refund." },
  { key: "round_min", label: "Round minimum", says: "A round runs at least this long before the next one opens." },
  { key: "rerun_window", label: "Re-run window", says: "Re-runs at a proposed fix may be asked for within this of its confirmed check." },
  { key: "disclosure_max_delay", label: "Disclosure delay", says: "A paid proof becomes public at most this long after its claim." },
  { key: "exit_grace", label: "Exit grace", says: "After the deadline plus this, anyone may settle the program (exit)." },
];

/* ------------------------------------------------------------------ a proposal in flight */

export interface ProposalView {
  round: number | null;
  manifest_sha256: string | null;
  check_run: number | null;
  proposed_at: number | null;
  canaries_ok: boolean | null;
}

export function proposalOf(p: unknown): ProposalView | null {
  if (!p || typeof p !== "object") return null;
  const o = p as Record<string, unknown>;
  return {
    round: num(o.round),
    manifest_sha256: str(o.manifest_sha256),
    check_run: num(o.check_run),
    proposed_at: num(o.proposed_at),
    canaries_ok: typeof o.canaries_ok === "boolean" ? o.canaries_ok : null,
  };
}
