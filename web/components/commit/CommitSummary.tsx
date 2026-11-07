import Link from "next/link";
import { KindBadge, StateBadge, TierBadge } from "@/components/ui/Badges";
import { Icon } from "@/components/ui/Icon";
import { Pill } from "@/components/ui/Pill";
import { Time } from "@/components/ui/Time";
import { AccountLink } from "@/components/ui/TxLink";
import { VerdictStamp } from "@/components/ui/Verdict";
import type { CommitDetail, ProgramDetail, VerdictKey } from "@/lib/api";
import { wnear } from "@/lib/format";
import { bondFate, numOf, slotInfo, slotName, stateData, strOf, type ProgramInfo, who } from "./model";
import styles from "./commit.module.css";

/** The commit at a glance: where it belongs, who sent it, its state and the verdict as a stamp. */
export function CommitSummary({
  c,
  label,
  kind,
  keys,
  tier,
  t2Contradicts,
  severity,
  program,
  info,
  moment,
}: {
  c: CommitDetail;
  label: string;
  kind: string | null;
  keys: VerdictKey[];
  tier: string | null;
  t2Contradicts: boolean;
  severity: string | null;
  program: ProgramDetail | null;
  info: ProgramInfo;
  moment: boolean;
}) {
  const commit = c.commit;
  const fate = bondFate(label);
  const paidAmount = label === "Done:Paid" ? strOf(stateData(commit.state)?.amount) : null;
  const runs: { id: number; what: string }[] = [];
  const add = (v: unknown, what: string) => {
    const n = numOf(v);
    if (n !== null && !runs.some((r) => r.id === n)) runs.push({ id: n, what });
  };
  add(commit.run, "verification");
  add(commit.second, "second run");
  add(commit.rerun, "latest re-run");
  add(commit.wrapped, "release wrap");
  return (
    <div className="page">
      <div className={`card ${styles.summary}`}>
        <dl className={`kv ${styles.facts}`}>
          <dt>Program</dt>
          <dd>
            <span className={styles.inline}>
              <Link href={`/programs/${commit.program}`}>Program {commit.program}</Link>
              {info.name ? <span className="small">{info.name}</span> : null}
              {info.practice ? <span className="tag">practice</span> : info.ours ? <span className="tag">our fixture</span> : null}
              {info.earlier ? <span className="tag">earlier run</span> : null}
            </span>
          </dd>
          <dt>Round</dt>
          <dd className="num">{commit.round ?? "—"}</dd>
          <dt>Hunter</dt>
          <dd>
            <span className={styles.inline}>
              <AccountLink account={commit.hunter} />
              {who(commit.hunter) ? <span className="small muted">{who(commit.hunter)}</span> : null}
            </span>
          </dd>
          {commit.beneficiary && commit.beneficiary !== commit.hunter ? (
            <>
              <dt>Paid to</dt>
              <dd>
                <span className={styles.inline}>
                  <AccountLink account={commit.beneficiary} />
                  {who(commit.beneficiary) ? <span className="small muted">{who(commit.beneficiary)}</span> : null}
                </span>
              </dd>
            </>
          ) : null}
          <dt>State</dt>
          <dd>
            <span className={styles.inline}>
              <StateBadge state={label} />
              {label ? (
                <code className={styles.rawLabel} title="The contract's own state name">
                  {label}
                </code>
              ) : null}
            </span>
          </dd>
          <dt>Verdict</dt>
          <dd>
            <span className={styles.inline}>
              {kind || label !== "Committed" ? <KindBadge kind={kind} /> : <Pill tone="pending">not revealed yet</Pill>}
              <TierBadge tier={tier} />
              {t2Contradicts ? (
                <Pill tone="bad" size="sm">
                  T2 contradicts
                </Pill>
              ) : null}
            </span>
            {keys.length ? (
              <div className="small muted" style={{ marginTop: 4 }}>
                {keys
                  .map((k) => {
                    const s = slotInfo(program, k.slot);
                    return `${slotName(program, k.slot)}${s.severity ? `, ${s.severity}` : ""}${s.added ? " (added by a round)" : ""}`;
                  })
                  .join(" · ")}
              </div>
            ) : null}
          </dd>
          <dt>Committed</dt>
          <dd>
            <Time t={commit.committed_at} />
          </dd>
          {runs.length ? (
            <>
              <dt>Runs</dt>
              <dd>
                <span className={styles.inline}>
                  {runs.map((r) => (
                    <Link key={r.id} href={`/runs/${r.id}`}>
                      Run {r.id} <span className="muted">({r.what})</span>
                    </Link>
                  ))}
                </span>
              </dd>
            </>
          ) : null}
          <dt>Bond</dt>
          <dd className="num">
            {commit.bond ? wnear(commit.bond) : "—"}{" "}
            <span className="small muted">{fate === "returned" ? "· returned" : fate === "forfeited" ? "· forfeited to the escrow" : fate === "held" ? "· locked" : ""}</span>
          </dd>
          <dt>Payout</dt>
          <dd className="num">{paidAmount ? wnear(paidAmount) : <span className="muted">none</span>}</dd>
        </dl>
        <div className={styles.stampCol}>
          {kind ? (
            <>
              <VerdictStamp kind={kind} slots={keys.map((k) => k.slot)} severity={severity ?? undefined} receipt={c.proof_card?.receipt_sha256 ?? null} />
              <p className="xsmall muted">The verdict as a stamp: its kind, the broken slots and the receipt hash that any re-run of the same input reproduces.</p>
            </>
          ) : (
            <>
              <Pill tone="pending">no verdict yet</Pill>
              <p className="xsmall muted">The verdict appears here once the TEE has judged the revealed proof.</p>
            </>
          )}
        </div>
      </div>
      <div className={`${styles.notes} ${styles.spaced}`}>
        {info.practice ? (
          <div className="callout">
            <Icon name="info" size={16} />
            <p>
              On the practice program: our deliberately vulnerable tutorial, open to anyone with no account. Its payouts are testnet wNEAR, with no
              value. <Link href="/try">Try it yourself</Link>
            </p>
          </div>
        ) : info.ours && !info.earlier ? (
          <div className="callout">
            <Icon name="info" size={16} />
            <p>{info.name ? `${info.name.replace(/ — .*$/, "")} is` : "This program is"} our deliberately vulnerable test fixture, with planted bugs; wNEAR on testnet has no value.</p>
          </div>
        ) : null}
        {info.earlier ? (
          <div className="callout">
            <Icon name="clock" size={16} />
            <p>Program {commit.program} is from an earlier run on this deployment; the recorded run&apos;s programs are 5, 6 and 7.</p>
          </div>
        ) : null}
        {moment ? (
          <div className="callout">
            <Icon name="play" size={16} />
            <p>
              This commit is the one moment: its Critical was paid and the fixture paused on both chains. The moment is recorded, not live.{" "}
              <Link href="/moment">Every step, checked</Link>
            </p>
          </div>
        ) : null}
      </div>
    </div>
  );
}
