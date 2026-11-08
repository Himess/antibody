import Link from "next/link";
import { KindBadge, StateBadge, TierBadge } from "@/components/ui/Badges";
import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { Empty } from "@/components/ui/States";
import { Time } from "@/components/ui/Time";
import { AccountLink, TxLink } from "@/components/ui/TxLink";
import type { FeedRow, ProgramDetail, Result, T2Badge, T2Panel } from "@/lib/api";
import { t2NotYet } from "@/lib/api";
import { feedIsTries, isZero, slotName, whoLabel, type ProgramKind, type SlotView } from "./helpers";
import { wnear } from "@/lib/format";
import { Cell } from "./Cell";
import styles from "./programs.module.css";

function T2Mark({ t2 }: { t2: T2Badge | null | undefined }) {
  if (!t2) return null;
  if (t2.outcome === "confirmed")
    return (
      <Pill tone="t2" size="sm" title="Level T2: near-t2 confirmed a zero-knowledge validity proof of this disclosed verdict">
        Zero-knowledge proof
      </Pill>
    );
  return (
    <Pill tone="bad" size="sm" title="near-t2 recorded a contradiction: the same input re-ran to another result">
      ZK proof {t2.outcome}
    </Pill>
  );
}

function FeedRowView({
  r,
  slots,
  promotedIn,
  txOf,
}: {
  r: FeedRow;
  slots: SlotView[];
  promotedIn: number | undefined;
  txOf: (event: string, commit: number) => { tx: string; explorer?: string } | undefined;
}) {
  const who = whoLabel(r.hunter, r.program_id);
  const paidTo = whoLabel(r.beneficiary, r.program_id);
  const keys = r.verdict?.keys ?? [];
  const committed = txOf("commit", r.commit_id);
  const paid = txOf("paid", r.commit_id);
  return (
    <tr>
      <Cell label="Commit" lead>
        <Link href={`/commits/${r.commit_id}`} className={styles.progLink}>
          <span className={`${styles.progName} nowrap`}>Commit {r.commit_id}</span>
        </Link>
        <span className={styles.sub}>
          <span className="nowrap">
            #{r.index} · round {r.round}
          </span>
        </span>
      </Cell>
      <Cell label="Who">
        <AccountLink account={r.hunter} />
        {who ? <span className={styles.sub}>{who}</span> : null}
        {r.beneficiary && r.beneficiary !== r.hunter ? (
          <span className={styles.sub}>
            paid to {r.beneficiary}
            {paidTo ? ` (${paidTo})` : ""}
          </span>
        ) : null}
      </Cell>
      <Cell label="When">
        <Time t={r.committed_at} />
        {committed ? (
          <span className={`${styles.sub} nowrap`}>
            <TxLink hash={committed.tx} href={committed.explorer} />
          </span>
        ) : null}
      </Cell>
      <Cell label="State">
        <StateBadge state={r.state} size="sm" />
      </Cell>
      <Cell label="Verdict">
        <div className={styles.keyRow}>
          <KindBadge kind={r.verdict?.kind} size="sm" />
          <TierBadge tier={r.verdict?.tier} size="sm" />
          <T2Mark t2={r.verdict?.t2} />
        </div>
      </Cell>
      <Cell label="Keys">
        {keys.length ? (
          <div className="stack" style={{ ["--gap" as string]: "4px" }}>
            {keys.map((k) => (
              <span key={`${k.slot}-${k.tag}`} className={styles.keyRow}>
                <span className="mono xsmall">{slotName(slots, k.slot)}</span>
                <Hash value={k.tag} head={8} label={`tag of ${slotName(slots, k.slot)}`} />
              </span>
            ))}
          </div>
        ) : (
          <span className="small muted">{r.verdict?.kind ? "no rule broken" : "—"}</span>
        )}
        {promotedIn !== undefined ? (
          <span className={styles.sub}>promoted in round {promotedIn}: its re-run at that round&apos;s code broke a rule, so it is paid at its original priority</span>
        ) : null}
      </Cell>
      <Cell label="Payout">
        <span className="num">{r.paid_raw && !isZero(r.paid_raw) ? wnear(r.paid_raw) : "—"}</span>
        {paid && r.paid_raw && !isZero(r.paid_raw) ? (
          <span className={`${styles.sub} nowrap`}>
            <TxLink hash={paid.tx} href={paid.explorer} />
          </span>
        ) : null}
      </Cell>
    </tr>
  );
}

