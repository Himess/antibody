import Link from "next/link";
import { Cell } from "@/components/programs/Cell";
import cards from "@/components/programs/programs.module.css";
import { KindBadge, StateBadge, TierBadge } from "@/components/ui/Badges";
import { Section } from "@/components/ui/PageHead";
import { Empty, ErrorState, Loading } from "@/components/ui/States";
import { Hash } from "@/components/ui/Hash";
import { Time } from "@/components/ui/Time";
import { config, explorers } from "@/lib/config";
import { provenance } from "@/lib/format";
import { rawOf } from "@/lib/wallet/amounts";
import type { Activity, ActivityRow } from "@/lib/wallet/types";
import { Amount, type Load } from "./shared";
import styles from "./wallet.module.css";

/** Who committed, said to the account's owner (`own`) or about the account (a read-only view). */
function Role({ row, account, own }: { row: ActivityRow; account: string; own: boolean }) {
  if (row.hunter === account && row.beneficiary === account) return <span>{own ? "You" : "This account"}</span>;
  if (row.hunter === account)
    return (
      <span className={styles.cellStack}>
        <span>{own ? "You, as an agent" : "As an agent"}</span>
        <span className="xsmall muted">
          paid to <Hash value={row.beneficiary} head={28} copy={false} href={explorers.nearAccount(row.beneficiary)} label="beneficiary" />
        </span>
      </span>
    );
  return (
    <span className={styles.cellStack}>
      <span>{provenance(row.hunter) === "practice" ? "A practice try" : own ? "Your agent" : "Its agent"}</span>
      <span className="xsmall muted">
        <Hash value={row.hunter} head={28} copy={false} href={explorers.nearAccount(row.hunter)} label="hunter" />
      </span>
    </span>
  );
}

/** The account's commits across programs (as hunter, or as the beneficiary of an agent's commits). */
export function ActivityTable({ account, activity, own = true }: { account: string; activity: Load<Activity>; own?: boolean }) {
  return (
    <Section
      id="commits"
      title="Commits and payouts"
      lede={
        <>
          Every commit whose hunter or beneficiary is this account, from each program&apos;s feed (<span className="mono">GET /v1/programs/&#123;id&#125;/feed</span>
          ). An agent&apos;s commits are paid to its owner.
        </>
      }
    >
      {activity.status === "loading" ? (
        <Loading label="Reading every program's feed" />
      ) : activity.status === "error" ? (
        <ErrorState what="the programs and their feeds" error={activity.error} />
      ) : (
        <Rows account={account} activity={activity.data} own={own} />
      )}
    </Section>
  );
}

function Rows({ account, activity, own }: { account: string; activity: Activity; own: boolean }) {
  const { rows, failed } = activity;
  const paid = rows.filter((r) => r.state === "Done:Paid" && r.beneficiary === account);
  const total = paid.reduce((s, r) => s + (rawOf(r.paid_raw) ?? BigInt(0)), BigInt(0));
  const open = rows.filter((r) => !String(r.state ?? "").startsWith("Done:")).length;
  return (
    <div className="stack">
      {failed.length ? (
        <p className="callout callout-warn small" role="status">
          The feed of program{failed.length === 1 ? "" : "s"} {failed.join(", ")} could not be read; its commits are missing below. Refresh to try again.
        </p>
      ) : null}
      {rows.length === 0 ? (
        <Empty>
          No commit from or for this account yet. Pick a program on <Link href="/programs">Programs</Link>, or run the practice program on{" "}
          <Link href="/try">Try it</Link>.
        </Empty>
      ) : (
        <>
          <p className="small" aria-live="polite">
            <strong className="num">{rows.length}</strong> commit{rows.length === 1 ? "" : "s"} · <strong className="num">{paid.length}</strong> paid to {own ? "you" : "this account"}, <Amount raw={total} /> in all
            {open ? (
              <>
                {" "}
                · <strong className="num">{open}</strong> still open
              </>
            ) : null}
          </p>
          <div className={`table-wrap ${cards.cardsWrap}`}>
            <table className={`table ${cards.rtable}`}>
              <caption className="visually-hidden">Commits of {account}, newest first</caption>
              <thead>
                <tr>
                  <th scope="col">Commit</th>
                  <th scope="col">Program</th>
                  <th scope="col">Who</th>
                  <th scope="col">State</th>
                  <th scope="col">Verdict</th>
                  <th scope="col">Paid</th>
                  <th scope="col">Committed</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr key={r.commit_id}>
                    <Cell label="Commit" lead className={styles.nowrap}>
                      <Link href={`/commits/${r.commit_id}`}>Commit {r.commit_id}</Link>
                    </Cell>
                    <Cell label="Program">
                      <span className={styles.cellStack}>
                        <Link href={`/programs/${r.program_id}`}>Program {r.program_id}</Link>
                        <span className="xsmall muted">
                          {r.programName}
                          {r.program_id <= 4 ? " · earlier run" : ""} · round {r.round ?? "—"}
                        </span>
                      </span>
                    </Cell>
                    <Cell label="Who">
                      <Role row={r} account={account} own={own} />
                    </Cell>
                    <Cell label="State">
                      <StateBadge state={r.state} size="sm" />
                    </Cell>
                    <Cell label="Verdict">
                      <span className="row" style={{ ["--gap" as string]: "4px" }}>
                        <KindBadge kind={r.verdict?.kind ?? null} size="sm" />
                        {r.verdict?.kind ? <TierBadge tier={r.verdict?.tier} size="sm" /> : null}
                      </span>
                    </Cell>
                    <Cell label="Paid" className={styles.nowrap}>
                      {r.state === "Done:Paid" ? <Amount raw={r.paid_raw} /> : <span className="muted">—</span>}
                    </Cell>
                    <Cell label="Committed" className={styles.nowrap}>
                      <Time t={r.committed_at} showRelative={false} />
                    </Cell>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="xsmall muted">
            Each commit&apos;s page shows its transactions and the checks behind its verdict. The same list from a terminal:{" "}
            <span className="mono">antibody status --program &lt;id&gt;</span>. Source: <span className="mono">{config.apiBase.replace(/^https?:\/\//, "")}</span>.
          </p>
        </>
      )}
    </div>
  );
}
