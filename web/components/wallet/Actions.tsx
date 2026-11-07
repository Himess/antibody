"use client";

import { useId, useState } from "react";
import { Icon } from "@/components/ui/Icon";
import { Section } from "@/components/ui/PageHead";
import { ErrorState, Loading } from "@/components/ui/States";
import { loadNearBalance, loadTokenRegistration } from "@/lib/wallet/account";
import { formatUnits, parseUnits, rawOf } from "@/lib/wallet/amounts";
import type { Session } from "@/lib/wallet/connector";
import { ANTIBODY, MIN_NEAR_CREDIT, WNEAR } from "@/lib/wallet/contracts";
import { planAddNearCredit, planDeposit, planWithdraw, planWithdrawNear, type Plan } from "@/lib/wallet/tx";
import type { AccountData, History } from "@/lib/wallet/types";
import { TxLink } from "@/components/ui/TxLink";
import { Time } from "@/components/ui/Time";
import type { ApiContracts } from "./AccountApp";
import type { Blocked } from "./AccountOverview";
import { Amount, type Load } from "./shared";
import { TxResults, TxReview, useTxFlow } from "./TxFlow";
import styles from "./wallet.module.css";

type Kind = "deposit" | "withdraw" | "near-credit" | "near-withdraw";

const KINDS: { id: Kind; title: string; text: string }[] = [
  { id: "deposit", title: "Deposit wNEAR into your credit", text: "Wraps NEAR first if your wallet is short. Bonds are taken from this credit." },
  { id: "withdraw", title: "Withdraw wNEAR credit", text: "Payouts, refunds and unused deposits, back to your wallet." },
  { id: "near-credit", title: "Add NEAR credit", text: "Pays the storage and verification runs of your commits." },
  { id: "near-withdraw", title: "Withdraw NEAR credit", text: "Returns NEAR credit you no longer need." },
];

/** Some NEAR kept back for gas when wrapping (the wallet asks for the exact fee). */
const GAS_RESERVE = BigInt("50000000000000000000000");

export function blockedText(blocked: Blocked, apiContracts: ApiContracts): string | null {
  if (blocked === "network") return "This site is configured for another network; the account page signs on NEAR testnet only.";
  if (blocked === "api")
    return `The API names other contracts (${apiContracts?.antibody ?? "?"}, ${apiContracts?.token ?? "?"}) than the ones fixed in this page (${ANTIBODY}, ${WNEAR}). Signing stays off until they agree.`;
  return null;
}

