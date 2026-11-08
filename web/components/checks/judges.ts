import raw from "@/data/judges.json";
import type { CheckSpec, Expect } from "@/components/ui/LiveCheck";
import { chainName } from "@/lib/config";

/**
 * The site's copy of JUDGES.md (web/data/judges.json), written by scripts/docs/judges.py from the same
 * results file and the same values, so the claims here and in JUDGES.md say the same thing. Regenerate:
 * `python3 scripts/docs/judges.py docs/results/<run>.json`.
 */

/** A check the browser runs (LiveCheck's spec), with what the claim expects and the terminal command. */
export type JudgeCheck = CheckSpec & { expect?: Expect; label?: string; cli?: string };

export interface JudgeLink {
  label: string;
  /** An explorer page (NEAR or an EVM chain). */
  url?: string | null;
  tx?: string;
  chain?: string;
  chain_id?: number;
  /** A file in the repository. */
  path?: string;
}

export interface JudgeClaim {
  n: number;
  kind: string;
  /** The Markdown cells of JUDGES.md's claims table, verbatim. */
  claim: string;
  check: string;
  expected: string;
  evidence: string;
  /** The commands the check names, runnable as they are. */
  commands: string[];
  links: JudgeLink[];
  live: JudgeCheck[];
  commit?: number | null;
  run?: number | null;
  program?: number | null;
  state?: string;
  tier?: string;
  agent?: string;
  /** What the claim does not show, in one line, once the generator writes it. */
  does_not_show?: string | null;
}

export interface JudgeEvmCheck {
  claim: number | null;
  label: string;
  chain_id: number;
  subject?: string;
  address: string;
  expected_suffix?: string;
  expected_result?: string;
  rpc: string;
  method: string;
  params: unknown[];
  curl: string;
  live: boolean;
  expect?: Expect;
}

export interface JudgeProgram {
  program_id: number;
  name: string;
  phase: string | null;
  anchor: string | null;
  t1_confirmed: number | null;
  runs: number | null;
  payouts: number;
  pauses: number | null;
  releases: number | null;
  audit_ok: boolean | null;
  /** This run's role for the program (acme, practice, fork); null for an earlier run's. */
  this_run: string | null;
  each_try: boolean;
  line: string;
}

export interface JudgesData {
  schema: number;
  generated_by: string;
  source: string;
  network: string;
  started_unix: number | null;
  run_utc: string;
  all_ok: boolean;
  title: string;
  generated: string;
  intro: string[];
  site: string | null;
  recorded_cmd: string;
  deployment: {
    near_antibody?: string | null;
    guardian?: string | null;
    token?: string | null;
    light_client?: string | null;
    outlayer?: string | null;
    verifier?: { wasm_hash: string; wasm_url?: string } | null;
    contracts?: { account: string; code_hash?: string; locked?: boolean; role: string }[];
    recorded_at?: number | null;
  };
  chains: { verifies_and_pauses: number; pauses: number };
  anchor: { block?: number; hash?: string; chain_id?: number; light_client?: string | null; confirmed_unix?: number | null; timestamp?: number };
  programs: Record<string, number>;
  one_moment: {
    commit?: number | null;
    seconds_from_commit?: Record<string, number>;
    paid_raw?: string | null;
    instant?: boolean | null;
    paused_both?: boolean | null;
  };
  one_look: {
    intro: string;
    setup: string[];
    command: string;
    expected: string | null;
    programs_note: string | null;
    contracts: { role: string; account: string; locked: boolean | null }[];
    audit_ok: boolean | null;
    programs: JudgeProgram[];
    live: JudgeCheck[];
  };
  claims: JudgeClaim[];
  evm_checks: JudgeEvmCheck[];
  transactions: { intro: string; items: { what: string; tx: string; signer: string; command: string; url: string | null }[] };
  timings: string[];
  does_not_show: string[];
  limits: { source: string; anchor: string; items: { title: string; text: string }[] };
  /** The newest measured agent runs (docs/agent-runs/<net>-<date>/task-*), when there are any. */
  agent_runs?: {
    dir: string;
    /** The release whose binary was the agent's MCP server (for example "v0.5.0"). */
    binary?: string | null;
    tasks: {
      task: string;
      line: string;
      runs: number;
      succeeded: number;
      paid_on_chain: number | null;
      seconds: { median: number | null; min: number | null; max: number | null };
      cost_usd_total: number;
      models: string[];
      tools: string;
      report: string;
      commits: number[];
    }[];
  };
}

export const judges = raw as unknown as JudgesData;

