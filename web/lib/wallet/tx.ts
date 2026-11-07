/**
 * The transactions this page can ask a wallet to sign, as the contracts define them
 * (contracts/near-antibody/src/lib.rs) and as the CLI sends them (crates/cli/src/cmd.rs): the same methods,
 * arguments, deposits and gas. Each plan says in plain words what every transaction does, and gives the CLI
 * command that does the same.
 */
import { formatUnits } from "./amounts";
import { ANTIBODY, DEPOSIT_MSG, GAS, MIN_NEAR_CREDIT, ONE_YOCTO, WNEAR } from "./contracts";

export interface FunctionCall {
  method: string;
  args: Record<string, unknown>;
  gas: bigint;
  /** Attached yoctoNEAR. */
  deposit: bigint;
}

export interface PlannedTx {
  receiver: string;
  calls: FunctionCall[];
  /** What this transaction does, in plain words. */
  says: string;
  /** What a success is checked against once it lands. */
  expect?: "deposit" | "withdraw" | "agent" | "plain";
  /** The raw amount a token transfer moves (for the result check). */
  amount?: bigint;
}

export interface Plan {
  title: string;
  txs: PlannedTx[];
  notes: string[];
  /** The CLI command that does the same, for anyone who prefers a terminal. */
  cli: string;
}

const ZERO = BigInt(0);
const w = (raw: bigint) => `${formatUnits(raw)} wNEAR`;
const n = (yocto: bigint) => `${formatUnits(yocto)} NEAR`;

/** The connector's action format (a FunctionCall with string amounts). */
export function toConnectorActions(tx: PlannedTx) {
  return tx.calls.map((c) => ({
    type: "FunctionCall" as const,
    params: { methodName: c.method, args: c.args, gas: c.gas.toString(), deposit: c.deposit.toString() },
  }));
}

// ------------------------------------------------------------------ single transactions

export function addNearCredit(yocto: bigint): PlannedTx {
  return {
    receiver: ANTIBODY,
    calls: [{ method: "storage_deposit", args: {}, gas: GAS.storageDeposit, deposit: yocto }],
    says: `Adds ${n(yocto)} to your NEAR credit at near-antibody. Storage and verification runs are paid from it.`,
    expect: "plain",
  };
}

/** wrap.testnet's NEP-145 registration: an account must hold one to receive wNEAR. */
function registerCall(account: string, min: bigint): FunctionCall {
  return { method: "storage_deposit", args: { account_id: account, registration_only: true }, gas: GAS.storageDeposit, deposit: min };
}

export function wrapNear(account: string, yocto: bigint, registration: bigint | null): PlannedTx {
  const calls: FunctionCall[] = [];
  if (registration !== null) calls.push(registerCall(account, registration));
  calls.push({ method: "near_deposit", args: {}, gas: GAS.nearDeposit, deposit: yocto });
  return {
    receiver: WNEAR,
    calls,
    says:
      (registration !== null ? `Registers your account with wrap.testnet (${n(registration)}, once), then wraps ` : "Wraps ") +
      `${n(yocto)} into ${w(yocto)} in your wallet.`,
    expect: "plain",
  };
}

export function registerWithToken(account: string, min: bigint): PlannedTx {
  return {
    receiver: WNEAR,
    calls: [registerCall(account, min)],
    says: `Registers your account with wrap.testnet (${n(min)}, once), so the withdrawn wNEAR can reach it.`,
    expect: "plain",
  };
}

export function depositTokens(raw: bigint): PlannedTx {
  return {
    receiver: WNEAR,
    calls: [{ method: "ft_transfer_call", args: { receiver_id: ANTIBODY, amount: raw.toString(), msg: DEPOSIT_MSG }, gas: GAS.ftTransferCall, deposit: ONE_YOCTO }],
    says: `Moves ${w(raw)} from your wallet into your token credit at near-antibody. Your bonds, and your agents' bonds, are taken from it.`,
    expect: "deposit",
    amount: raw,
  };
}

export function withdrawTokens(raw: bigint): PlannedTx {
  return {
    receiver: ANTIBODY,
    calls: [{ method: "withdraw", args: { amount: raw.toString() }, gas: GAS.withdraw, deposit: ONE_YOCTO }],
    says: `Withdraws ${w(raw)} of token credit to your wallet on wrap.testnet.`,
    expect: "withdraw",
    amount: raw,
  };
}

export function withdrawNearCredit(yocto: bigint): PlannedTx {
  return {
    receiver: ANTIBODY,
    calls: [{ method: "storage_withdraw", args: { amount: yocto.toString() }, gas: GAS.storageWithdraw, deposit: ONE_YOCTO }],
    says: `Returns ${n(yocto)} of NEAR credit to your account.`,
    expect: "plain",
  };
}

