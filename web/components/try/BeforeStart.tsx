import type { ReactNode } from "react";
import { Icon, type IconName } from "@/components/ui/Icon";
import { Skeleton } from "@/components/ui/States";
import { relative, utc } from "@/lib/format";
import { nextHourMs } from "@/lib/practice/vending";
import { FAUCET_MIN_ALLOWANCE_NEAR, FAUCET_MIN_BALANCE_NEAR, type TryInitial } from "./initial";
import type { Preflight } from "./store";
import styles from "./try.module.css";

export type PreState = { status: "loading" } | { status: "ready"; data: Preflight } | { status: "error"; error: string };

function Row({ icon, tone, children }: { icon: IconName; tone?: "ok" | "warn"; children: ReactNode }) {
  return (
    <li className={styles.checkRow}>
      <Icon name={icon} size={16} className={tone === "ok" ? styles.okIcon : tone === "warn" ? styles.warnIcon : undefined} />
      <div>{children}</div>
    </li>
  );
}

const nearAmount = (n: number) => `${n.toLocaleString("en-US", { maximumFractionDigits: 3 })} NEAR`;

/** Whether the try can run now, said before it starts: the program, the vend limit, the faucet, T1, the queue. */
export function BeforeStart({ initial, pre, now }: { initial: TryInitial; pre: PreState; now: number | null }) {
  const f = initial.status.faucet;
  const data = pre.status === "ready" ? pre.data : undefined;
  const nowS = (now ?? initial.readAt) / 1000;
  // An image counts at once for a program created after it was listed (its snapshot holds it).
  const pending = initial.status.pendingImages
    .filter((i) => i.countsFrom > nowS && (data?.createdAt === undefined || i.listedAt >= data.createdAt))
    .sort((a, b) => a.countsFrom - b.countsFrom);
  const challenge = data?.challengeSecs ?? 60;
  // The vending contract's balance: from our status, else from the practice panel.
  const balanceNear = f?.balanceNear ?? (initial.practice.balanceYocto ? Number(initial.practice.balanceYocto) / 1e24 : undefined);
  const lentNear = data?.gasFundsYocto ? Number(data.gasFundsYocto) / 1e24 : undefined;
  const lowBalance = balanceNear !== undefined && balanceNear < FAUCET_MIN_BALANCE_NEAR;
  const keyGone = f?.allowanceNear === null;
  const lowAllowance = f?.allowanceNear !== undefined && f.allowanceNear !== null && f.allowanceNear < FAUCET_MIN_ALLOWANCE_NEAR;
  const practiceAlerts = initial.status.alerts.filter((a) => a.component === "practice");
  const programName = data ? `program ${data.programId}${data.programKey ? ` (${data.programKey})` : ""}` : "the practice program";
  return (
    <div className="card stack" style={{ ["--gap" as string]: "12px" }}>
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
          <Row icon="check" tone={data?.phase && data.phase !== "Open" ? "warn" : "ok"}>
            <strong className={styles.wrapAny}>{programName.charAt(0).toUpperCase() + programName.slice(1)}</strong>, read from the vending contract&apos;s{" "}
            <code>get_config</code> just now, never from a constant; its challenge window is {challenge} s.
            {data?.phase && data.phase !== "Open" ? ` It is ${data.phase}: a try cannot commit to it now.` : ""}
          </Row>
        )}
        <Row icon="info">
          {data?.perHour !== undefined ? (
            <>
              The vending contract vends at most <strong>{data.perHour} practice accounts per clock hour</strong> (UTC)
              {now !== null ? `; the count resets at ${utc(nextHourMs(now), "ms").slice(11, 16)} UTC` : ""}.
            </>
          ) : (
            <>The vending contract limits how many practice accounts it vends per clock hour.</>
          )}
        </Row>
        {balanceNear !== undefined ? (
          <Row icon={lowBalance || keyGone || lowAllowance ? "alert" : "check"} tone={lowBalance || keyGone || lowAllowance ? "warn" : "ok"}>
            The faucet: the vending contract holds <strong>{nearAmount(balanceNear)}</strong>
            {keyGone
              ? ", and the public vend key is gone"
              : f?.allowanceNear !== undefined && f.allowanceNear !== null
                ? `; the public vend key has ${nearAmount(f.allowanceNear)} of allowance`
                : ""}
            . A try costs it about 0.046 NEAR (measured on testnet);{" "}
            {lentNear !== undefined ? `the ${nearAmount(lentNear)} it lends the account comes back at the close.` : "what it lends the account comes back at the close."}
            {lowBalance || keyGone || lowAllowance ? (
              <>
                {" "}
                <strong>It is low, so a try may be refused:</strong> the <a href="#recorded">recorded tries</a> show the same loop.
              </>
            ) : null}
          </Row>
        ) : null}
        {practiceAlerts.map((a) => (
          <Row key={a.key} icon="alert" tone="warn">
            Our status: {a.message}
          </Row>
        ))}
        {initial.status.ok ? (
          pending.length ? (
            <Row icon="alert" tone="warn">
              <strong>T1 may not come yet.</strong> OutLayer listed new worker images recently, and near-antibody counts them for {programName} only from{" "}
              <time dateTime={new Date(pending[0].countsFrom * 1000).toISOString()}>{utc(pending[0].countsFrom)}</time>
              {now !== null ? ` (${relative(pending[0].countsFrom, now)})` : ""}
              {pending.length > 1 && pending[pending.length - 1].countsFrom !== pending[0].countsFrom ? `, the last from ${utc(pending[pending.length - 1].countsFrom)}` : ""}. A run
              on one gets no T1 before then: the try waits 300 s for T1, then the verdict stays T0 and is paid after the program&apos;s {challenge} s challenge window, about 6
              minutes in the worst case.
            </Row>
          ) : (
            <Row icon="check" tone="ok">
              Every worker image counts for T1 on {programName} now: T1 usually follows the verdict within seconds (9 and 12 s in the recorded tries).
            </Row>
          )
        ) : (
          <Row icon="info">
            Our status could not be read ({initial.status.error ?? "unavailable"}), so whether T1 comes in seconds is unknown. If it does not within 300 s, the try takes the T0
            path: paid after the program&apos;s {challenge} s challenge window.
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
