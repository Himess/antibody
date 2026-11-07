"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { KindBadge, StateBadge, TierBadge } from "@/components/ui/Badges";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { Pill } from "@/components/ui/Pill";
import { AccountLink } from "@/components/ui/TxLink";
import { wnear } from "@/lib/format";
import type { TrySnapshot } from "@/lib/practice/flow";
import type { TryInitial } from "./initial";
import { fromCommit } from "./StepList";
import { tryStore } from "./store";
import styles from "./try.module.css";

/** The try's outcome as the chain has it: the state, the payout, the times from the commit, and checks. */
export function TryResult({ s }: { s: TrySnapshot }) {
  const id = s.commitId;
  const paid = s.state === "Done:Paid";
  const c = s.steps.commit;
  const t = (x: Parameters<typeof fromCommit>[0]) => {
    const r = fromCommit(x, c);
    return r ? `${r.exact ? "" : "≈ "}${r.text}` : "—";
  };
  const vend = s.steps.vend;
  const pay = s.steps.pay;
  const vendToPaid = vend.at !== undefined && pay.at !== undefined ? { secs: (pay.at - vend.at) / 1000, exact: !!vend.exact && !!pay.exact } : null;
  const antibody = s.antibody ?? "bounty.antibody.testnet";
  return (
    <div className={`card ${styles.result} ${paid ? "" : styles.resultPending}`}>
      <div className={styles.resultHead}>
        <StateBadge state={s.state} />
        <KindBadge kind={s.verdict?.kind} />
        <TierBadge tier={s.tier ?? null} />
        {s.closed ? (
          <Pill tone="neutral" icon="check" size="sm">
            account closed
          </Pill>
        ) : null}
      </div>
      {paid ? (
        <div className="stack" style={{ ["--gap" as string]: "6px" }}>
          <p className={styles.amount}>{wnear(s.paidRaw)}</p>
          <p className={`small ${styles.wrapAny}`}>
            Paid to <AccountLink account={s.beneficiary ?? s.vending} />, the vending contract: it paid your bond and your run, so the payout returns to it. Testnet wNEAR has
            no value.
          </p>
        </div>
      ) : (
        <p className="small">
          {s.state === "Verifying"
            ? `Verified; its payout (wNEAR to ${s.vending}) follows once the program's earlier commits are settled: claims go in commit order, and the keeper runs allocate and pay.`
            : `The commit is ${s.state ?? "in an unknown state"}.`}
        </p>
      )}
      <dl className={styles.times}>
        <div>
          <dt>verdict</dt>
          <dd>{t(s.steps.verdict)}</dd>
        </div>
        <div>
          <dt>T1</dt>
          <dd>{s.tier === "T1" ? t(s.steps.t1) : "none (T0)"}</dd>
        </div>
        <div>
          <dt>paid</dt>
          <dd>{paid ? (pay.at !== undefined ? t(pay) : "by the keeper") : "—"}</dd>
        </div>
        <div>
          <dt>vend to payout</dt>
          <dd>{vendToPaid ? `${vendToPaid.exact ? "" : "≈ "}${vendToPaid.secs.toFixed(1)} s` : "—"}</dd>
        </div>
      </dl>
      <p className="xsmall muted">
        Seconds from the commit&apos;s block, by block times; ≈ marks a time this tab measured itself. The recorded tries took 43.7 s and 39.9 s from vend to payout.
      </p>
      {s.t0Note ? <p className="small">{s.t0Note.charAt(0).toUpperCase() + s.t0Note.slice(1)}.</p> : null}
      {id !== undefined ? (
        <>
          <LiveCheck
            check={{ kind: "near-view", contract: antibody, method: "get_commit", args: { commit_id: id } }}
            expect={
              paid
                ? { path: "state.Done.Paid.amount", equals: s.paidRaw ?? "", says: `commit ${id} is Done:Paid, ${wnear(s.paidRaw)} (${s.paidRaw} raw)` }
                : { path: "hunter", equals: s.account, says: `commit ${id}'s hunter is ${s.account}` }
            }
            cli={`antibody status --commit ${id}`}
            label={`Check commit ${id} on NEAR`}
          />
          <div className="row">
            <Link href={`/commits/${id}`} className="btn btn-sm">
              Commit {id}, every event <Icon name="arrow" size={12} />
            </Link>
          </div>
        </>
      ) : null}
    </div>
  );
}

/**
 * T2 for this try: not yet on testnet (near-t2 is not deployed), or, once it is, the run's T2 record as
 * the API reports it (the prover service proves every practice try, minutes after the verdict).
 */
export function T2Row({ t2, runId, over }: { t2: TryInitial["t2"]; runId?: number; over: boolean }) {
  const [record, setRecord] = useState<string | null>(null);
  useEffect(() => {
    if (t2 !== "live" || runId === undefined || !over) return;
    let alive = true;
    let n = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      const r = await tryStore.apiGet<{ t2?: { outcome?: string } | null }>(`/v1/runs/${runId}`);
      if (!alive) return;
      if (r?.t2?.outcome) {
        setRecord(r.t2.outcome);
        return;
      }
      n += 1;
      if (n < 40) timer = setTimeout(poll, 30_000);
    };
    timer = setTimeout(poll, 5_000);
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
    };
  }, [t2, runId, over]);
  return (
    <div className={styles.t2Row}>
      {t2 === "live" ? (
        record ? (
          <>
            <Pill tone="t2">T2 · {record}</Pill>
            <span>near-t2 checked a validity proof of this try&apos;s verdict and recorded it as {record}.</span>
          </>
        ) : (
          <>
            <Pill tone="pending" icon="sigma">
              T2 · waiting
            </Pill>
            <span>The prover service proves every practice try: a validity proof of this try&apos;s verdict arrives minutes later, here and on its commit page.</span>
          </>
        )
      ) : t2 === "not-yet" ? (
        <>
          <Pill tone="pending" icon="sigma">
            T2 · not yet on testnet
          </Pill>
          <span>A validity proof of this try&apos;s verdict would arrive minutes later once near-t2 is on testnet (not yet). No payout depends on it.</span>
        </>
      ) : (
        <>
          <Pill tone="neutral" icon="sigma">
            T2 · status unavailable
          </Pill>
          <span>T2 is a validity proof of a disclosed verdict; its status could not be read just now. No payout depends on it.</span>
        </>
      )}
    </div>
  );
}
