import { Fragment } from "react";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { Empty } from "@/components/ui/States";
import type { ProgramDetail } from "@/lib/api";
import { amount, TOKEN } from "@/lib/format";
import { isZero } from "./helpers";
import styles from "./programs.module.css";

function Equation({ total, terms, label }: { total: { label: string; raw?: string | null }; terms: { label: string; raw?: string | null }[]; label: string }) {
  return (
    <div className={styles.eq} role="group" aria-label={label}>
      <div className={`${styles.term} ${styles.termTotal}`}>
        <span className={styles.termLabel}>{total.label}</span>
        <span className={styles.termValue}>{amount(total.raw)}</span>
      </div>
      <span className={styles.op} aria-hidden>
        =
      </span>
      {terms.map((t, i) => (
        <Fragment key={t.label}>
          {i ? (
            <span className={styles.op} aria-hidden>
              +
            </span>
          ) : null}
          <div className={styles.term}>
            <span className={styles.termLabel}>{t.label}</span>
            <span className={styles.termValue}>{amount(t.raw)}</span>
          </div>
        </Fragment>
      ))}
    </div>
  );
}

/**
 * 9. The program's escrow, both sides of its conservation equation (near-antibody's `audit`): what came
 * in equals where it is, and the reserved amount equals what the commits hold.
 */
export function LedgerSection({ d, id, antibody }: { d: ProgramDetail; id: number; antibody: string }) {
  const a = d.audit;
  const l = d.program?.ledger;
  if (!a && !l)
    return (
      <Section id="ledger" title="The ledger">
        <Empty>The API sent no ledger for this program.</Empty>
      </Section>
    );
  const forfeits = l?.forfeits;
  return (
    <Section
      id="ledger"
      title="The ledger"
      lede={`Every movement of the escrow keeps funded equal to available + reserved + paid + fees + returned. Amounts in ${TOKEN.symbol}, which has no value on testnet.`}
      actions={
        a ? (
          a.ok ? (
            <Pill tone="ok">balances</Pill>
          ) : (
            <Pill tone="bad">does not balance</Pill>
          )
        ) : (
          <Pill tone="pending">audit not read</Pill>
        )
      }
    >
      <div className="grid grid-2">
        <div className="card stack">
          <h3 className={styles.cardTitle}>Where the escrow is</h3>
          <Equation
            label="funded equals available plus reserved plus paid plus fees plus returned"
            total={{ label: "funded", raw: a?.funded }}
            terms={[
              { label: "available", raw: a?.available ?? l?.available },
              { label: "reserved", raw: a?.reserved ?? l?.reserved },
              { label: "paid", raw: a?.paid ?? l?.paid },
              { label: "fees", raw: a?.fees ?? l?.fees },
              { label: "returned", raw: a?.returned ?? l?.returned },
            ]}
          />
          <h3 className={styles.cardTitle}>What came in</h3>
          <Equation
            label="funded equals deposits plus forfeited bonds plus challenge bonds"
            total={{ label: "funded", raw: a?.funded }}
            terms={[
              { label: "deposits", raw: l?.deposits },
              { label: "forfeited bonds", raw: forfeits },
              { label: "challenge bonds", raw: l?.challenge_bonds },
            ]}
          />
          <p className="small muted">
            Forfeited bonds go to this program&apos;s escrow, where they pay future findings: never to us.
            {forfeits && !isZero(forfeits) ? ` Here, ${amount(forfeits)} ${TOKEN.symbol} came in that way.` : ""}
          </p>
        </div>

        <div className="card stack">
          <h3 className={styles.cardTitle}>The audit</h3>
          <dl className={`kv ${styles.kvStack}`}>
            <dt>Accounted</dt>
            <dd className="num">{amount(a?.accounted)}</dd>
            <dt>Reservations</dt>
            <dd>
              <span className="num">{amount(a?.reservations)}</span>
              <span className="xsmall muted"> · held by the commits; equals reserved when it balances</span>
            </dd>
            <dt>Debt</dt>
            <dd>
              <span className="num">{amount(a?.debt)}</span>
              <span className="xsmall muted"> · owed and not yet funded, queued in commit order</span>
            </dd>
            <dt>Defaulted</dt>
            <dd className="num">{amount(a?.defaulted ?? d.record?.defaulted)}</dd>
          </dl>
          <LiveCheck
            check={{ kind: "near-view", contract: antibody, method: "audit", args: { program_id: id } }}
            expect={{ path: "ok", equals: true, says: "ok: true (funded equals accounted, and reserved equals the commits' reservations)" }}
            cli={`antibody view --contract ${antibody} --method audit --args '{"program_id":${id}}'`}
            label="Run the audit on NEAR"
          />
        </div>
      </div>
    </Section>
  );
}
