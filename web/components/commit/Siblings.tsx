import Link from "next/link";
import { KindBadge, StateBadge, TierBadge } from "@/components/ui/Badges";
import { Pill } from "@/components/ui/Pill";
import { Empty, ErrorState } from "@/components/ui/States";
import type { FeedRow } from "@/lib/api";
import { stateLabel, wnear } from "@/lib/format";
import { isPaidGap, kindWords, shortWho, who } from "./model";
import styles from "./commit.module.css";

/**
 * The program's other commits around this one (the sealed feed: never a proof's content). On wide screens
 * the state, the verdict, the tier and the payout each keep a column, so the rows line up.
 */
export function Siblings({ rows, current, programId, error, max = 9 }: { rows: FeedRow[] | null; current: number; programId: number; error: string | null; max?: number }) {
  if (!rows) return <ErrorState what={`program ${programId}'s feed`} error={error ?? undefined} partial />;
  const sorted = [...rows].sort((a, b) => a.commit_id - b.commit_id);
  if (sorted.filter((r) => r.commit_id !== current).length === 0) return <Empty>No other commits in this program yet.</Empty>;
  // A window around this commit, so a long practice feed stays short.
  const at = Math.max(0, sorted.findIndex((r) => r.commit_id === current));
  const start = Math.max(0, Math.min(at - Math.floor(max / 2), sorted.length - max));
  const shown = sorted.slice(start, start + max);
  return (
    <div className="stack" style={{ ["--gap" as string]: "10px" }}>
      <ul className={styles.sibs}>
        {shown.map((r) => {
          const mine = r.commit_id === current;
          const label = who(r.hunter);
          const kind = r.verdict?.kind ?? null;
          // "No violation · bond forfeited" already names the kind.
          const kindShown = !kind || !stateLabel(r.state).text.toLowerCase().includes(kindWords(kind).toLowerCase());
          return (
            <li key={r.commit_id} className={`${styles.sib} ${mine ? styles.sibCurrent : ""}`}>
              <div className={styles.sibMain}>
                {mine ? (
                  <span aria-current="page">
                    <strong>Commit {r.commit_id}</strong> <span className="xsmall muted">(this one)</span>
                  </span>
                ) : (
                  <Link href={`/commits/${r.commit_id}`}>Commit {r.commit_id}</Link>
                )}
                <span className="small muted" title={r.hunter}>
                  {label ? shortWho(r.hunter) : r.hunter}
                </span>
              </div>
              <div className={styles.sibBadges}>
                <span className={styles.sibState}>
                  <StateBadge state={r.state} size="sm" />
                  {isPaidGap(r.state, kind) ? (
                    <Pill tone="info" icon={null} size="sm">
                      gap claim
                    </Pill>
                  ) : null}
                </span>
                {kindShown ? (
                  <span className={styles.sibKind}>
                    <KindBadge kind={kind} size="sm" />
                  </span>
                ) : null}
                {r.verdict?.tier ? (
                  <span className={styles.sibTier}>
                    <TierBadge tier={r.verdict.tier} size="sm" />
                  </span>
                ) : null}
              </div>
              <span className={styles.sibPaid}>{r.paid_raw && r.paid_raw !== "0" ? wnear(r.paid_raw) : <span className="muted">not paid</span>}</span>
            </li>
          );
        })}
      </ul>
      {sorted.length > shown.length ? (
        <p className="small muted">
          {shown.length} of {sorted.length} commits. <Link href={`/programs/${programId}`}>Every commit of program {programId}</Link>
        </p>
      ) : (
        <p className="small muted">
          <Link href={`/programs/${programId}`}>Program {programId}</Link>: its rules, escrow and rounds.
        </p>
      )}
    </div>
  );
}
