"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { Pill } from "@/components/ui/Pill";
import { TxLink } from "@/components/ui/TxLink";
import { checkContracts } from "@/lib/wallet/account";
import { formatUnits } from "@/lib/wallet/amounts";
import { WalletFailure, classify, sendPlan, type TxResult } from "@/lib/wallet/connector";
import { ANTIBODY, RELEASE } from "@/lib/wallet/contracts";
import { attachedTotal, type Plan } from "@/lib/wallet/tx";
import type { ContractState } from "@/lib/wallet/types";
import { Busy, FailureNote, type Load } from "./shared";
import styles from "./wallet.module.css";

export type Step =
  | { kind: "form" }
  | { kind: "review"; plan: Plan }
  | { kind: "signing"; plan: Plan; checking?: boolean }
  | { kind: "done"; plan: Plan; results: TxResult[]; error?: WalletFailure }
  | { kind: "failed"; plan: Plan; error: WalletFailure };

/** A plan's way from the form to the wallet and back: review (with a fresh contract check), sign, results. */
export function useTxFlow(onSettled: () => void) {
  const [step, setStep] = useState<Step>({ kind: "form" });
  const [checks, setChecks] = useState<Load<ContractState[]>>({ status: "loading" });

  const review = useCallback((plan: Plan) => {
    setStep({ kind: "review", plan });
    setChecks({ status: "loading" });
    checkContracts()
      .then((c) => setChecks({ status: "ok", data: c }))
      .catch((e: unknown) => setChecks({ status: "error", error: e instanceof Error ? e.message : String(e) }));
  }, []);

  const sign = useCallback(
    async (plan: Plan) => {
      setStep({ kind: "signing", plan });
      try {
        const { results, error } = await sendPlan(plan, () => setStep({ kind: "signing", plan, checking: true }));
        setStep({ kind: "done", plan, results, error });
        onSettled();
      } catch (e) {
        setStep({ kind: "failed", plan, error: classify(e) });
      }
    },
    [onSettled],
  );

  const back = useCallback(() => setStep({ kind: "form" }), []);
  return { step, checks, review, sign, back };
}

const tgas = (g: bigint) => `${(g / BigInt(10) ** BigInt(12)).toString()} Tgas`;

function Deposit({ yocto }: { yocto: bigint }) {
  if (yocto === BigInt(1)) return <span>1 yoctoNEAR (the full-access key check)</span>;
  if (yocto === BigInt(0)) return <span>none</span>;
  return (
    <span>
      <strong className="num">{formatUnits(yocto)} NEAR</strong> <span className="xsmall muted num">({yocto.toString()} yoctoNEAR)</span>
    </span>
  );
}

/** The fresh check of the code the plan's receivers run, against the release record. */
function ContractCheckLine({ checks, receivers }: { checks: Load<ContractState[]>; receivers: string[] }) {
  if (checks.status === "loading") return <Busy>Checking the code at {receivers.join(" and ")} against the release record…</Busy>;
  if (checks.status === "error") return <p className="small muted">Could not check the contracts&apos; code just now ({checks.error}).</p>;
  return (
    <ul className={styles.notes} aria-label="Contract check">
      {checks.data
        .filter((c) => receivers.includes(c.account))
        .map((c) => (
          <li key={c.account}>
            <span className="mono">{c.account}</span>{" "}
            {c.matches === true ? (
              <Pill tone="ok" size="sm">
                code matches the release record
              </Pill>
            ) : c.matches === false ? (
              <Pill tone="bad" size="sm">
                code differs from the release record
              </Pill>
            ) : (
              <Pill tone="warn" size="sm">
                not checked
              </Pill>
            )}{" "}
            {c.fullAccessKeys === 0 ? (
              <Pill tone="ok" size="sm" icon="lock">
                locked: no full-access key
              </Pill>
            ) : c.fullAccessKeys !== null ? (
              <Pill tone="warn" size="sm">
                {c.fullAccessKeys} full-access key{c.fullAccessKeys === 1 ? "" : "s"}
              </Pill>
            ) : null}
            <span className="xsmall muted">
              {" "}
              expected <Hash value={c.expected} head={10} copy={false} />
              {c.actual && c.actual !== c.expected ? (
                <>
                  , found <Hash value={c.actual} head={10} copy={false} />
                </>
              ) : null}
              {c.error ? ` · ${c.error}` : ""}
            </span>
          </li>
        ))}
    </ul>
  );
}