/** Deposit and withdraw: credit at near-antibody, each through a reviewed plan the wallet signs. */
export function Actions({
  session,
  account,
  history,
  blocked,
  apiContracts,
  onSettled,
}: {
  session: Session;
  account: Load<AccountData>;
  history: Load<History>;
  blocked: Blocked;
  apiContracts: ApiContracts;
  onSettled: () => void;
}) {
  const flow = useTxFlow(onSettled);
  const [kind, setKind] = useState<Kind>("deposit");
  const [value, setValue] = useState("");
  const [wrap, setWrap] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [preparing, setPreparing] = useState(false);
  const ids = useId();
  const blockedReason = blockedText(blocked, apiContracts);

  const data = account.status === "ok" ? account.data : null;
  const isAgent = Boolean(data?.statement?.registered || data?.credit?.agent);
  const credit = rawOf(data?.credit?.tokens) ?? BigInt(0);
  const nearCredit = rawOf(data?.credit?.near) ?? BigInt(0);
  const walletTokens = rawOf(data?.tokenBalance) ?? BigInt(0);
  const me = session.accountId;
  const owner = data?.statement?.owner ?? data?.credit?.agent?.owner;
  const unit = kind === "near-credit" || kind === "near-withdraw" ? "NEAR" : "wNEAR";

  async function prepare() {
    setError(null);
    const p = parseUnits(value, 24, unit);
    if (!p.ok) return setError(p.error);
    if (p.raw <= BigInt(0)) return setError("Enter an amount above zero.");
    setPreparing(true);
    try {
      let plan: Plan | { error: string };
      if (kind === "deposit") {
        const [reg, bal] = await Promise.all([loadTokenRegistration(me), loadNearBalance(me)]);
        if (!reg.ok) return setError(`Could not read your wrap.testnet registration: ${reg.error}`);
        const missing = p.raw > walletTokens ? p.raw - walletTokens : BigInt(0);
        const wrapNeed = missing > BigInt(0) && wrap ? missing + (reg.data.registered ? BigInt(0) : reg.data.min) : BigInt(0);
        const topUp = nearCredit < MIN_NEAR_CREDIT ? MIN_NEAR_CREDIT - nearCredit : BigInt(0);
        const held = bal.ok && bal.data ? (rawOf(bal.data.amount) ?? BigInt(0)) : null;
        if (held !== null && wrapNeed + topUp > BigInt(0) && wrapNeed + topUp + GAS_RESERVE > held) {
          const parts = [wrapNeed > BigInt(0) ? `${formatUnits(wrapNeed)} to wrap` : "", topUp > BigInt(0) ? `${formatUnits(topUp)} of NEAR credit` : ""].filter(Boolean).join(" and ");
          return setError(`This attaches ${formatUnits(wrapNeed + topUp)} NEAR (${parts}) plus gas; your wallet holds ${formatUnits(held)} NEAR.`);
        }
        plan = planDeposit({ account: me, amount: p.raw, walletTokens, nearCredit, registered: reg.data.registered, registration: reg.data.min, wrapMissing: wrap });
      } else if (kind === "withdraw") {
        if (p.raw > credit) return setError(`Your token credit is ${formatUnits(credit)} wNEAR.`);
        const reg = await loadTokenRegistration(me);
        if (!reg.ok) return setError(`Could not read your wrap.testnet registration: ${reg.error}`);
        plan = planWithdraw({ account: me, amount: p.raw, registered: reg.data.registered, registration: reg.data.min });
      } else if (kind === "near-credit") {
        const bal = await loadNearBalance(me);
        if (bal.ok && bal.data && p.raw + GAS_RESERVE > (rawOf(bal.data.amount) ?? BigInt(0))) return setError(`Your wallet holds ${formatUnits(rawOf(bal.data.amount) ?? BigInt(0))} NEAR, some of it kept for gas.`);
        plan = planAddNearCredit(me, p.raw);
      } else {
        if (p.raw > nearCredit) return setError(`Your NEAR credit is ${formatUnits(nearCredit)} NEAR.`);
        plan = planWithdrawNear(me, p.raw);
      }
      if ("error" in plan) return setError(plan.error);
      flow.review(plan);
    } finally {
      setPreparing(false);
    }
  }

  const maxFor = kind === "withdraw" ? credit : kind === "near-withdraw" ? nearCredit : null;

  return (
    <Section
      id="credit"
      title="Deposit and withdraw"
      lede="Credit, then withdraw: near-antibody pays only into credit, and only the account's own full-access key takes it out. Each action shows every transaction before your wallet opens."
    >
      {account.status === "loading" ? (
        <Loading label="Reading your credit" />
      ) : account.status === "error" ? (
        <ErrorState what="your credit" error={account.error}>
          <span className="small">The actions need your current credit. Use “Refresh from NEAR” above.</span>
        </ErrorState>
      ) : isAgent ? (
        <div className="callout" role="note">
          <Icon name="info" size={16} />
          <span>
            This account is an agent{owner ? ` of ${owner}` : ""}. Its bonds come from its owner&apos;s token credit, and its commits&apos; storage and
            verification runs from its owner&apos;s NEAR credit. An agent cannot withdraw, so anything deposited here would stay locked: deposit into
            the owner&apos;s account instead.
          </span>
        </div>
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
        <TxResults
          plan={flow.step.plan}
          results={flow.step.results}
          error={flow.step.error}
          onDone={() => {
            setValue("");
            flow.back();
          }}
        />
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
          {blockedReason ? (
            <div className="callout callout-warn" role="alert">
              <Icon name="alert" size={16} />
              <span>{blockedReason}</span>
            </div>
          ) : null}
          <fieldset className={styles.choices}>
            <legend>What do you want to do?</legend>
            {KINDS.map((k) => (
              <label key={k.id} className={styles.choice}>
                <input
                  type="radio"
                  name={`${ids}-kind`}
                  value={k.id}
                  checked={kind === k.id}
                  onChange={() => {
                    setKind(k.id);
                    setError(null);
                  }}
                />
                <span>
                  <span className={styles.choiceTitle}>{k.title}</span>
                  <span className={styles.choiceText}>{k.text}</span>
                </span>
              </label>
            ))}
          </fieldset>

          <div className={styles.field}>
            <label className="label" htmlFor={`${ids}-amount`}>
              Amount ({unit})
            </label>
            <div className={styles.inputRow}>
              <input
                id={`${ids}-amount`}
                className="input mono"
                inputMode="decimal"
                autoComplete="off"
                spellCheck={false}
                placeholder="0.5"
                value={value}
                onChange={(e) => setValue(e.target.value)}
                aria-invalid={error ? true : undefined}
                aria-describedby={`${ids}-hint${error ? ` ${ids}-error` : ""}`}
              />
              {maxFor !== null && maxFor > BigInt(0) ? (
                <button type="button" className="btn btn-sm" onClick={() => setValue(formatUnits(maxFor))}>
                  All
                </button>
              ) : null}
            </div>
            <p id={`${ids}-hint`} className={styles.hint}>
              {kind === "deposit" ? (
                <>
                  Your wallet holds <Amount raw={walletTokens} />; your token credit is <Amount raw={credit} />.
                </>
              ) : kind === "withdraw" ? (
                <>
                  Your token credit is <Amount raw={credit} />. It goes to your wallet on {WNEAR}.
                </>
              ) : kind === "near-credit" ? (
                <>
                  Your NEAR credit is <Amount raw={nearCredit} unit="NEAR" />. The CLI keeps at least 0.25 NEAR before it commits.
                </>
              ) : (
                <>
                  Your NEAR credit is <Amount raw={nearCredit} unit="NEAR" />.
                </>
              )}{" "}
              Up to 24 decimals.
            </p>
            {error ? (
              <p id={`${ids}-error`} className={styles.error} role="alert">
                {error}
              </p>
            ) : null}
          </div>

          {kind === "deposit" ? (
            <label className={styles.checkbox}>
              <input type="checkbox" checked={wrap} onChange={(e) => setWrap(e.target.checked)} />
              <span>
                If my wallet holds less wNEAR than this, wrap the missing NEAR first (wrap.testnet&apos;s <span className="mono">near_deposit</span>, one more
                transaction in the same request).
              </span>
            </label>
          ) : null}

          <div className="row">
            <button type="submit" className="btn btn-primary" disabled={preparing || Boolean(blockedReason)} aria-busy={preparing}>
              {preparing ? "Preparing…" : "Review the transactions"}
            </button>
            <span className="xsmall muted">Nothing is sent from this step.</span>
          </div>
        </form>
      )}
      {history.status === "ok" && history.data.withdrawals.length ? (
        <div className="stack" style={{ marginTop: 28 }}>
          <h3>Your withdrawals</h3>
          <div className="table-wrap">
            <table className="table">
              <caption className="visually-hidden">Withdrawals of token credit by {me}, newest first</caption>
              <thead>
                <tr>
                  <th scope="col">Amount</th>
                  <th scope="col">Transaction</th>
                  <th scope="col">When</th>
                </tr>
              </thead>
              <tbody>
                {history.data.withdrawals.slice(0, 20).map((w, i) => (
                  <tr key={w.tx ?? i}>
                    <td className={styles.nowrap}>
                      <Amount raw={w.amount} />
                    </td>
                    <td>
                      <TxLink hash={w.tx} />
                    </td>
                    <td className={styles.nowrap}>
                      <Time t={w.at} unit="ns" />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="xsmall muted">From near-antibody&apos;s withdraw events (deposits emit none). The credit shown above is the current balance.</p>
        </div>
      ) : null}
    </Section>
  );
}
