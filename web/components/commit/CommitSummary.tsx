import Link from "next/link";
import type { ReactNode } from "react";
import { KindBadge, StateBadge, TierBadge } from "@/components/ui/Badges";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { Pill } from "@/components/ui/Pill";
import { Time } from "@/components/ui/Time";
import { AccountLink } from "@/components/ui/TxLink";
import { VerdictStamp } from "@/components/ui/Verdict";
import type { CommitDetail, ProgramDetail, VerdictKey } from "@/lib/api";
import { wnear } from "@/lib/format";
import { bondFate, numOf, slotInfo, slotName, stateData, strOf, type ProgramInfo, who } from "./model";
import styles from "./commit.module.css";

/** A gap claim a round promoted and paid: the round, and the re-run that caught it (its run, its round and the slots it broke). */
export interface GapPaid {
  round: number | null;
  rerun: number | null;
  rerunRound: number | null;
  slots: string;
}

/** "gap claim · promoted at round 1": why a NoViolation was paid, said where the state is shown. */
export function GapPill({ gap }: { gap: GapPaid }) {
  return (
    <Pill tone="info" icon={null}>
      {gap.round !== null ? `gap claim · promoted at round ${gap.round}` : "gap claim · promoted"}
    </Pill>
  );
}

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
  gap,
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
  /** Set when this commit is a gap claim a round promoted and paid. */
  gap: GapPaid | null;
}) {
  const commit = c.commit;
  const hunterIs = who(commit.hunter, commit.program);
  const beneficiaryIs = who(commit.beneficiary, commit.program);
  const fate = bondFate(label);
  const paidAmount = label === "Done:Paid" ? strOf(stateData(commit.state)?.amount) : null;
  const receipt = c.proof_card?.receipt_sha256 ?? null;
  const runs: { id: number; what: string }[] = [];
  const add = (v: unknown, what: string) => {
    const n = numOf(v);
    if (n !== null && !runs.some((r) => r.id === n)) runs.push({ id: n, what });
  };
  add(commit.run, "verification");
  add(commit.second, "second run");
  add(commit.rerun, "latest re-run");
  add(commit.wrapped, "release wrap");
  // Run ids count up across the deployment: in id order, the runs read in the order they happened.
  runs.sort((a, b) => a.id - b.id);
  // The Program row's tag already says "our fixture" (and the site's notice says testnet): a callout only
  // where it adds something, for the practice program and earlier runs.
  const notes: ReactNode[] = [];
  if (info.practice)
    notes.push(
      <div key="practice" className="callout">
        <Icon name="info" size={16} />
        <p>
          On the practice program: our deliberately vulnerable tutorial, open to anyone with no account. Its payouts are testnet wNEAR, with no value.{" "}
          <Link href="/try">Try it yourself</Link>
        </p>
      </div>,
    );
  if (info.earlier)
    notes.push(
      <div key="earlier" className="callout">
        <Icon name="clock" size={16} />
        <p>Program {commit.program} is from an earlier run on this deployment; the recorded run&apos;s programs are 5, 6 and 7.</p>
      </div>,
    );
  if (moment)
    notes.push(
      <div key="moment" className="callout">
        <Icon name="play" size={16} />
        <p>
          This commit is the one moment: its Critical was paid and the fixture paused on both chains. The moment is recorded, not live.{" "}
          <Link href="/moment">Every step, checked</Link>
        </p>
      </div>,
    );
  return (
    <div className={`page ${styles.summaryWrap}`}>
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
              {hunterIs ? <span className="small muted">{hunterIs}</span> : null}
            </span>
          </dd>
          {commit.beneficiary && commit.beneficiary !== commit.hunter ? (
            <>
              <dt>Paid to</dt>
              <dd>
                <span className={styles.inline}>
                  <AccountLink account={commit.beneficiary} />
                  {beneficiaryIs ? <span className="small muted">{beneficiaryIs}</span> : null}
                </span>
              </dd>
            </>
          ) : null}
          <dt>State</dt>
          <dd>
            <span className={styles.inline}>
              <StateBadge state={label} />
              {gap ? <GapPill gap={gap} /> : null}
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
              <VerdictStamp kind={kind} slots={keys.map((k) => k.slot)} severity={severity ?? undefined} />
              {receipt ? (
                <span className={`xsmall muted ${styles.inline}`}>
                  receipt <Hash value={receipt} head={12} label="receipt sha256" />
                </span>
              ) : null}
              {gap ? (
                <p className="xsmall muted">
                  No rule broke, but a protected account lost more than the program&apos;s threshold
                  {gap.rerun !== null ? `; re-run ${gap.rerun}, at round ${gap.rerunRound ?? gap.round ?? "—"}'s fix, broke ${gap.slots || "a rule"}` : ""}.
                </p>
              ) : null}
            </>
          ) : (
            <>
              <Pill tone="pending">no verdict yet</Pill>
              <p className="xsmall muted">The verdict appears here once the TEE has judged the revealed proof.</p>
            </>
          )}
        </div>
      </div>
      {notes.length ? <div className={`${styles.notes} ${styles.spaced}`}>{notes}</div> : null}
    </div>
  );
}