export function registerAgentTx(agent: string, budget: bigint, cap: bigint, programs: number[] | null): PlannedTx {
  return {
    receiver: ANTIBODY,
    calls: [
      {
        method: "register_agent",
        args: { agent, bond_budget: budget.toString(), per_commit_cap: cap.toString(), programs },
        gas: GAS.registerAgent,
        deposit: ONE_YOCTO,
      },
    ],
    says:
      `Makes ${agent} your agent: its bonds come from your token credit, at most ${w(cap)} per commit and ${w(budget)} locked plus lost in all, ` +
      `${programs ? `on program${programs.length === 1 ? "" : "s"} ${programs.join(", ")} only` : "on any program"}. Its payouts go to you.`,
    expect: "agent",
  };
}

// ------------------------------------------------------------------ plans

/** The NEAR credit top-up the CLI makes first (`ensure_near_credit`), or none. */
function topUp(nearCredit: bigint): PlannedTx | null {
  return nearCredit < MIN_NEAR_CREDIT ? addNearCredit(MIN_NEAR_CREDIT - nearCredit) : null;
}

export function planAddNearCredit(account: string, yocto: bigint): Plan {
  return {
    title: "Add NEAR credit",
    txs: [addNearCredit(yocto)],
    notes: ["Unused NEAR credit stays yours: Withdraw NEAR credit returns it."],
    cli: `antibody --account ${account} storage-deposit --near ${formatUnits(yocto)}`,
  };
}

export function planDeposit(o: {
  account: string;
  amount: bigint;
  walletTokens: bigint;
  nearCredit: bigint;
  registered: boolean;
  registration: bigint;
  wrapMissing: boolean;
}): Plan | { error: string } {
  const txs: PlannedTx[] = [];
  const notes: string[] = [];
  const missing = o.amount > o.walletTokens ? o.amount - o.walletTokens : ZERO;
  if (missing > ZERO) {
    if (!o.wrapMissing) return { error: `Your wallet holds ${w(o.walletTokens)}. Lower the amount, or let the page wrap the missing ${n(missing)} first.` };
    txs.push(wrapNear(o.account, missing, o.registered ? null : o.registration));
  }
  const t = topUp(o.nearCredit);
  if (t) {
    txs.push(t);
    notes.push("Your NEAR credit is below 0.25 NEAR, the amount the CLI keeps before a deposit: the page tops it up first, as the CLI's deposit command does.");
  }
  txs.push(depositTokens(o.amount));
  notes.push("If near-antibody refuses the tokens, wrap.testnet returns them to your wallet in the same transaction.");
  return { title: "Deposit wNEAR into your credit", txs, notes, cli: `antibody --account ${o.account} deposit --amount ${formatUnits(o.amount)}` };
}

export function planWithdraw(o: { account: string; amount: bigint; registered: boolean; registration: bigint }): Plan {
  const txs: PlannedTx[] = [];
  const notes: string[] = [];
  if (!o.registered) {
    txs.push(registerWithToken(o.account, o.registration));
    notes.push("Your account holds no wrap.testnet registration yet; without one the transfer would fail and near-antibody would credit the amount back.");
  }
  txs.push(withdrawTokens(o.amount));
  notes.push("One yoctoNEAR is attached: only a full-access key can sign it, so an agent's key never can.");
  return { title: "Withdraw wNEAR credit", txs, notes, cli: `antibody --account ${o.account} withdraw --amount ${formatUnits(o.amount)}` };
}

export function planWithdrawNear(account: string, yocto: bigint): Plan {
  return {
    title: "Withdraw NEAR credit",
    txs: [withdrawNearCredit(yocto)],
    notes: [
      "Commits and verification runs are paid from this credit: keep some if you will submit.",
      "One yoctoNEAR is attached: only a full-access key can sign it.",
    ],
    cli: `antibody --account ${account} admin call --contract ${ANTIBODY} --method storage_withdraw --args '{"amount":"${yocto.toString()}"}' --deposit-yocto 1 --gas-tgas 30`,
  };
}

export function planRegisterAgent(o: {
  owner: string;
  agent: string;
  budget: bigint;
  cap: bigint;
  programs: number[] | null;
  nearCredit: bigint;
  update: boolean;
}): Plan {
  const txs: PlannedTx[] = [];
  const notes: string[] = [];
  const t = topUp(o.nearCredit);
  if (t) {
    txs.push(t);
    notes.push("The agent's entry is stored at your expense, from your NEAR credit: the page tops it up to 0.25 NEAR first, as the CLI's agent register command does.");
  }
  txs.push(registerAgentTx(o.agent, o.budget, o.cap, o.programs));
  notes.push("One yoctoNEAR is attached: only your full-access key can grant an agent a budget.");
  if (o.update) notes.push("The agent is already yours: this replaces its budget, cap and programs, and keeps its record.");
  const progs = o.programs ? o.programs.map((p) => ` --program ${p}`).join("") : "";
  return {
    title: o.update ? "Update your agent" : "Register an agent",
    txs,
    notes,
    cli: `antibody --account ${o.owner} agent register --agent ${o.agent} --budget ${formatUnits(o.budget)} --per-commit-cap ${formatUnits(o.cap)}${progs}`,
  };
}

/** Total yoctoNEAR a plan attaches (for the summary). */
export function attachedTotal(plan: Plan): bigint {
  return plan.txs.reduce((s, t) => s + t.calls.reduce((a, c) => a + c.deposit, ZERO), ZERO);
}