/** LiveCheck's spec from a generated check (the extra fields stay out). */
export function specOf(c: JudgeCheck): CheckSpec {
  if (c.kind === "api") return { kind: "api", path: c.path };
  if (c.kind === "near-view") return { kind: "near-view", contract: c.contract, method: c.method, args: c.args ?? {} };
  return { kind: "evm", chainId: c.chainId, method: c.method, params: c.params };
}

export { safeUrl } from "./url";

/** One browser check of a claim, with a stable id: the claims index and the claim's card share its answer. */
export interface ClaimCheck {
  id: string;
  check: CheckSpec;
  expect?: Expect;
  cli?: string;
  label?: string;
  /** An EVM read: what it reads ("AcmeLend", "practice vault"). */
  subject?: string;
}

/** An EVM check as a claim's check: read from the chain's public RPC, with its curl command. */
export function evmCheck(e: JudgeEvmCheck, id: string): ClaimCheck {
  return {
    id,
    check: { kind: "evm", chainId: e.chain_id, method: e.method, params: e.params },
    expect: e.expect,
    cli: e.curl,
    label: `Read it on ${chainName(e.chain_id)}`,
    subject: e.subject,
  };
}

/**
 * The checks a claim's result stands on: its first live check, or, when its check is on the EVM chains,
 * every EVM read that backs it.
 */
export function primaryChecks(c: JudgeClaim, evm: JudgeEvmCheck[]): ClaimCheck[] {
  const first = c.live?.[0];
  if (first) return [{ id: `${c.n}`, check: specOf(first), expect: first.expect, cli: first.cli, label: first.label }];
  return evm.filter((e) => e.claim === c.n && e.live).map((e, i) => evmCheck(e, `${c.n}.evm.${i}`));
}

/**
 * README's limitations that bound what a claim shows, by the claim's kind (matched by the limit's own
 * opening words, so a reworded limit drops out rather than points at the wrong one).
 */
const LIMITS_BY_KIND: Record<string, string[]> = {
  submission: ["TEE trust is tiered", "Worker images", "One block"],
  one_moment: ["Ethereum L1 for verification", "Worker images"],
  round: ["Duplicates are judged by"],
  release: ["Ethereum L1 for verification"],
  practice: ["Claims are paid in commit order", "Worker images"],
  rule_strength: ["Rule strength is the rules"],
};

/** The limits that bound a claim, each with its index in the Limits list (for its link). */
export function limitsFor(c: JudgeClaim, items: { title: string; text: string }[]): { index: number; label: string; text: string }[] {
  const wanted = [...(LIMITS_BY_KIND[c.kind] ?? []), ...(c.state === "Done:DuplicateConfirmed" ? ["Duplicates are judged by"] : [])];
  const out: { index: number; label: string; text: string }[] = [];
  for (const start of wanted) {
    const index = items.findIndex((l) => (l.title ?? "").startsWith(start));
    if (index < 0 || out.some((o) => o.index === index)) continue;
    const title = (items[index].title ?? "").trim();
    const whole = `${title}${/^[,.;:]/.test(items[index].text ?? "") ? "" : " "}${items[index].text ?? ""}`.trim();
    // A title cut mid-sentence ("Rule strength is the rules' measure,") reads with its text.
    out.push({ index, label: title.endsWith(".") ? title : whole, text: whole });
  }
  return out;
}

/** "1 h 16 min 30 s" for a whole number of seconds (a minute or more). */
export function longDuration(s: number): string {
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = Math.round(s % 60);
  return [h ? `${h} h` : "", m ? `${m} min` : "", sec ? `${sec} s` : ""].filter(Boolean).join(" ");
}

/** This run's programs, by id: "Acme Lend", "practice", "fork mode". */
export function runRole(role: string | null | undefined): string | null {
  if (role === "acme") return "Acme Lend";
  if (role === "practice") return "practice";
  if (role === "fork") return "fork mode";
  return null;
}

/** From A's commit block: the verdict, T1, the payment and the last chain's pause, in seconds. */
export function momentTimes(j: JudgesData) {
  const s = j.one_moment?.seconds_from_commit ?? {};
  const pauses = Object.entries(s)
    .filter(([k]) => k.startsWith("paused_"))
    .map(([, v]) => v)
    .filter((v) => Number.isFinite(v));
  return {
    verdict: Number.isFinite(s.verdict) ? s.verdict : null,
    t1: Number.isFinite(s.t1) ? s.t1 : null,
    paid: Number.isFinite(s.paid) ? s.paid : null,
    lastPause: pauses.length ? Math.max(...pauses) : null,
  };
}
