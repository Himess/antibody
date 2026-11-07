import { LiveCheck } from "@/components/ui/LiveCheck";
import { Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { Empty } from "@/components/ui/States";
import type { ProgramDetail } from "@/lib/api";
import { amount, TOKEN } from "@/lib/format";
import { isZero } from "./helpers";
import styles from "./programs.module.css";

type Term = { label: string; raw?: string | null };

const big = (raw?: string | null): bigint | null => {
  if (!raw) return null;
  try {
    return BigInt(raw);
  } catch {
    return null;
  }
};

/** Digits after the point that `amount` shows for a raw value (it keeps at most six). */
const fracDigits = (raw?: string | null) => amount(raw).split(".")[1]?.length ?? 0;

/** A raw value with exactly `dp` decimals, so a column of them lines up on the point. */
function fixed(raw: string | null | undefined, dp: number): string {
  const s = amount(raw);
  if (!/^-?[\d,]+(\.\d+)?$/.test(s)) return s;
  const [int, frac = ""] = s.split(".");
  return `${int}.${frac.padEnd(dp, "0")}`;
}

/**
 * One side of the escrow's equation as a statement: a term per row, values right-aligned on the point,
 * then a rule and the total, so the sum can be checked by eye.
 */
function Statement({ total, terms, label }: { total: Term; terms: Term[]; label: string }) {
  const dp = Math.min(6, Math.max(3, ...[total, ...terms].map((t) => fracDigits(t.raw))));
  // The page adds the terms too: a sum that differs from the total is said, not hidden.
  const parts = terms.map((t) => big(t.raw));
  const sum = parts.every((p): p is bigint => p !== null) ? parts.reduce((x, y) => x + y, BigInt(0)) : null;
  const tot = big(total.raw);
  const off = sum !== null && tot !== null && sum !== tot;
  return (
    <>
      <table className={styles.stmt} aria-label={label}>
        <tbody>
          {terms.map((t, i) => (
            <tr key={t.label}>
              <th scope="row">
                <span className={styles.stmtOp} aria-hidden>
                  {i ? "+" : ""}
                </span>
                {t.label}
              </th>
              <td className="num">{fixed(t.raw, dp)}</td>
            </tr>
          ))}
        </tbody>
        <tfoot>
          <tr>
            <th scope="row">
              <span className={styles.stmtOp} aria-hidden>
                =
              </span>
              {total.label}
            </th>
            <td className="num">{fixed(total.raw, dp)}</td>
          </tr>
        </tfoot>
      </table>
      {off ? (
        <p className="small" role="note">
          The terms add up to <span className="num">{fixed(sum.toString(), dp)}</span>, not <span className="num">{fixed(total.raw, dp)}</span>.
        </p>
      ) : null}
    </>
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
          <Statement
            label="Where the escrow is: available plus reserved plus paid plus fees plus returned equals funded"
            total={{ label: "Funded", raw: a?.funded }}
            terms={[
              { label: "Available", raw: a?.available ?? l?.available },
              { label: "Reserved", raw: a?.reserved ?? l?.reserved },
              { label: "Paid", raw: a?.paid ?? l?.paid },
              { label: "Fees", raw: a?.fees ?? l?.fees },
              { label: "Returned", raw: a?.returned ?? l?.returned },
            ]}
          />
          <h3 className={styles.cardTitle}>What came in</h3>
          <Statement
            label="What came in: deposits plus forfeited bonds plus challenge bonds equals funded"
            total={{ label: "Funded", raw: a?.funded }}
            terms={[
              { label: "Deposits", raw: l?.deposits },
              { label: "Forfeited bonds", raw: forfeits },
              { label: "Challenge bonds", raw: l?.challenge_bonds },
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
