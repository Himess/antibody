import { Empty, ErrorState } from "@/components/ui/States";
import { AccountLink } from "@/components/ui/TxLink";
import { api, type LeaderRow, type Result } from "@/lib/api";
import { amount, fixtureAgentLetter, provenance, wnear } from "@/lib/format";
import styles from "./build.module.css";

type Row = { account: string; paid: string; findings: number; duplicates: number; forfeited: number };

const count = (v: unknown) => (typeof v === "number" && Number.isFinite(v) && v >= 0 ? Math.trunc(v) : 0);

/** A raw token amount as a decimal string; anything else counts as zero rather than breaking the sum. */
const rawAmount = (v: unknown) => {
  if (typeof v === "string" && /^\d+$/.test(v)) return v;
  if (typeof v === "number" && Number.isFinite(v) && v >= 0) return String(Math.trunc(v));
  return "0";
};

/** Whose account a row is: our fixture agents, practice tries, our own, or anyone else (no label). */
export function provenanceOf(account: string): { tag: string; text: string } | null {
  switch (provenance(account)) {
    case "fixture-agent":
      return { tag: "ours", text: `our fixture agent ${fixtureAgentLetter(account) ?? ""}`.trim() };
    case "practice":
      return account === "practice.antibody.testnet"
        ? { tag: "practice", text: "practice tries: their payouts go to the vending contract" }
        : { tag: "practice", text: "a practice try" };
    case "ours":
      return { tag: "ours", text: "our account" };
    default:
      return null;
  }
}

function Who({ account }: { account: string }) {
  const p = provenanceOf(account);
  return (
    <>
      <AccountLink account={account || null} />
      {p ? (
        <span className={styles.prov}>
          <span className="tag">{p.tag}</span>
          <span>{p.text}</span>
        </span>
      ) : null}
    </>
  );
}

/** Reads /v1/leaderboard on the server; wrap it in Suspense so the page does not wait for it. */
export async function LeaderboardBlock() {
  const r = await api.leaderboard();
  return <Leaderboard result={r} />;
}

export function Leaderboard({ result }: { result: Result<{ leaderboard: LeaderRow[] }> }) {
  if (!result.ok) return <ErrorState what="the leaderboard" error={result.error} />;
  const list: unknown[] = Array.isArray(result.data?.leaderboard) ? result.data.leaderboard : [];
  const rows: Row[] = list
    .filter((x): x is Partial<LeaderRow> => typeof x === "object" && x !== null)
    .map((x) => ({
      account: typeof x.account === "string" ? x.account : "",
      paid: rawAmount(x.paid_raw),
      findings: count(x.paid_findings),
      duplicates: count(x.duplicates),
      forfeited: count(x.forfeited),
    }));
  if (!rows.length) return <Empty>No paid findings, duplicates or forfeited bonds on this deployment yet.</Empty>;

  const findings = rows.reduce((n, r) => n + r.findings, 0);
  const paid = rows.reduce((n, r) => n + BigInt(r.paid), BigInt(0));

  return (
    <div className="stack">
      <p className="small muted">
        <span className="num">{rows.length}</span> beneficiaries · <span className="num">{findings}</span> paid findings ·{" "}
        <span className="num">{amount(paid.toString())}</span> wNEAR paid in all
      </p>

      <div className={`table-wrap ${styles.lbTable}`}>
        <table className="table">
          <caption className="visually-hidden">Paid findings, payouts, duplicates and forfeited bonds by beneficiary, across every program</caption>
          <thead>
            <tr>
              <th scope="col">Account</th>
              <th scope="col" className={styles.numCol}>
                Paid findings
              </th>
              <th scope="col" className={styles.numCol}>
                Payouts
              </th>
              <th scope="col" className={styles.numCol}>
                Duplicates
              </th>
              <th scope="col" className={styles.numCol}>
                Forfeited
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r, i) => (
              <tr key={`${i}-${r.account}`}>
                <th scope="row">
                  <Who account={r.account} />
                </th>
                <td className={`num ${styles.numCol}`}>{r.findings}</td>
                <td className={`num ${styles.numCol}`}>{wnear(r.paid)}</td>
                <td className={`num ${styles.numCol}`}>{r.duplicates}</td>
                <td className={`num ${styles.numCol}`}>{r.forfeited}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <ul className={styles.lbCards} aria-label="Leaderboard">
        {rows.map((r, i) => (
          <li key={`${i}-${r.account}`} className={styles.lbCard}>
            <div>
              <Who account={r.account} />
            </div>
            <dl className={styles.lbStats}>
              <div>
                <dt>Paid findings</dt>
                <dd className="num">{r.findings}</dd>
              </div>
              <div>
                <dt>Payouts</dt>
                <dd className="num">{wnear(r.paid)}</dd>
              </div>
              <div>
                <dt>Duplicates</dt>
                <dd className="num">{r.duplicates}</dd>
              </div>
              <div>
                <dt>Forfeited</dt>
                <dd className="num">{r.forfeited}</dd>
              </div>
            </dl>
          </li>
        ))}
      </ul>

      <ul className={styles.legend}>
        <li>
          Paid findings: commits that ended <code>Done:Paid</code>, at the program&apos;s severity table.
        </li>
        <li>Duplicates: valid proofs of a key (a rule and an entry point) someone held earlier; the bond is refunded and nothing is paid.</li>
        <li>Forfeited: commits whose bond went to that program&apos;s escrow: a proof that broke no rule, an invalid one, or one never revealed.</li>
      </ul>
    </div>
  );
}
