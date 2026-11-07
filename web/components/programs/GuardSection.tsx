import Link from "next/link";
import { Suspense } from "react";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { Loading } from "@/components/ui/States";
import { Time } from "@/components/ui/Time";
import { AccountLink, TxLink } from "@/components/ui/TxLink";
import { sourceUrl } from "@/lib/config";
import { getJudge } from "./data";
import { GUARD } from "./recorded";
import styles from "./programs.module.css";

interface GuardView {
  owner?: string;
  program_id?: number;
  frozen?: { commit_id?: number; run_id?: number; slot?: number; at_ns?: number; reported_by?: string } | null;
}

/** The guard's state as the API last read it (`/v1/judge`, from `get_state`). */
async function GuardState() {
  const j = await getJudge();
  const g = j.ok ? (j.data?.verdict_guard as GuardView | undefined) : undefined;
  if (!g || typeof g !== "object") return <p className="small muted">The API has not read the guard right now; the check below asks NEAR directly.</p>;
  const f = g.frozen;
  return (
    <dl className={`kv ${styles.kvStack}`}>
      <dt>Watches</dt>
      <dd className="small">{typeof g.program_id === "number" ? <Link href={`/programs/${g.program_id}`}>program {g.program_id}</Link> : "—"}</dd>
      <dt>Owner</dt>
      <dd>
        <AccountLink account={g.owner} />
      </dd>
      <dt>State</dt>
      <dd>
        {f ? (
          <Pill tone="warn" icon="lock" size="sm">
            frozen
          </Pill>
        ) : (
          <Pill tone="ok" size="sm">
            not frozen
          </Pill>
        )}
      </dd>
      {f ? (
        <>
          <dt>Frozen on</dt>
          <dd className="small">
            {typeof f.commit_id === "number" ? <Link href={`/commits/${f.commit_id}`}>commit {f.commit_id}</Link> : "—"}
            {typeof f.run_id === "number" ? (
              <>
                , <Link href={`/runs/${f.run_id}`}>run {f.run_id}</Link>
              </>
            ) : null}
            {typeof f.slot === "number" ? `, slot ${f.slot}` : ""}
          </dd>
          <dt>At</dt>
          <dd>
            <Time t={f.at_ns} unit="ns" />
          </dd>
          <dt>Reported by</dt>
          <dd>
            <AccountLink account={f.reported_by} />
          </dd>
        </>
      ) : null}
    </dl>
  );
}

/**
 * 10. A NEAR app acting on this program's verdicts: guard.antibody.testnet, a DAO treasury the Acme
 * project owns, froze on A's T1-confirmed Critical, and D's duplicate did not freeze it.
 */
export function GuardSection({ guard }: { guard: string }) {
  return (
    <Section
      id="guard"
      title="Who acts on this program's verdicts"
      lede="Verdicts are NEAR state: any NEAR contract can read them and act. This one is the smallest example, a treasury that freezes when the program it watches gets a verified Critical."
    >
      <div className="grid grid-2">
        <div className="card stack">
          <h3 className={styles.cardTitle}>
            <span className="mono">{guard}</span>, a DAO treasury
          </h3>
          <p className="small">
            Owned by the Acme project and locked: its full-access key is deleted. Anyone may report a commit; the guard reads
            near-antibody itself and trusts the reporter for nothing. It freezes only on a T1-confirmed Critical of the program it
            watches, on a light-client anchor, and only for the first finding of its key.
          </p>
          <ol className={styles.plain}>
            <li>
              <Icon name="check" size={14} />
              <span className="small">
                The owner&apos;s first transfer went through. <TxLink hash={GUARD.transfer_before_tx} />
              </span>
            </li>
            <li>
              <Icon name="x" size={14} />
              <span className="small">
                D&apos;s duplicate (<Link href="/commits/16">commit 16</Link>) was reported first: not a first finding, so the report
                returned false and nothing froze. <TxLink hash={GUARD.d_report_tx} />
              </span>
            </li>
            <li>
              <Icon name="lock" size={14} />
              <span className="small">
                A&apos;s T1-confirmed Critical (<Link href="/commits/13">commit 13</Link>, solvency, slot 0) froze it.{" "}
                <TxLink hash={GUARD.a_report_tx} />
              </span>
            </li>
            <li>
              <Icon name="alert" size={14} />
              <span className="small">
                Its next transfer was refused. <TxLink hash={GUARD.transfer_refused_tx} />
              </span>
            </li>
            <li>
              <Icon name="key" size={14} />
              <span className="small">
                Then its full-access key was deleted: the guard is locked (code <span className="mono">{GUARD.code_hash.slice(0, 8)}…</span>).{" "}
                <TxLink hash={GUARD.lock_tx} />
              </span>
            </li>
          </ol>
          <p className="small">
            The guard was added on {GUARD.added}, after the recorded run, and that run&apos;s commits were reported to it then: so
            it froze almost two days after A&apos;s verdict, not seconds after.
          </p>
          <p className="xsmall muted">
            Recorded in{" "}
            <a href={sourceUrl(GUARD.results_file)} target="_blank" rel="noreferrer noopener">
              the results file <Icon name="external" size={10} className={styles.inlineIcon} />
            </a>{" "}
            under <code>verdict_consumer</code> (its <code>note</code> gives the date). The decision is one pure function,{" "}
            <a href={sourceUrl("contracts/verdict-guard/src/lib.rs", 65)} target="_blank" rel="noreferrer noopener">
              critical_finding <Icon name="external" size={10} className={styles.inlineIcon} />
            </a>
            ; how it decides:{" "}
            <a href={sourceUrl("docs/verdict-guard.md")} target="_blank" rel="noreferrer noopener">
              docs/verdict-guard.md <Icon name="external" size={10} className={styles.inlineIcon} />
            </a>
            .
          </p>
        </div>

        <div className="card stack">
          <h3 className={styles.cardTitle}>Its state now</h3>
          <Suspense fallback={<Loading label="Reading the guard" />}>
            <GuardState />
          </Suspense>
          <LiveCheck
            check={{ kind: "near-view", contract: guard, method: "get_state" }}
            expect={{ path: "frozen.commit_id", equals: 13, says: "frozen on commit 13 (A's Critical), slot 0" }}
            cli={`antibody view --contract ${guard} --method get_state`}
            label="Ask the guard"
          />
        </div>
      </div>
    </Section>
  );
}
