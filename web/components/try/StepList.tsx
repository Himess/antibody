import { TxLink } from "@/components/ui/TxLink";
import { isoOf } from "@/lib/format";
import type { StepId, StepState, StepStatus, TrySnapshot } from "@/lib/practice/flow";
import styles from "./try.module.css";

/** Each step's name and what it does, in the order `antibody try` takes them. */
export const STEP_TEXT: Record<StepId, { title: string; waiting: string }> = {
  name: { title: "Name", waiting: "A new practice account name: lowercase, used once." },
  key: { title: "Key", waiting: "A throwaway ed25519 key, made in this tab." },
  program: { title: "Program", waiting: "The practice program, read from the vending contract (get_config)." },
  vend: { title: "Vend", waiting: "The public vend key creates your practice account; the vending contract pays." },
  base: { title: "Base witness", waiting: "The program's pinned base witness, read from NEAR and checked against its hash." },
  seal: { title: "Seal", waiting: "The tutorial proof against your vault, sealed in this tab (ABSEAL1)." },
  commit: { title: "Commit", waiting: "The commitment and the bond go on chain; the proof stays sealed." },
  reveal: { title: "Reveal", waiting: "The sealed blob goes on chain; near-antibody asks OutLayer's enclave to verify it." },
  verdict: { title: "Verdict", waiting: "The enclave re-runs the proof on the pinned base and answers." },
  t1: { title: "Attested on NEAR", waiting: "The run's Intel TDX quote, verified on NEAR." },
  allocate: { title: "Allocate", waiting: "Claims are allocated in commit order (permissionless)." },
  pay: { title: "Pay", waiting: "The payout goes to the beneficiary (permissionless)." },
  close: { title: "Close", waiting: "Your practice account says its try is done." },
  reclaim: { title: "Reclaim", waiting: "The public vend key has the vending contract delete the account; its NEAR goes back." },
};

export const PHASES: { title: string; steps: StepId[] }[] = [
  { title: "Set up", steps: ["name", "key", "program", "base", "vend"] },
  { title: "Prove", steps: ["seal", "commit", "reveal"] },
  { title: "Verdict", steps: ["verdict", "t1"] },
  { title: "Payout", steps: ["allocate", "pay"] },
  { title: "Clean up", steps: ["close", "reclaim"] },
];

/** Steps done in this tab, with no transaction: the commit page's smaller dot. */
const LOCAL: ReadonlySet<StepId> = new Set<StepId>(["name", "key", "program", "base", "seal"]);

/** A done step's dot, in the commit page's colours: the enclave's result, the attestation, the payout. */
const MARK: Partial<Record<StepId, string>> = { verdict: styles.markAccent, t1: styles.markT1, allocate: styles.markOk, pay: styles.markOk };

const STATE: Record<StepStatus, string> = {
  waiting: styles.isWaiting,
  running: styles.isRunning,
  done: styles.isDone,
  failed: styles.isFailed,
  skipped: styles.isSkipped,
};

/** Seconds from the commit's block, signed; null until both times are known. */
export function fromCommit(step: StepState, commit: StepState): { text: string; exact: boolean } | null {
  if (step.at === undefined || commit.at === undefined) return null;
  if (step.id === "commit") return { text: "0.0 s", exact: !!step.exact };
  const d = (step.at - commit.at) / 1000;
  return { text: `${d < 0 ? "−" : "+"}${Math.abs(d).toFixed(1)} s`, exact: !!step.exact && !!commit.exact };
}

/** Which key signed the step's transaction: every one is signed in this tab. */
function signedWith(step: StepState, s: TrySnapshot): string | null {
  if (!step.signer) return null;
  if (step.signer === s.account) return "signed with your practice key";
  if (step.signer.startsWith(s.vending)) return "signed with the public vend key";
  return `signed by ${step.signer}`;
}

function StepRow({ step, s }: { step: StepState; s: TrySnapshot }) {
  const text = STEP_TEXT[step.id];
  const t = step.status === "running" ? null : fromCommit(step, s.steps.commit);
  const by = signedWith(step, s);
  const cls = [styles.tlStep, STATE[step.status], LOCAL.has(step.id) ? styles.tlMinor : "", step.status === "done" ? (MARK[step.id] ?? "") : ""].filter(Boolean).join(" ");
  return (
    <li className={cls} aria-current={step.status === "running" ? "step" : undefined}>
      <span className={`${styles.tlTime} num`}>
        {step.status === "running" ? (
          <span className={styles.tlNow}>now</span>
        ) : t ? (
          <time
            dateTime={step.at !== undefined ? isoOf(step.at, "ms") : undefined}
            title={t.exact ? "Seconds from the commit's block, by block times" : "Seconds from the commit, by this tab's clock (approximate)"}
          >
            {t.exact ? null : (
              <span className={styles.approx} aria-hidden>
                ≈{" "}
              </span>
            )}
            {t.text}
            <span className="visually-hidden">{t.exact ? " from the commit's block" : " from the commit, approximately"}</span>
          </time>
        ) : null}
      </span>
      <span className={styles.tlRail} aria-hidden="true">
        <span className={styles.tlDot} />
      </span>
      <div className={styles.tlBody}>
        <h4 className={styles.tlTitle}>
          {text.title}
          {step.status === "failed" || step.status === "skipped" ? (
            <span className={styles.tlState}> · {step.status}</span>
          ) : (
            <span className="visually-hidden"> ({step.status})</span>
          )}
        </h4>
        <p className={styles.tlDetail}>{step.detail ?? text.waiting}</p>
      </div>
      {step.tx || by ? (
        <div className={styles.tlTx}>
          {step.tx ? <TxLink hash={step.tx} /> : null}
          {by ? <span className="xsmall muted">{by}</span> : null}
        </div>
      ) : null}
    </li>
  );
}

/**
 * The try's steps on the commit page's rail: seconds from the commit on the left, a dot per step (dashed while
 * waiting, pulsing while running, filled once done), its transaction on the right, and the phases between.
 */
export function StepList({ s }: { s: TrySnapshot }) {
  return (
    <div className={styles.tl}>
      {PHASES.map((p) => (
        <div key={p.title} className={styles.tlGroup}>
          <h3 className={`eyebrow ${styles.tlPhase}`}>{p.title}</h3>
          <ol className={styles.tlList}>
            {p.steps.map((id) => (
              <StepRow key={id} step={s.steps[id]} s={s} />
            ))}
          </ol>
        </div>
      ))}
    </div>
  );
}
