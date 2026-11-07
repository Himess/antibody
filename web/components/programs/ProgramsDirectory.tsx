import Link from "next/link";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { Section } from "@/components/ui/PageHead";
import { Skeleton, Empty, ErrorState } from "@/components/ui/States";
import type { ProgramDetail, ProgramSummary, Result } from "@/lib/api";
import { sourceUrl } from "@/lib/config";
import { wnear } from "@/lib/format";
import { Cell } from "./Cell";
import { CoverageBar } from "./CoverageBar";
import { contractsOf, getProgram, getPrograms } from "./data";
import { anchorConfirmed, coverageOf, coverageText, isZero, largestPayout, phaseOf, programKind, RECORDED_RUN, runGroup, STAGING_RUN } from "./helpers";
import { AnchorBadge, PhaseBadge, ProgramKindBadge } from "./ProgramBadges";
import { ProgramName } from "./ProgramName";
import styles from "./programs.module.css";

type Row = { summary: ProgramSummary; detail: Result<ProgramDetail> };

const COLUMNS = ["Program", "Phase · round", "Anchor", "Escrow coverage", "Commits · payouts"];

function ProgramRow({ summary: s, detail }: Row) {
  const d = detail.ok ? detail.data : null;
  const kind = programKind(s.name || d?.meta?.name, d?.meta?.description);
  const largest = d ? largestPayout(d.rules?.payouts_raw) : null;
  const coverage = d ? coverageOf(d.rules?.escrow_coverage, s.ledger?.available, largest) : null;
  const round = typeof s.round === "number" ? s.round : null;
  const commits = typeof s.commits === "number" ? s.commits : null;
  const paid = typeof d?.record?.paid_findings === "number" ? d.record.paid_findings : null;
  const paidAmount = s.ledger?.paid && !isZero(s.ledger.paid) ? wnear(s.ledger.paid) : null;
  const check = d?.rules?.anchor?.check;
  return (
    <tr>
      <Cell label="Program" lead>
        <Link href={`/programs/${s.program_id}`} className={`${styles.progLink} ${styles.stretch}`}>
          <span className={styles.progId}>Program {s.program_id}</span>
          <span className={styles.progName}>
            <ProgramName name={s.name || `Program ${s.program_id}`} />
          </span>
        </Link>
        <span className={styles.progMeta}>
          <ProgramKindBadge kind={kind} size="sm" />
          <span className="xsmall muted">{s.project}</span>
        </span>
        {!detail.ok ? <span className={styles.sub}>Details unavailable right now ({detail.error}).</span> : null}
        <Icon name="arrow" size={16} className={styles.chev} />
      </Cell>

      {/* Phones and tablets: the same facts as two lines; the program page has the detail. */}
      <td className={styles.dirSummary}>
        <span className="small">
          {phaseOf(s.phase).text} · round {round ?? "—"} · {check ? (anchorConfirmed(check) ? "light-client confirmed" : "project-attested") : "anchor not read"}
        </span>
        <span className="small muted">
          {coverage === null ? (
            "Escrow coverage: —"
          ) : coverage < 1 ? (
            <>
              Escrow: {coverageText(coverage)} the largest payout{largest ? ` (${wnear(largest)})` : ""}, <strong>below it</strong>
            </>
          ) : (
            `Escrow: ${coverageText(coverage)} the largest payout${largest ? ` (${wnear(largest)})` : ""}`
          )}
        </span>
        {commits !== null ? (
          <span className="small muted">
            {paid ?? "—"} of {commits} {commits === 1 ? "commit" : "commits"} paid
          </span>
        ) : null}
      </td>

      <Cell label="Phase · round" className={styles.dirWide}>
        <span className={styles.phaseRound}>
          <PhaseBadge phase={s.phase} size="sm" />
          <span className="num">round {round ?? "—"}</span>
        </span>
      </Cell>
      <Cell label="Anchor" className={styles.dirWide}>
        {d ? <AnchorBadge check={check} size="sm" /> : <span className="muted">—</span>}
      </Cell>
      <Cell label="Escrow coverage" className={styles.dirWide}>
        <CoverageBar coverage={coverage} available={s.ledger?.available} largest={largest} />
      </Cell>
      <Cell label="Commits · payouts" className={styles.dirWide}>
        <span className={`num ${styles.nowrap}`}>{commits !== null ? `${commits} ${commits === 1 ? "commit" : "commits"}` : "—"}</span>
        <span className={`${styles.sub} ${styles.nowrap}`}>
          {paid !== null ? `${paid} paid` : "payouts —"}
          {paidAmount ? ` · ${paidAmount}` : ""}
        </span>
      </Cell>
    </tr>
  );
}

