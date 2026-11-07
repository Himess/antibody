import type { ReactNode } from "react";
import { AlertMessage } from "@/components/checks/AlertMessage";
import { Icon, type IconName } from "@/components/ui/Icon";
import { Skeleton } from "@/components/ui/States";
import { relative, utc } from "@/lib/format";
import { nextHourMs, vendsLeft } from "@/lib/practice/vending";
import { FAUCET_MIN_ALLOWANCE_NEAR, FAUCET_MIN_BALANCE_NEAR, FAUCET_MIN_CREDIT_NEAR, TRY_CREDIT_NEAR, type PendingImage, type TryInitial } from "./initial";
import type { Preflight } from "./store";
import styles from "./try.module.css";

export type PreState = { status: "loading" } | { status: "ready"; data: Preflight } | { status: "error"; error: string };

/** What decides how a try goes, read before it starts: the Start card and "Before you start" both say it. */
export interface Readiness {
  data?: Preflight;
  /** Worker images still inside their delay for this program, earliest first: a run on one gets no T1. */
  pending: PendingImage[];
  /** The program's challenge window (s): the T0 path pays after it. */
  challenge: number;
  programName: string;
  /** Practice accounts left this clock hour; undefined when the count could not be read. */
  left?: number;
  balanceNear?: number;
  lentNear?: number;
  lowBalance: boolean;
  keyGone: boolean;
  lowAllowance: boolean;
  /** The vending contract's NEAR credit at near-antibody: it pays each try's verification run and storage. */
  creditNear?: number;
  lowCredit: boolean;
  /** Under one try's run and storage: a try would stop at its reveal. */
  outOfCredit: boolean;
  /** The program does not take commits now. */
  closed: boolean;
}

export function readiness(initial: TryInitial, pre: PreState, now: number | null): Readiness {
  const f = initial.status.faucet;
  const data = pre.status === "ready" ? pre.data : undefined;
  const nowMs = now ?? initial.readAt;
  // An image counts at once for a program created after it was listed (its snapshot holds it).
  const pending = initial.status.pendingImages
    .filter((i) => i.countsFrom > nowMs / 1000 && (data?.createdAt === undefined || i.listedAt >= data.createdAt))
    .sort((a, b) => a.countsFrom - b.countsFrom);
  // The vending contract's balance: from our status, else from the practice panel.
  const balanceNear = f?.balanceNear ?? (initial.practice.balanceYocto ? Number(initial.practice.balanceYocto) / 1e24 : undefined);
  return {
    data,
    pending,
    challenge: data?.challengeSecs ?? 60,
    programName: data ? `program ${data.programId}${data.programKey ? ` (${data.programKey})` : ""}` : "the practice program",
    left: data?.perHour !== undefined && data.vendHour ? vendsLeft(data.perHour, data.vendHour, nowMs) : undefined,
    balanceNear,
    lentNear: data?.gasFundsYocto ? Number(data.gasFundsYocto) / 1e24 : undefined,
    lowBalance: balanceNear !== undefined && balanceNear < FAUCET_MIN_BALANCE_NEAR,
    keyGone: f?.allowanceNear === null,
    lowAllowance: f?.allowanceNear !== undefined && f.allowanceNear !== null && f.allowanceNear < FAUCET_MIN_ALLOWANCE_NEAR,
    creditNear: f?.creditNear,
    lowCredit: f?.creditNear !== undefined && f.creditNear < FAUCET_MIN_CREDIT_NEAR,
    outOfCredit: f?.creditNear !== undefined && f.creditNear < TRY_CREDIT_NEAR,
    closed: data?.phase !== undefined && data.phase !== "Open",
  };
}

/** "13:00 UTC": when the vending contract's hourly count resets. */
export const resetAt = (now: number) => `${utc(nextHourMs(now), "ms").slice(11, 16)} UTC`;

function Row({ icon, tone, children }: { icon: IconName; tone?: "ok" | "warn"; children: ReactNode }) {
  return (
    <li className={`${styles.checkRow} ${tone === "warn" ? styles.warnRow : ""}`}>
      <Icon name={icon} size={16} className={tone === "ok" ? styles.okIcon : tone === "warn" ? styles.warnIcon : undefined} />
      <div>{children}</div>
    </li>
  );
}

const nearAmount = (n: number) => `${n.toLocaleString("en-US", { maximumFractionDigits: 3 })} NEAR`;

