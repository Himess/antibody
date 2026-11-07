import Link from "next/link";
import { Fragment } from "react";
import { programKind, runGroup, type ProgramKind } from "@/components/programs/helpers";
import { RECORDED_TRIES } from "@/components/try/recorded";
import { Empty, ErrorState } from "@/components/ui/States";
import { AccountLink } from "@/components/ui/TxLink";
import { api, type FeedRow, type LeaderRow, type Result } from "@/lib/api";
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

/* ------------------------------------------------------------ the commits behind each row */

/** A commit's end as the API counts it in each column (crates/services/src/api.rs, `leaderboard`); known issues it lists but does not count. */
type Outcome = "paid" | "duplicates" | "forfeited" | "known";

const OUTCOMES: { key: Outcome; word: string }[] = [
  { key: "paid", word: "Paid" },
  { key: "duplicates", word: "Duplicates" },
  { key: "forfeited", word: "Forfeited" },
  { key: "known", word: "Known issue" },
];

const OUTCOME_OF = new Map<string, Outcome>([
  ["Done:Paid", "paid"],
  ["Done:DuplicateConfirmed", "duplicates"],
  ["Duplicate", "duplicates"],
  ["Done:Forfeited", "forfeited"],
  ["Done:Known", "known"],
]);

/** One commit behind a row: a practice try is ours or a visitor's; any other commit names its program's kind. */
type Source = { commit: number; label: string; visitor: boolean };
type Sources = Map<string, Record<Outcome, Source[]>>;

const KIND_WORD: Record<ProgramKind, string | null> = { fixture: "Acme Lend", practice: "practice", fork: "fork mode", program: null };

/**
 * Our own practice accounts on this run's programs: the recorded run's two tries
 * (docs/results/testnet-2026-11-04.json) and a rehearsal after it. Every try on an earlier run's program
 * (0–4) is ours too; any other try is a visitor's.
 */
const OUR_TRIES = new Map<string, string>([
  ...RECORDED_TRIES.map((t): [string, string] => [t.account, "the recorded run"]),
  ["pcs-rehearsal.practice.antibody.testnet", "a rehearsal"],
  ["site-check.practice.antibody.testnet", "our check"],
  ["cli-check-t0.practice.antibody.testnet", "our check"],
  ["byo-alpha.practice.antibody.testnet", "our check"],
  ["byo-beta.practice.antibody.testnet", "our check"],
  ["byo-wrong.practice.antibody.testnet", "our check"],
  ["byo-invalid.practice.antibody.testnet", "our check"],
]);

const VISITORS = "visitors'";

function sourceOf(r: FeedRow, kind: ProgramKind | undefined): Source {
  const hunter = typeof r.hunter === "string" ? r.hunter : "";
  if (provenance(hunter) === "practice") {
    const ours = runGroup(r.program_id) === "earlier" ? "earlier runs" : OUR_TRIES.get(hunter);
    return ours ? { commit: r.commit_id, label: `ours, ${ours}`, visitor: false } : { commit: r.commit_id, label: VISITORS, visitor: true };
  }
  return { commit: r.commit_id, label: (kind ? KIND_WORD[kind] : null) ?? `program ${r.program_id}`, visitor: false };
}

/**
 * The commits behind each beneficiary's numbers, from every program's feed. Null if any read fails: the
 * table then shows its numbers alone rather than a partial account of them.
 */
async function readSources(): Promise<Sources | null> {
  const list = await api.programs();
  if (!list.ok || !Array.isArray(list.data?.programs)) return null;
  const programs = list.data.programs.filter((p) => Number.isInteger(p?.program_id));
  const feeds = await Promise.all(programs.map((p) => api.feed(p.program_id)));
  const kinds = new Map(programs.map((p) => [p.program_id, programKind(p.name)]));
  const by: Sources = new Map();
  for (const f of feeds) {
    if (!f.ok) return null;
    const rows: unknown[] = Array.isArray(f.data?.submissions) ? f.data.submissions : [];
    for (const x of rows) {
      if (typeof x !== "object" || x === null) continue;
      const r = x as FeedRow;
      const outcome = OUTCOME_OF.get(String(r.state));
      if (!outcome || typeof r.beneficiary !== "string" || !Number.isInteger(r.commit_id)) continue;
      const e = by.get(r.beneficiary) ?? { paid: [], duplicates: [], forfeited: [], known: [] };
      e[outcome].push(sourceOf(r, kinds.get(r.program_id)));
      by.set(r.beneficiary, e);
    }
  }
  return by;
}

