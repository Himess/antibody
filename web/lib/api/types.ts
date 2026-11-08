/**
 * The Antibody API's payloads (docs/api.md, docs/openapi.yaml), as the site reads them. Amounts are raw
 * token units as decimal strings (`*_raw`, wNEAR: 24 decimals). Fields the API may omit are optional.
 */

export type Raw = string;

export interface IndexedEvent {
  seq: number;
  standard: string;
  event: string;
  data: Record<string, unknown>;
  tx: string;
  explorer?: string;
  block_height: number;
  block_ts_ns: number;
  signer?: string;
  receipt?: string;
  network?: string;
}

export interface Health {
  ok: boolean;
  network: string;
  version: string;
  endpoints: string[];
  contracts: {
    antibody: string;
    token: string;
    token_decimals?: number;
    token_symbol?: string;
    guardian?: string;
    outlayer?: string;
    light_client?: string;
    verdict_guard?: string;
    t2?: string;
    practice?: { program: number; vending: string };
    release?: {
      contracts: { account: string; code_hash: string; locked: boolean; role: string }[];
      git_commit?: string;
      verifier?: { wasm_hash: string; wasm_url?: string };
      recorded_at?: number;
    };
  };
}

export interface Ledger {
  available: Raw;
  challenge_bonds: Raw;
  deposits: Raw;
  fees: Raw;
  forfeits: Raw;
  paid: Raw;
  reserved: Raw;
  returned: Raw;
}

export interface ProgramSummary {
  program_id: number;
  name: string;
  project: string;
  phase: string;
  round: number;
  anchor: { chain_id: number; block: number };
  commits: number;
  ledger: Ledger;
}

export type Severity = "critical" | "high" | "medium" | "low";

export interface Slot {
  bit: number;
  name: string;
  severity: Severity;
  payout_raw: Raw;
}

export interface Rules {
  slots: Slot[];
  payouts_raw: Record<Severity, Raw>;
  bond_raw: Raw;
  instant: { cap_raw: Raw; budget_raw: Raw; window_secs: number };
  escrow_coverage: number | null;
  known_issues_sealed: number;
  anchor: { chain_id: number; block: number; hash: string; check: string };
}

export interface VerdictKey {
  slot: number;
  tag: string;
}

export interface T2Badge {
  outcome: "confirmed" | "contradicts" | string;
  differ?: string[] | null;
  receipt_sha256?: string;
  at?: number;
  block?: number;
}

export interface FeedRow {
  commit_id: number;
  program_id: number;
  index: number;
  round: number;
  hunter: string;
  beneficiary: string;
  committed_at: number;
  state: string;
  run_id: number | null;
  verdict: { kind: string | null; tier: string | null; keys: VerdictKey[] | null; t2: T2Badge | null };
  paid_raw: Raw;
  sealed: boolean;
}

export interface KnownIssue {
  fixed: boolean;
  impact_usd_micro: string;
  run: number;
  severity: Severity;
  slots: number[];
  tags: string[];
}

export interface Round {
  round: number;
  manifest_sha256: string;
  opened_at: number;
  release_gate: boolean;
  released: string[];
  carried: string[];
  promoted: number[];
}

export interface ProgramDetail {
  program: {
    id: number;
    key: string;
    project: string;
    phase: string;
    round: number;
    ledger: Ledger;
    known: KnownIssue[];
    rounds: Round[];
    params: Record<string, unknown> & {
      windows?: Record<string, number>;
      decimals?: number;
      protocol_fee_bps?: number;
      release_fee?: Raw;
      run_deposit?: Raw;
    };
    manifest_sha256?: string;
    sealing_pk?: string;
    key_epoch?: string;
    created_at?: number;
    opened_at?: number;
    commits?: number;
    proposal?: unknown;
    release_permits?: unknown;
    [k: string]: unknown;
  };
  rules: Rules;
  meta: {
    name: string;
    description: string;
    chain_id: number;
    anchor_block: number;
    anchor_hash: string;
    manifest_url?: string;
    base?: { location: string; sha256: string } | null;
    project_pk?: string;
    slots: [number, string, Severity][];
    targets: [string, string][];
    terms_sha256: string;
    round_releases?: { proxy: string; implementation: string; code_hash: string }[][];
    round_manifests?: { location: string; sha256: string }[];
    [k: string]: unknown;
  };
  record: {
    anchor: string;
    at_stake: Raw;
    defaulted: Raw;
    paid_findings: number;
    phase: string;
    program_id: number;
    release_permits: number;
    rounds: number;
    share_claimed_first_hour_bps: number;
    survived_secs: number;
    time_to_first_claim_secs: number | null;
    verified_attempts: number;
  };
  audit: {
    ok: boolean;
    funded: Raw;
    accounted: Raw;
    available: Raw;
    reserved: Raw;
    paid: Raw;
    fees: Raw;
    returned: Raw;
    debt: Raw;
    defaulted: Raw;
    reservations: Raw;
  };
  commits: FeedRow[];
  events: IndexedEvent[];
}

export interface ReflexPermit {
  permit_id: number;
  round: number;
  body: { Pause?: { ticket: number }; Release?: { proxy: string; code_hash: string } };
  status: string | null;
  tx: string | null;
  explorer: string | null;
  source?: string | null;
  signed_ns?: number;
  deadline?: number;
  digest?: string;
}

export interface Reflex {
  program_id: number;
  guardian_address: string | null;
  chains: {
    chain_id: number;
    adapter: string;
    proxies: string[];
    state: "live" | "paused" | "released";
    permits: ReflexPermit[];
  }[];
}