/** Every transaction of a plan, as the wallet will be asked to sign it, before it opens. */
export function TxReview({
  plan,
  checks,
  blockedReason,
  signing,
  onSign,
  onBack,
}: {
  plan: Plan;
  checks: Load<ContractState[]>;
  blockedReason: string | null;
  /** "wallet": the wallet is asking; "checking": it answered and the outcome is being read from NEAR. */
  signing: false | "wallet" | "checking";
  onSign: () => void;
  onBack: () => void;
}) {
  const head = useRef<HTMLHeadingElement>(null);
  useEffect(() => head.current?.focus(), []);
  const receivers = Array.from(new Set(plan.txs.map((t) => t.receiver)));
  const codeDiffers = checks.status === "ok" && checks.data.some((c) => c.account === ANTIBODY && receivers.includes(ANTIBODY) && c.matches === false);
  const total = attachedTotal(plan);
  const n = plan.txs.length;
  return (
    <div className={`card ${styles.review}`}>
      <div className="stack" style={{ ["--gap" as string]: "6px" }}>
        <p className="eyebrow">Review before your wallet opens</p>
        <h3 ref={head} tabIndex={-1}>
          {plan.title}
        </h3>
        <p className="small muted">
          Your wallet will ask you to approve {n === 1 ? "one transaction" : `${n} transactions`}, from your account, on NEAR testnet. Nothing is sent
          before you approve there; you can still reject it.
        </p>
      </div>
      <ol className={styles.txList}>
        {plan.txs.map((t, i) => (
          <li key={i} className={styles.tx}>
            <div className={styles.txHead}>
              <span className={styles.txNo}>
                Transaction {i + 1} of {n}
              </span>
              <span className="small">
                to <span className="mono">{t.receiver}</span>
              </span>
            </div>
            <p className="small">{t.says}</p>
            {t.calls.map((c, j) => (
              <div key={j} className={styles.call}>
                <dl className={styles.callKv}>
                  <dt>Method</dt>
                  <dd className="mono">{c.method}</dd>
                  <dt>Arguments</dt>
                  <dd>
                    <pre className={styles.args}>{JSON.stringify(c.args, null, 2)}</pre>
                  </dd>
                  <dt>Attached</dt>
                  <dd>
                    <Deposit yocto={c.deposit} />
                  </dd>
                  <dt>Gas</dt>
                  <dd className="num">{tgas(c.gas)} at most; unused gas is refunded</dd>
                </dl>
              </div>
            ))}
          </li>
        ))}
      </ol>
      <p className="small">
        Attached in all: <strong className="num">{formatUnits(total)} NEAR</strong>, plus gas. wNEAR and NEAR on testnet have no value.
      </p>
      <ContractCheckLine checks={checks} receivers={receivers} />
      {plan.notes.length ? (
        <ul className={styles.notes}>
          {plan.notes.map((x) => (
            <li key={x}>{x}</li>
          ))}
        </ul>
      ) : null}
      <div className="stack" style={{ ["--gap" as string]: "6px" }}>
        <p className="xsmall muted">The same from a terminal, with your key file:</p>
        <Command label="CLI command">{plan.cli}</Command>
      </div>
      {blockedReason || codeDiffers ? (
        <div className="callout callout-warn" role="alert">
          <Icon name="alert" size={16} />
          <span>
            {blockedReason ?? `The code at ${ANTIBODY} differs from the release record (${RELEASE.antibody.codeHash.slice(0, 8)}…). This page will not ask your wallet to sign.`}
          </span>
        </div>
      ) : null}
      <div className="row">
        <button
          type="button"
          className="btn btn-primary"
          onClick={onSign}
          disabled={Boolean(signing) || Boolean(blockedReason) || codeDiffers}
          aria-busy={Boolean(signing)}
        >
          <Icon name="wallet" size={15} /> {signing === "checking" ? "Reading the outcome…" : signing ? "Waiting for your wallet…" : "Open your wallet to sign"}
        </button>
        <button type="button" className="btn" onClick={onBack} disabled={Boolean(signing)}>
          Back
        </button>
      </div>
      {signing === "wallet" ? <Busy>Approve or reject the request in your wallet&apos;s window. This page waits.</Busy> : null}
      {signing === "checking" ? <Busy>Your wallet sent it. Reading the outcome from NEAR…</Busy> : null}
    </div>
  );
}

const statusPill: Record<TxResult["status"], { tone: "ok" | "bad" | "warn" | "pending" | "neutral"; text: string }> = {
  ok: { tone: "ok", text: "landed" },
  failed: { tone: "bad", text: "failed on chain" },
  refunded: { tone: "warn", text: "refunded" },
  unknown: { tone: "pending", text: "sent, outcome not read yet" },
  "not-sent": { tone: "neutral", text: "not sent" },
};

/** What happened to each transaction, read back from NEAR by the browser, with its link. */
export function TxResults({ plan, results, error, onDone }: { plan: Plan; results: TxResult[]; error?: WalletFailure; onDone: () => void }) {
  const head = useRef<HTMLHeadingElement>(null);
  useEffect(() => head.current?.focus(), []);
  const allOk = results.length > 0 && results.every((r) => r.status === "ok");
  return (
    <div className={`card ${styles.review}`}>
      <div className="stack" style={{ ["--gap" as string]: "6px" }}>
        <p className="eyebrow">{plan.title}</p>
        <h3 ref={head} tabIndex={-1}>
          {allOk ? "Done" : "Finished, with something to look at"}
        </h3>
        <p className="small muted">Each outcome below was read back from NEAR by your browser, not taken from the wallet&apos;s word. Your numbers above are re-read from NEAR.</p>
      </div>
      <ol className={styles.txList} aria-live="polite">
        {results.map((r) => (
          <li key={r.index} className={styles.result}>
            <div className="row" style={{ ["--gap" as string]: "8px" }}>
              <span className={styles.txNo}>Transaction {r.index + 1}</span>
              <Pill tone={statusPill[r.status].tone} size="sm">
                {statusPill[r.status].text}
              </Pill>
              {r.hash ? <TxLink hash={r.hash} head={10} /> : null}
            </div>
            <p className="small">{r.says}</p>
            {r.detail ? <p className={r.status === "failed" ? `small ${styles.error}` : "small muted"}>{r.detail}</p> : null}
          </li>
        ))}
      </ol>
      {error ? <FailureNote kind={error.kind} message={error.message} detail={error.detail} title="The wallet stopped before the end" /> : null}
      <div className="row">
        <button type="button" className="btn" onClick={onDone}>
          Done
        </button>
      </div>
    </div>
  );
}