/** Commits in order, grouped by label in the order the labels first appear. */
function byLabel(list: Source[]): { label: string; commits: number[] }[] {
  const out: { label: string; commits: number[] }[] = [];
  for (const s of [...list].sort((a, b) => a.commit - b.commit)) {
    const g = out.find((x) => x.label === s.label);
    if (g) g.commits.push(s.commit);
    else out.push({ label: s.label, commits: [s.commit] });
  }
  return out;
}

/** At most this many commit links in a group; the rest are counted. */
const SHOWN = 8;

function CommitLinks({ ids }: { ids: number[] }) {
  const shown = ids.length > SHOWN ? ids.slice(0, SHOWN - 2) : ids;
  return (
    <>
      {shown.map((id, i) => (
        <Fragment key={id}>
          {i ? ", " : ""}
          <Link href={`/commits/${id}`} aria-label={`commit ${id}`}>
            {id}
          </Link>
        </Fragment>
      ))}
      {ids.length > shown.length ? ` and ${ids.length - shown.length} more` : null}
    </>
  );
}

/**
 * The commits behind a row's numbers, by outcome, each linked. Shown only when they add up to the row's
 * numbers: the leaderboard and the feeds are cached apart and may briefly disagree.
 */
function From({ row, sources }: { row: Row; sources?: Sources | null }) {
  const s = sources?.get(row.account);
  if (!s || s.paid.length !== row.findings || s.duplicates.length !== row.duplicates || s.forfeited.length !== row.forfeited) return null;
  const lines = OUTCOMES.filter((o) => s[o.key].length);
  if (!lines.length) return null;
  const tries = provenance(row.account) === "practice";
  return (
    <ul className={styles.from} aria-label="The commits behind these numbers">
      {lines.map(({ key, word }) => {
        const list = s[key];
        return (
          <li key={key}>
            {word}: commit{list.length === 1 ? "" : "s"}{" "}
            {byLabel(list).map((g, i) => (
              <Fragment key={g.label}>
                {i ? "; " : ""}
                <CommitLinks ids={g.commits} /> ({g.label === VISITORS && g.commits.length === 1 ? "a visitor's" : g.label})
              </Fragment>
            ))}
            {tries && key === "paid" && !list.some((x) => x.visitor) ? "; no visitor's try yet" : null}
          </li>
        );
      })}
    </ul>
  );
}

/* ------------------------------------------------------------ the table */

/** The commits behind the rows may not hold up the table: past this, it shows its numbers alone. */
const SOURCES_WAIT_MS = 6_000;

async function readSourcesWithin(ms: number): Promise<Sources | null> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const late = new Promise<null>((resolve) => {
    timer = setTimeout(() => resolve(null), ms);
  });
  try {
    return await Promise.race([readSources().catch(() => null), late]);
  } finally {
    clearTimeout(timer);
  }
}

/** Reads /v1/leaderboard and every program's feed on the server; wrap it in Suspense so the page does not wait for it. */
export async function LeaderboardBlock() {
  const [result, sources] = await Promise.all([api.leaderboard(), readSourcesWithin(SOURCES_WAIT_MS)]);
  return <Leaderboard result={result} sources={sources} />;
}

export function Leaderboard({ result, sources }: { result: Result<{ leaderboard: LeaderRow[] }>; sources?: Sources | null }) {
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
                  <From row={r} sources={sources} />
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
              <From row={r} sources={sources} />
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
        <li>
          Forfeited: commits whose proof broke no rule or was invalid: the bond went to that program&apos;s escrow. A commit never
          revealed loses its bond too, and is not counted here.
        </li>
      </ul>
    </div>
  );
}
