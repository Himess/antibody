"use client";

import { useEffect, useId, useState } from "react";
import { Command } from "@/components/ui/Command";
import { Icon } from "@/components/ui/Icon";
import { Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { Empty, ErrorState, Loading } from "@/components/ui/States";
import { AccountLink, TxLink } from "@/components/ui/TxLink";
import { Time } from "@/components/ui/Time";
import { isImplicit, loadAccount, loadNearBalance, validAccountId } from "@/lib/wallet/account";
import { parseUnits, rawOf } from "@/lib/wallet/amounts";
import type { Session } from "@/lib/wallet/connector";
import { MAX_AMOUNT } from "@/lib/wallet/contracts";
import { planRegisterAgent } from "@/lib/wallet/tx";
import type { AccountData, Activity, AgentEntry, AgentStatement, History } from "@/lib/wallet/types";
import type { ApiContracts } from "./AccountApp";
import { blockedText } from "./Actions";
import type { Blocked } from "./AccountOverview";
import { ProgramList } from "./AgentStatementView";
import { Amount, type Load } from "./shared";
import { TxResults, TxReview, useTxFlow } from "./TxFlow";
import styles from "./wallet.module.css";

/** One agent this account registered: what the event recorded, and its statement now. */
function AgentCard({ entry }: { entry: AgentEntry }) {
  const [now, setNow] = useState<Load<AccountData>>({ status: "loading" });
  useEffect(() => {
    let live = true;
    loadAccount(entry.agent).then((r) => {
      if (live) setNow(r.ok ? { status: "ok", data: r.data } : { status: "error", error: r.error });
    });
    return () => {
      live = false;
    };
  }, [entry.agent]);
  const s: AgentStatement | null = now.status === "ok" ? now.data.statement : null;
  const programs = now.status === "ok" ? (now.data.credit?.agent?.programs ?? entry.programs) : entry.programs;
  return (
    <li className={`card ${styles.agentCard}`}>
      <div className="row" style={{ justifyContent: "space-between" }}>
        <AccountLink account={entry.agent} />
        {s && !s.registered ? <Pill tone="warn" size="sm">no longer an agent</Pill> : s ? <Pill tone="ok" size="sm">agent</Pill> : null}
      </div>
      <dl className={`kv ${styles.kvTight}`}>
        <dt>Budget</dt>
        <dd>
          <Amount raw={s?.bond_budget ?? entry.bondBudget} />
        </dd>
        <dt>Per-commit cap</dt>
        <dd>
          <Amount raw={s?.per_commit_cap ?? entry.perCommitCap} />
        </dd>
        <dt>Bound to</dt>
        <dd>
          <ProgramList programs={programs} />
        </dd>
        {s?.registered ? (
          <>
            <dt>Bonds locked / lost</dt>
            <dd>
              <Amount raw={s.bonds_locked} /> / <Amount raw={s.bonds_lost} />
            </dd>
            <dt>Paid to you</dt>
            <dd>
              <Amount raw={s.paid} />
            </dd>
            <dt>Commits</dt>
            <dd className="num">
              {s.commits ?? 0}
              {s.containment_events ? (
                <>
                  {" "}
                  <Pill tone="warn" size="sm">
                    {s.containment_events} refused
                  </Pill>
                </>
              ) : null}
            </dd>
          </>
        ) : null}
        <dt>Registered</dt>
        <dd>
          {entry.tx ? <TxLink hash={entry.tx} /> : <span className="muted">—</span>}
          {entry.at ? (
            <span className="xsmall muted">
              {" "}
              <Time t={entry.at} unit="ns" showRelative={false} />
            </span>
          ) : null}
        </dd>
      </dl>
      {now.status === "error" ? <p className="xsmall muted">Its statement could not be read now ({now.error}).</p> : null}
    </li>
  );
}

/**
 * Agents: the ones this account registered (from the `agent_registered` events), and a form to register one
 * of its direct sub-accounts, or to change an agent's budget, cap or programs (`register_agent`).
 */
export function Agents({
  session,
  account,
  history,
  programs,
  isAgent,
  blocked,
  apiContracts,
  onSettled,
}: {
  session: Session;
  account: Load<AccountData>;
  history: Load<History>;
  programs: Activity["programs"] | null;
  isAgent: boolean;
  blocked: Blocked;
  apiContracts: ApiContracts;
  onSettled: () => void;
}) {
  const me = session.accountId;
  const flow = useTxFlow(onSettled);
  const ids = useId();
  const [label, setLabel] = useState("");
  const [budget, setBudget] = useState("");
  const [cap, setCap] = useState("");
  const [scope, setScope] = useState<"any" | "some">("any");
  const [picked, setPicked] = useState<number[]>([]);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [preparing, setPreparing] = useState(false);
  const blockedReason = blockedText(blocked, apiContracts);
  const canOwn = !isAgent && !isImplicit(me);
  const agentId = label ? `${label}.${me}` : `<name>.${me}`;
  const nearCredit = account.status === "ok" ? (rawOf(account.data.credit?.near) ?? BigInt(0)) : BigInt(0);

  async function prepare() {
    const e: Record<string, string> = {};
    const full = `${label}.${me}`;
    if (!/^[a-z0-9]+([-_][a-z0-9]+)*$/.test(label)) e.label = "Use lowercase letters, digits, and - or _ between them; no dots (a direct sub-account).";
    else if (!validAccountId(full)) e.label = `${full} is not a valid NEAR account id (at most 64 characters).`;
    const b = parseUnits(budget);
    const c = parseUnits(cap);
    if (!b.ok) e.budget = b.error;
    else if (b.raw <= BigInt(0)) e.budget = "Enter a budget above zero.";
    else if (b.raw > MAX_AMOUNT) e.budget = "That is above near-antibody's limit for an amount.";
    if (!c.ok) e.cap = c.error;
    else if (c.raw <= BigInt(0)) e.cap = "Enter a cap above zero.";
    else if (b.ok && c.raw > b.raw) e.cap = "The per-commit cap cannot exceed the budget.";
    if (scope === "some" && (picked.length < 1 || picked.length > 16)) e.programs = "Pick 1 to 16 programs, or choose any program.";
    setErrors(e);
    if (Object.keys(e).length || !b.ok || !c.ok) return;
    setPreparing(true);
    try {
      // The contract's own rules, checked first against the agent account as NEAR has it now.
      const [cur, exists] = await Promise.all([loadAccount(full, true), loadNearBalance(full)]);
      if (!cur.ok) return setErrors({ label: `Could not read ${full}: ${cur.error}` });
      const st = cur.data.statement;
      const cr = cur.data.credit;
      if (st?.registered && st.owner && st.owner !== me) return setErrors({ label: `${full} is already the agent of ${st.owner}.` });
      const hasValue = (rawOf(cr?.tokens) ?? BigInt(0)) > BigInt(0) || (rawOf(cr?.near) ?? BigInt(0)) > BigInt(0) || cr?.committed;
      if (!st?.registered && hasValue) return setErrors({ label: `${full} holds credit or commits of its own, so it cannot become an agent (its credit would be locked).` });
      const plan = planRegisterAgent({ owner: me, agent: full, budget: b.raw, cap: c.raw, programs: scope === "any" ? null : [...picked].sort((x, y) => x - y), nearCredit, update: Boolean(st?.registered) });
      if (exists.ok && exists.data === null) plan.notes.push(`${full} does not exist on testnet yet. Registering works, but the agent needs the account and a key to commit: create both with the commands below.`);
      flow.review(plan);
    } finally {
      setPreparing(false);
    }
  }

  return (
    <Section
      id="agents"
      title="Agents"
      lede="An agent is a direct sub-account of yours with a key that can commit, reveal and retry, and cannot withdraw. Its bonds come from your credit within a per-commit cap and a budget that counts bonds lost as well as locked; its payouts go to you."
    >
      <div className="stack" style={{ ["--gap" as string]: "28px" }}>
        <div className="stack">
          <h3>Your agents</h3>
          {history.status === "loading" ? (
            <Loading label="Reading the event log" />
          ) : history.status === "error" ? (
            <ErrorState what="the event log" error={history.error} />
          ) : history.data.agents.length === 0 ? (
            <Empty>No agent registered by this account yet.</Empty>
          ) : (
            <ul className="grid grid-2" style={{ listStyle: "none", margin: 0, padding: 0 }}>
              {history.data.agents.map((a) => (
                <AgentCard key={a.agent} entry={a} />
              ))}
            </ul>
          )}
          {history.status === "ok" && history.data.truncated ? <p className="xsmall muted">The event log is longer than this page reads; older registrations may be missing.</p> : null}
        </div>

        <div className="stack">
          <h3>Register an agent, or change one</h3>
          {!canOwn ? (
            <p className="callout small" role="note">
              <Icon name="info" size={16} />
              <span>
                {isAgent ? "This account is itself an agent: register agents from its owner's account." : "An implicit account has no sub-accounts, so it cannot have agents."}
              </span>
            </p>
          ) : account.status !== "ok" ? (
            account.status === "loading" ? <Loading label="Reading your credit" /> : <ErrorState what="your credit" error={account.error} />
          ) : flow.step.kind === "review" || flow.step.kind === "signing" ? (
            <TxReview
              plan={flow.step.plan}
              checks={flow.checks}
              blockedReason={blockedReason}
              signing={flow.step.kind === "signing" ? (flow.step.checking ? "checking" : "wallet") : false}
              onSign={() => flow.step.kind === "review" && flow.sign(flow.step.plan)}
              onBack={flow.back}
            />
          ) : flow.step.kind === "done" ? (
            <TxResults plan={flow.step.plan} results={flow.step.results} error={flow.step.error} onDone={flow.back} />
          ) : (
            <form
              className={styles.form}
              noValidate
              onSubmit={(e) => {
                e.preventDefault();
                void prepare();
              }}
            >
              {flow.step.kind === "failed" ? (
                <div className="callout callout-warn" role="alert">
                  <Icon name="alert" size={16} />
                  <div className="stack" style={{ ["--gap" as string]: "4px" }}>
                    <strong>Nothing was sent.</strong>
                    <span>{flow.step.error.message}</span>
                    {flow.step.error.detail ? <span className="mono xsmall muted">Wallet said: {flow.step.error.detail}</span> : null}
                  </div>
                </div>
              ) : null}
              <div className={styles.field}>
                <label className="label" htmlFor={`${ids}-name`}>
                  Agent account
                </label>
                <div className={styles.inputRow}>
                  <input
                    id={`${ids}-name`}
                    className="input mono"
                    autoComplete="off"
                    spellCheck={false}
                    placeholder="bot"
                    value={label}
                    onChange={(e) => setLabel(e.target.value.trim().toLowerCase())}
                    aria-invalid={errors.label ? true : undefined}
                    aria-describedby={`${ids}-name-hint${errors.label ? ` ${ids}-name-err` : ""}`}
                  />
                  <span className={styles.suffix} aria-hidden>
                    .{me}
                  </span>
                </div>
                <p id={`${ids}-name-hint`} className={styles.hint}>
                  A direct sub-account of yours: <span className="mono">{agentId}</span>. An account with credit or commits of its own cannot become an agent.
                </p>
                {errors.label ? (
                  <p id={`${ids}-name-err`} className={styles.error} role="alert">
                    {errors.label}
                  </p>
                ) : null}
              </div>
              <div className="grid grid-2">
                <div className={styles.field}>
                  <label className="label" htmlFor={`${ids}-budget`}>
                    Bond budget (wNEAR)
                  </label>
                  <input
                    id={`${ids}-budget`}
                    className="input mono"
                    inputMode="decimal"
                    autoComplete="off"
                    placeholder="10"
                    value={budget}
                    onChange={(e) => setBudget(e.target.value)}
                    aria-invalid={errors.budget ? true : undefined}
                    aria-describedby={`${ids}-budget-hint${errors.budget ? ` ${ids}-budget-err` : ""}`}
                  />
                  <p id={`${ids}-budget-hint`} className={styles.hint}>
                    The most it may cost you: bonds locked now plus bonds it lost.
                  </p>
                  {errors.budget ? (
                    <p id={`${ids}-budget-err`} className={styles.error} role="alert">
                      {errors.budget}
                    </p>
                  ) : null}
                </div>
                <div className={styles.field}>
                  <label className="label" htmlFor={`${ids}-cap`}>
                    Per-commit cap (wNEAR)
                  </label>
                  <input
                    id={`${ids}-cap`}
                    className="input mono"
                    inputMode="decimal"
                    autoComplete="off"
                    placeholder="2"
                    value={cap}
                    onChange={(e) => setCap(e.target.value)}
                    aria-invalid={errors.cap ? true : undefined}
                    aria-describedby={`${ids}-cap-hint${errors.cap ? ` ${ids}-cap-err` : ""}`}
                  />
                  <p id={`${ids}-cap-hint`} className={styles.hint}>
                    The largest bond one commit may take; a program whose bond is larger is refused.
                  </p>
                  {errors.cap ? (
                    <p id={`${ids}-cap-err`} className={styles.error} role="alert">
                      {errors.cap}
                    </p>
                  ) : null}
                </div>
              </div>
              <fieldset className={styles.programs} aria-describedby={errors.programs ? `${ids}-prog-err` : undefined}>
                <legend>Programs it may commit to</legend>
                <label className={styles.checkbox}>
                  <input type="radio" name={`${ids}-scope`} checked={scope === "any"} onChange={() => setScope("any")} />
                  <span>Any program</span>
                </label>
                <label className={styles.checkbox}>
                  <input type="radio" name={`${ids}-scope`} checked={scope === "some"} onChange={() => setScope("some")} />
                  <span>Only the programs I pick (1 to 16); a commit elsewhere is refused and counted as a containment event</span>
                </label>
                {scope === "some" ? (
                  programs && programs.length ? (
                    <div className={styles.programGrid}>
                      {programs.map((p) => (
                        <label key={p.id} className={styles.checkbox}>
                          <input
                            type="checkbox"
                            checked={picked.includes(p.id)}
                            onChange={(e) => setPicked((cur) => (e.target.checked ? [...cur, p.id] : cur.filter((x) => x !== p.id)))}
                          />
                          <span>
                            Program {p.id} <span className="xsmall muted">· {p.name}{p.id <= 4 ? " · earlier run" : ""}</span>
                          </span>
                        </label>
                      ))}
                    </div>
                  ) : (
                    <p className="xsmall muted">The program list could not be read; choose any program, or refresh.</p>
                  )
                ) : null}
                {errors.programs ? (
                  <p id={`${ids}-prog-err`} className={styles.error} role="alert">
                    {errors.programs}
                  </p>
                ) : null}
              </fieldset>
              <div className="row">
                <button type="submit" className="btn btn-primary" disabled={preparing || Boolean(blockedReason)} aria-busy={preparing}>
                  {preparing ? "Checking the account…" : "Review the transaction"}
                </button>
                <span className="xsmall muted">
                  Your NEAR credit: <Amount raw={nearCredit} unit="NEAR" />
                </span>
              </div>
            </form>
          )}
        </div>

        {canOwn ? (
          <div className="stack">
            <h3>Then, in a terminal</h3>
            <p className="small muted" style={{ maxWidth: "70ch" }}>
              The agent needs its account and a key that can only commit, reveal and retry on near-antibody. Both are made in the CLI: this page never adds a
              key to an account. Amounts are in NEAR for the account and the key&apos;s gas allowance.
            </p>
            <Command label="Create the agent's account">{`antibody --account ${me} admin create-account --new-account-id ${agentId} --amount-near 1 --key-out agent-full-key.json`}</Command>
            <Command label="Add the agent's function-call key">{`antibody --account ${agentId} --key-file agent-full-key.json agent key add --allowance-near 1 --out agent-key.json`}</Command>
            <Command label="Register it from the CLI instead">{`antibody --account ${me} agent register --agent ${agentId} --budget ${budget || "10"} --per-commit-cap ${cap || "2"}${scope === "some" ? picked.map((p) => ` --program ${p}`).join("") : ""}`}</Command>
            <p className="small muted">Then give an AI agent the key over MCP:</p>
            <Command label="MCP server config" prompt={false}>
              {JSON.stringify({ mcpServers: { antibody: { command: "antibody", args: ["--account", agentId, "--key-file", "agent-key.json", "mcp", "--workspace", "<dir>"] } } }, null, 2)}
            </Command>
            <p className="xsmall muted">
              Keep <span className="mono">agent-full-key.json</span> offline: it controls the agent&apos;s account. The agent itself gets only{" "}
              <span className="mono">agent-key.json</span>. Until you fill in the form, the budget and cap in the command are examples.
            </p>
          </div>
        ) : null}
      </div>
    </Section>
  );
}
