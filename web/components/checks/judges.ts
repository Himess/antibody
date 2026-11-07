import raw from "@/data/judges.json";
import type { CheckSpec, Expect } from "@/components/ui/LiveCheck";

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
}

export const judges = raw as unknown as JudgesData;

/** LiveCheck's spec from a generated check (the extra fields stay out). */
export function specOf(c: JudgeCheck): CheckSpec {
  if (c.kind === "api") return { kind: "api", path: c.path };
  if (c.kind === "near-view") return { kind: "near-view", contract: c.contract, method: c.method, args: c.args ?? {} };
  return { kind: "evm", chainId: c.chainId, method: c.method, params: c.params };
}

export { safeUrl } from "./url";

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