export interface RunView {
  run_id: number;
  commit_id?: number | null;
  mode: string;
  kind: string | null;
  tier: string | null;
  keys?: VerdictKey[];
  impact_usd_micro?: string;
  protected_harm_usd_micro?: string;
  receipt_sha256?: string | null;
  reason?: string | null;
  answered_at?: number;
  failed?: unknown;
}

export interface T2Record {
  run_id?: string | number;
  outcome: string;
  differ?: string[] | null;
  receipt_sha256?: string;
  at?: number;
  block?: number;
  [k: string]: unknown;
}

export interface RunDetail {
  run: RunView;
  tier: string;
  t2: T2Record | null;
  raw: {
    id: number;
    commit: number | null;
    mode: string;
    program: number;
    round: number;
    payer: string;
    signer: string;
    request_sha256: string;
    started_at: number;
    outcome: unknown;
  };
}

export interface CommitDetail {
  commit: {
    id: number;
    program: number;
    round: number;
    index: number;
    hunter: string;
    beneficiary: string;
    commitment: string;
    blob_sha256: string | null;
    bond: Raw;
    committed_at: number;
    run: number | null;
    rerun?: number | null;
    second?: number | null;
    wrapped?: number | null;
    attempts?: number;
    ticket?: boolean;
    state: unknown;
    challenge?: unknown;
    retry_until?: number;
  };
  row: FeedRow;
  events: IndexedEvent[];
  proof_card: {
    facts: { kind: string | null; keys: VerdictKey[] | null; impact_usd_micro: string | null; tier: string | null };
    receipt_sha256: string | null;
    sealed_until_disclosure: boolean;
    check: string[];
  };
}

/** An agent account's terms and statement (near-antibody's get_account `agent`, and agent_statement). */
export interface AgentTerms {
  owner: string;
  bond_budget: Raw;
  per_commit_cap: Raw;
  bonds_locked: Raw;
  bonds_lost: Raw;
  bonds_returned: Raw;
  paid: Raw;
  commits: number;
  containment_events: number;
}

export interface Account {
  account: string;
  /** get_account: token credit, NEAR credit (yocto), whether it ever committed, and its agent terms. */
  credit: {
    tokens: Raw;
    near: Raw;
    committed?: boolean;
    agent: (AgentTerms & { verification_spent?: Raw; programs: number[] | null }) | null;
  } | null;
  /** agent_statement: `registered: false` for an account that is no agent. */
  agent:
    | ({ agent: string; registered: true; verification_spent_yocto?: Raw; net_tokens?: Raw; key_permissions?: unknown } & AgentTerms)
    | { agent: string; registered: false }
    | null;
  token_balance_raw: Raw | null;
}

export interface LeaderRow {
  account: string;
  paid_raw: Raw;
  paid_findings: number;
  duplicates: number;
  forfeited: number;
}

export interface Stats {
  contract: string;
  stats: {
    next_commit: number;
    next_program: number;
    next_run: number;
    totals: Record<string, Raw | number>;
  };
  latency: {
    events: number;
    submit_to_verdict_s: { n: number; p50: number | null; p95: number | null };
    submit_to_payout_s: { n: number; p50: number | null; p95: number | null };
    submit_to_verdict_blocks?: { p50: number | null; p95: number | null };
  };
  verdict_latency?: {
    commit_to_answer_s: { n: number; p50: number | null; p95: number | null };
    commit_to_answer_blocks: { p50: number | null; p95: number | null };
    note?: string;
  };
}

export interface Alert {
  level: "info" | "warn" | "critical";
  component: string;
  key: string;
  message: string;
}

export interface Status {
  at: number;
  ok: boolean;
  network: string;
  alerts: Alert[];
  components: Record<string, unknown>;
}

export interface TrustActor {
  actor: string;
  trusted_for: string;
  account: string | null;
  check: string;
}

export interface Practice {
  vending: string;
  program_id: number;
  vending_balance_yocto: string | null;
  public_vend_key: string;
  steps: string[];
  docs: string;
}

export interface MomentStep {
  key: string;
  label: string;
  seconds: number;
  seconds_exact: number;
  chain: string;
  tx: string | null;
  explorer: string | null;
  detail: Record<string, unknown> & { minor?: boolean; by?: string };
}

export interface Moment {
  id: string;
  kind: "recorded" | "drill" | string;
  network: string;
  all_ok: boolean;
  results_file: string;
  started_unix: number;
  program_id: number;
  program_key: string;
  commit_id: number;
  run_id: number | null;
  hunter: string | null;
  via: string | null;
  verdict: { kind: string; slots: number[]; tier: string };
  paid_raw: Raw;
  instant: boolean;
  paused_everywhere: boolean;
  commit: { tx: string | null; explorer: string | null; block_height: number | null; block_ts_ns: number; at_utc: string | null };
  clock: string;
  notes: string[];
  /** What is true of this run only (a drill whose reveal came late), shown with its times. */
  run_notes?: string[];
  steps: MomentStep[];
}

export interface Moments {
  moments: Moment[];
  latest: string | null;
  note: string;
}

export interface Mainnet {
  title: string;
  same: string[];
  changes: { part: string; testnet: string; mainnet: string }[];
  latency: string;
  status: string;
  light_client_check: Record<string, unknown>;
  transactions_sent: number;
}

export interface T2Panel {
  contract: string;
  pins: Record<string, unknown>;
  stats: Record<string, unknown>;
  records: T2Record[];
  events: IndexedEvent[];
  prover: Record<string, unknown> | null;
  note: string;
}
