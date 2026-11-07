import type { IconName } from "@/components/ui/Icon";
import { Pill } from "@/components/ui/Pill";
import { TxLink } from "@/components/ui/TxLink";
import type { Tone } from "@/lib/format";
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
  reveal: { title: "Reveal", waiting: "The sealed blob goes on chain; near-antibody asks OutLayer's TEE to verify it." },
  verdict: { title: "Verdict", waiting: "The TEE re-runs the proof on the pinned base and answers (T0)." },
  t1: { title: "T1", waiting: "The run's Intel TDX quote, verified on NEAR." },
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

const STATUS: Record<StepStatus, { tone: Tone; word: string; icon?: IconName | null }> = {
  waiting: { tone: "neutral", word: "waiting", icon: "dot" },
  running: { tone: "pending", word: "running", icon: "clock" },
  done: { tone: "ok", word: "done" },
  failed: { tone: "bad", word: "failed" },
  skipped: { tone: "neutral", word: "skipped", icon: "arrow" },
};

/** Seconds from the commit's block, signed; null until both times are known. */
export function fromCommit(step: StepState, commit: StepState): { text: string; exact: boolean } | null {
  if (step.at === undefined || commit.at === undefined) return null;
  if (step.id === "commit") return { text: "0 s", exact: !!step.exact };
  const d = (step.at - commit.at) / 1000;
  return { text: `${d < 0 ? "−" : "+"}${Math.abs(d).toFixed(1)} s`, exact: !!step.exact && !!commit.exact };
}

function StepRow({ step, commit }: { step: StepState; commit: StepState }) {
  const st = STATUS[step.status];
  const text = STEP_TEXT[step.id];
  const t = fromCommit(step, commit);
  const cls = step.status === "running" ? styles.stepRunning : step.status === "done" ? styles.stepDone : step.status === "failed" ? styles.stepFailed : step.status === "waiting" ? styles.stepWaiting : "";
  const pill = (
    <Pill tone={st.tone} icon={st.icon} size="sm">
      {st.word}
    </Pill>
  );
  return (
    <li className={`${styles.step} ${cls}`} aria-current={step.status === "running" ? "step" : undefined}>
      <div className={styles.stepStatus}>{pill}</div>
      <div className={styles.stepTitle}>
        <h4>{text.title}</h4>
        <span className={styles.stepStatusInline}>{pill}</span>
      </div>
      <div className={styles.stepTime}>
        {t ? (
          <span title={t.exact ? "Seconds from the commit's block, by block times" : "Seconds from the commit, by this tab's clock (approximate)"}>
            <span className="visually-hidden">{t.exact ? "from the commit, by block times: " : "from the commit, approximately: "}</span>
            {t.exact ? "" : <span className={styles.approx} aria-hidden>≈</span>}
            {t.text}
          </span>
        ) : null}
      </div>
      <p className={styles.stepDetail}>{step.detail ?? text.waiting}</p>
      {step.tx ? (
        <div className={styles.stepTx}>
          <span>transaction</span>
          <TxLink hash={step.tx} />
          {step.signer ? <span>signed by {step.signer}</span> : null}
        </div>
      ) : null}
    </li>
  );
}

/** The try's steps, grouped as the loop goes: set up, prove, verdict, payout, clean up. */
export function StepList({ s }: { s: TrySnapshot }) {
  return (
    <div className="stack" style={{ ["--gap" as string]: "18px" }}>
      {PHASES.map((p) => (
        <div key={p.title} className={styles.phase}>
          <h3 className={`eyebrow ${styles.phaseTitle}`}>{p.title}</h3>
          <ol className={styles.steps}>
            {p.steps.map((id) => (
              <StepRow key={id} step={s.steps[id]} commit={s.steps.commit} />
            ))}
          </ol>
        </div>
      ))}
    </div>
  );
}
