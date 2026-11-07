/**
 * An account at near-antibody as its views return it (contracts/near-antibody/src/lib.rs): `get_account`
 * (the credit, with the agent record) and `agent_statement`, plus the wallet's wNEAR balance
 * (`ft_balance_of` on wrap.testnet). /v1/accounts/{id} returns the same three views. Every field is
 * optional here: a page never crashes on a field the API leaves out.
 */
import type { FeedRow } from "@/lib/api/types";

/** `get_account(account_id).agent`: the agent record, with the programs it is bound to. */
export interface AgentRecord {
  owner?: string;
  bond_budget?: string;
  per_commit_cap?: string;
  bonds_locked?: string;
  bonds_lost?: string;
  bonds_returned?: string;
  paid?: string;
  verification_spent?: string;
  commits?: number;
  containment_events?: number;
  /** null: any program. */
  programs?: number[] | null;
}

/** `get_account(account_id)`: the credit held for an account. */
export interface Credit {
  /** Token credit (wNEAR raw, 24 decimals): payouts, refunds, deposits; bonds are taken from it. */
  tokens: string | null;
  /** NEAR credit (yoctoNEAR): storage and verification runs are paid from it. */
  near: string | null;
  /** The account committed as its own beneficiary, so it can never become an agent. */
  committed: boolean;
  agent: AgentRecord | null;
}

/** `agent_statement(agent)`: `{agent, registered: false}` for an account that is not an agent. */
export interface AgentStatement {
  agent?: string;
  registered: boolean;
  owner?: string;
  bond_budget?: string;
  per_commit_cap?: string;
  bonds_locked?: string;
  bonds_lost?: string;
  bonds_returned?: string;
  paid?: string;
  verification_spent_yocto?: string;
  net_tokens?: string;
  commits?: number;
  containment_events?: number;
  key_permissions?: string;
}

export interface AccountData {
  account: string;
  credit: Credit | null;
  statement: AgentStatement | null;
  /** The account's wNEAR on wrap.testnet (raw). */
  tokenBalance: string | null;
  /** Where the numbers came from: the API (cached for a few seconds) or the browser's own view calls. */
  source: "api" | "near";
  at: number;
  /** Set when the API failed and the views were read from NEAR instead. */
  apiError?: string;
}

/** A commit of the account (as hunter or as beneficiary), from a program's feed. */
export interface ActivityRow extends FeedRow {
  programName: string;
}

export interface Activity {
  rows: ActivityRow[];
  programs: { id: number; name: string; project: string; phase: string }[];
  /** Programs whose feed could not be read. */
  failed: number[];
}

export interface AgentEntry {
  agent: string;
  bondBudget: string | null;
  perCommitCap: string | null;
  programs: number[] | null;
  tx: string | null;
  at: number | null;
}

export interface Withdrawal {
  amount: string | null;
  tx: string | null;
  at: number | null;
}

export interface History {
  agents: AgentEntry[];
  withdrawals: Withdrawal[];
  /** True when the event log was longer than the pages read. */
  truncated: boolean;
}

export interface ContractState {
  account: string;
  role: string;
  expected: string;
  actual: string | null;
  matches: boolean | null;
  /** Full-access keys on the account (0: nobody can change its code). Only read for near-antibody. */
  fullAccessKeys: number | null;
  error?: string;
}