function ProgramsTable({ rows, caption }: { rows: Row[]; caption: string }) {
  return (
    <div className={`table-wrap ${styles.dirWrap}`}>
      <table className={`table ${styles.dirTable}`}>
        <caption className="visually-hidden">{caption}</caption>
        <thead>
          <tr>
            {COLUMNS.map((c) => (
              <th key={c} scope="col">
                {c}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <ProgramRow key={r.summary.program_id} {...r} />
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** While the API answers: the shape of the table, nothing invented. */
export function DirectorySkeleton() {
  return (
    <section className="page block" aria-busy="true" aria-label="Loading the programs">
      <div className="stack" style={{ ["--gap" as string]: "12px" }}>
        <Skeleton width="30%" height={20} />
        {Array.from({ length: 4 }, (_, i) => (
          <Skeleton key={i} height={52} />
        ))}
      </div>
      <span className="visually-hidden" role="status">
        Loading the programs…
      </span>
    </section>
  );
}

/**
 * Every program near-antibody holds: /v1/programs, and each /v1/programs/{id} for its rules, anchor
 * check and record, read in parallel. This run's programs first; earlier runs' in their own group.
 */
export async function ProgramsDirectory() {
  const list = await getPrograms();
  if (!list.ok)
    return (
      <section className="page block">
        <ErrorState what="the programs" error={list.error} />
      </section>
    );
  const programs = [...(list.data?.programs ?? [])].sort((a, b) => a.program_id - b.program_id);
  if (!programs.length)
    return (
      <section className="page block">
        <Empty>No program has been created on this deployment yet.</Empty>
      </section>
    );
  const [details, c] = await Promise.all([Promise.all(programs.map((p) => getProgram(p.program_id))), contractsOf()]);
  const rows: Row[] = programs.map((summary, i) => ({ summary, detail: details[i] }));
  const thisRun = rows.filter((r) => runGroup(r.summary.program_id) === "this-run");
  const later = rows.filter((r) => runGroup(r.summary.program_id) === "later");
  const earlier = rows.filter((r) => runGroup(r.summary.program_id) === "earlier");

  return (
    <>
      <Section
        id="this-run"
        eyebrow={`Recorded run, ${RECORDED_RUN.date}`}
        title="This run's programs"
        lede={
          <>
            Program 5 is Acme Lend, our deliberately vulnerable fixture on Sepolia and Base Sepolia. Program 6 is the practice
            program, a tutorial whose bug and proof are public. Program 7 is the practice program in fork mode: its contracts
            exist only inside the verifier. All three ran in the recorded run on the locked deployment.
          </>
        }
      >
        {thisRun.length ? (
          <ProgramsTable rows={thisRun} caption="This run's programs" />
        ) : (
          <Empty>The API does not list this run&apos;s programs right now.</Empty>
        )}
      </Section>

      {later.length ? (
        <Section id="later" title="Programs opened after the recorded run" lede="Not part of the recorded results; read live from near-antibody.">
          <ProgramsTable rows={later} caption="Programs opened after the recorded run" />
        </Section>
      ) : null}

      {earlier.length ? (
        <Section
          id="earlier"
          title="Earlier runs on the same contracts"
          lede={
            <>
              {/* The link and its ")" stay on one line: a line may break after the icon, and ")." must not start the next one. */}
              Programs 0 to 3 are the staging run of {STAGING_RUN.date}, on the same contracts before the lock (
              <span className="nowrap">
                <a href={sourceUrl(STAGING_RUN.results_file)} target="_blank" rel="noreferrer noopener">
                  its results file <Icon name="external" size={11} className={styles.inlineIcon} />
                </a>
                )
              </span>
              . Program 4 is an earlier run, before the recorded one. Their records stay on NEAR; the recorded run on the locked
              deployment is programs 5, 6 and 7.
            </>
          }
        >
          <ProgramsTable rows={earlier} caption="Programs from earlier runs on the same contracts" />
        </Section>
      ) : null}

      <Section id="check" title="Check it yourself" lede="Your browser asks near-antibody on NEAR testnet directly; the CLI command gives the same answer from a terminal.">
        <div className="grid grid-2">
          <div className="card stack">
            <h3 className={styles.cardTitle}>How many programs near-antibody holds</h3>
            <LiveCheck
              check={{ kind: "near-view", contract: c.antibody, method: "get_stats" }}
              expect={{
                path: "next_program",
                equals: programs[programs.length - 1].program_id + 1,
                says: `next_program ${programs[programs.length - 1].program_id + 1}: programs are numbered from 0`,
              }}
              cli="antibody judge"
            />
          </div>
          <div className="card stack">
            <h3 className={styles.cardTitle}>Every escrow equation of the contract balances</h3>
            <LiveCheck
              check={{ kind: "near-view", contract: c.antibody, method: "audit_contract" }}
              expect={{ path: "ok", equals: true, says: "ok: true (the tokens it holds equal every escrow, credit and locked bond)" }}
              cli="antibody judge"
            />
          </div>
        </div>
      </Section>
    </>
  );
}