/**
 * 7. The sealed feed: who committed when, the state, the verdict and the payout. Never the proof: it stays
 * sealed until disclosure, and the project reads only the proofs it pays for.
 */
export function CommitFeed({
  d,
  id,
  kind,
  slots,
  t2,
  practiceProgram,
}: {
  d: ProgramDetail;
  id: number;
  kind: ProgramKind;
  slots: SlotView[];
  t2: Result<T2Panel>;
  practiceProgram: number | null;
}) {
  const rows = [...(d.commits ?? [])].sort((a, b) => a.index - b.index);
  // Titled by what its rows are: practice tries, or (our fixture agents' commits included) the sealed feed.
  const tries = feedIsTries(rows, kind, practiceProgram === id);
  const events = d.events ?? [];
  const txOf = (event: string, commit: number) => {
    const e = events.find((x) => x.event === event && x.data?.commit_id === commit);
    return e ? { tx: e.tx, explorer: e.explorer } : undefined;
  };
  const withT2 = rows.filter((r) => r.verdict?.t2).length;
  // As on the landing: near-t2 on testnet with no record yet is still "not yet on testnet", never "0 with a zero-knowledge proof".
  const t2Records = t2.ok && Array.isArray(t2.data?.records) ? t2.data.records.length : 0;
  const promoted = new Map<number, number>();
  for (const round of d.program?.rounds ?? []) for (const c of round.promoted ?? []) promoted.set(c, round.round);

  return (
    <Section
      id="feed"
      title={tries ? "The tries" : "The sealed feed"}
      lede={
        tries
          ? "Every proof committed to this tutorial, practice tries labelled as such: who, when, the state, the verdict and the payout; never the proof."
          : "Every commit in order: who, when, the state, the verdict and the payout; never the proof. A proof stays sealed until disclosure, and the project reads only the proofs it pays for."
      }
      actions={
        withT2 > 0 ? (
          <Pill tone="t2" size="sm">
            {withT2} with a zero-knowledge proof
          </Pill>
        ) : (t2.ok && t2Records === 0) || t2NotYet(t2) ? (
          <Pill tone="pending" size="sm" title="Level T2: a zero-knowledge validity proof of a disclosed verdict. No privacy claim; payouts do not depend on it.">
            Zero-knowledge proofs: not yet on testnet
          </Pill>
        ) : t2.ok ? (
          <Pill tone="pending" size="sm" title="near-t2 is on testnet; none of these commits has a record yet">
            Zero-knowledge proofs: none yet
          </Pill>
        ) : (
          <Pill tone="pending" size="sm">
            Zero-knowledge proof status unavailable
          </Pill>
        )
      }
    >
      {rows.length ? (
        <div className="stack">
          <div className={`table-wrap ${styles.cardsWrap}`}>
            <table className={`table ${styles.rtable}`}>
              <caption className="visually-hidden">{tries ? "The tries" : "The sealed feed"}, in commit order</caption>
              <thead>
                <tr>
                  <th scope="col">Commit</th>
                  <th scope="col">Who</th>
                  <th scope="col">When</th>
                  <th scope="col">State</th>
                  <th scope="col">Verdict</th>
                  <th scope="col">Keys</th>
                  <th scope="col">Payout</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <FeedRowView key={r.commit_id} r={r} slots={slots} promotedIn={promoted.get(r.commit_id)} txOf={txOf} />
                ))}
              </tbody>
            </table>
          </div>
          <p className="small muted">
            A key is a rule and the entry point that broke it: the first proof of each key is paid; a later one is a duplicate that
            keeps its own proof record. Each commit&apos;s page shows its proof card and how to check it.
          </p>
          <Command label="CLI command">{`antibody status --program ${id}`}</Command>
        </div>
      ) : (
        <Empty>
          {tries ? (
            <>
              No try yet. <Link href="/try">Run the first one</Link>: no account, no key.
            </>
          ) : (
            "No commit yet: nobody has submitted a proof to this program."
          )}
        </Empty>
      )}
    </Section>
  );
}