/** Whether the try can run now, said before it starts: the program, the vend limit, the faucet, T1, the queue. */
export function BeforeStart({ initial, pre, r, now, className }: { initial: TryInitial; pre: PreState; r: Readiness; now: number | null; className?: string }) {
  const f = initial.status.faucet;
  const { data, pending, challenge, programName, balanceNear, lentNear } = r;
  const faucetLow = r.lowBalance || r.keyGone || r.lowAllowance || r.lowCredit;
  // The faucet row below states these; any other practice alert still shows as our status says it.
  const stated = ["vending-balance", "vend-key-missing", "vend-key-allowance", "vending-credit"];
  const practiceAlerts = initial.status.alerts.filter((a) => a.component === "practice" && !stated.includes(a.key));
  return (
    <div className={`card stack ${className ?? ""}`} style={{ ["--gap" as string]: "12px" }}>
      <h2 className="eyebrow">Before you start</h2>
      <ul className={styles.checks} style={{ listStyle: "none", margin: 0, padding: 0 }}>
        {pre.status === "loading" ? (
          <Row icon="clock">
            <span>Reading the practice program from the vending contract…</span>
            <Skeleton width="70%" height={12} />
          </Row>
        ) : pre.status === "error" ? (
          <Row icon="alert" tone="warn">
            Could not read the vending contract from NEAR ({pre.error}). The public RPCs rate-limit; the try reads it again when it starts.
          </Row>
        ) : (
          <Row icon="check" tone={r.closed ? "warn" : "ok"}>
            <strong className={styles.wrapAny}>{programName.charAt(0).toUpperCase() + programName.slice(1)}</strong>, read from the vending contract&apos;s{" "}
            <code>get_config</code> just now, never from a constant; its challenge window is {challenge} s.
            {r.closed ? ` It is ${data?.phase}: a try cannot commit to it now.` : ""}
          </Row>
        )}
        {data?.perHour !== undefined && r.left === 0 ? (
          <Row icon="alert" tone="warn">
            <strong>No practice account is left this hour.</strong> The vending contract vends at most {data.perHour} per clock hour (UTC), and its state on NEAR
            shows them all vended just now; the count resets at {resetAt(now ?? initial.readAt)}
            {now !== null ? ` (${relative(nextHourMs(now), now, "ms")})` : ""}. Until then, the <a href="#recorded">recorded tries</a> show the same loop.
          </Row>
        ) : data?.perHour !== undefined ? (
          <Row icon={r.left !== undefined ? "check" : "info"} tone={r.left !== undefined ? "ok" : undefined}>
            {r.left !== undefined ? (
              <>
                <strong>
                  {r.left} of {data.perHour} practice accounts left
                </strong>{" "}
                this clock hour (UTC), read from the vending contract&apos;s state on NEAR just now
              </>
            ) : (
              <>
                The vending contract vends at most <strong>{data.perHour} practice accounts per clock hour</strong> (UTC)
              </>
            )}
            {now !== null ? `; the count resets at ${resetAt(now)}` : ""}.
          </Row>
        ) : (
          <Row icon="info">The vending contract limits how many practice accounts it vends per clock hour.</Row>
        )}
        {balanceNear !== undefined ? (
          <Row icon={faucetLow ? "alert" : "check"} tone={faucetLow ? "warn" : "ok"}>
            The faucet: the vending contract holds <strong>{nearAmount(balanceNear)}</strong>
            {r.keyGone
              ? ", and the public vend key is gone"
              : f?.allowanceNear !== undefined && f.allowanceNear !== null
                ? `; the public vend key has ${nearAmount(f.allowanceNear)} of allowance`
                : ""}
            {r.creditNear !== undefined
              ? `; its NEAR credit at near-antibody, which pays each try's verification run and storage (about ${TRY_CREDIT_NEAR} NEAR a try), is ${nearAmount(r.creditNear)}`
              : ""}
            . A try costs it about 0.046 NEAR (measured on testnet);{" "}
            {lentNear !== undefined ? `the ${nearAmount(lentNear)} it lends the account comes back at the close.` : "what it lends the account comes back at the close."}
            {r.outOfCredit ? (
              <>
                {" "}
                <strong>That credit cannot pay for another try:</strong> a try now stops at its reveal; the <a href="#recorded">recorded tries</a> show the
                same loop.
              </>
            ) : faucetLow ? (
              <>
                {" "}
                <strong>It is low, so a try may be refused:</strong> the <a href="#recorded">recorded tries</a> show the same loop.
              </>
            ) : null}
          </Row>
        ) : null}
        {practiceAlerts.map((a) => (
          <Row key={a.key} icon="alert" tone="warn">
            Our status: <AlertMessage text={a.message} />
          </Row>
        ))}
        {initial.status.ok ? (
          pending.length ? (
            <Row icon="alert" tone="warn">
              <strong>T1 may not come yet.</strong> OutLayer listed new worker images recently, and near-antibody counts them for {programName} only from{" "}
              <time dateTime={new Date(pending[0].countsFrom * 1000).toISOString()}>{utc(pending[0].countsFrom)}</time>
              {now !== null ? ` (${relative(pending[0].countsFrom, now)})` : ""}
              {pending.length > 1 && pending[pending.length - 1].countsFrom !== pending[0].countsFrom ? `, the last from ${utc(pending[pending.length - 1].countsFrom)}` : ""}. A run
              on one gets no T1 before then: the verdict stays T0, and the try settles it as soon as the program&apos;s {challenge} s challenge window
              ends, about a minute and a half after the commit.
            </Row>
          ) : (
            <Row icon="check" tone="ok">
              Every worker image counts for T1 on {programName} now: T1 usually follows the verdict within seconds (9 and 12 s in the recorded tries).
            </Row>
          )
        ) : (
          <Row icon="info">
            Our status could not be read ({initial.status.error ?? "unavailable"}), so whether T1 comes in seconds is unknown. If it has not come when the
            program&apos;s {challenge} s challenge window ends, the try takes the T0 path and is paid then.
          </Row>
        )}
        {data?.held ? (
          <Row icon="alert" tone="warn">
            Commit {data.held.commitId}, committed at {utc(data.held.committedAt)}, is not yet revealed. Claims are paid in commit order, so your payout may wait until its reveal
            window ends at {utc(data.held.expiresAt)}
            {now !== null ? ` (${relative(data.held.expiresAt, now)})` : ""}, when the keeper expires it.
          </Row>
        ) : pre.status === "ready" ? (
          <Row icon="check" tone="ok">
            No unrevealed commit holds the program&apos;s queue (claims are paid in commit order).
          </Row>
        ) : null}
      </ul>
    </div>
  );
}
