import Link from "next/link";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { Section } from "@/components/ui/PageHead";
import { Skeleton, Empty, ErrorState } from "@/components/ui/States";
import type { ProgramDetail, ProgramSummary, Result } from "@/lib/api";
import { chainName } from "@/lib/config";
import { wnear } from "@/lib/format";
import { Cell } from "./Cell";
import { CoverageBar } from "./CoverageBar";
import { contractsOf, getProgram, getPrograms } from "./data";
import { coverageOf, isZero, largestPayout, programKind, RECORDED_RUN, runGroup } from "./helpers";
import { AnchorBadge, PhaseBadge, ProgramKindBadge } from "./ProgramBadges";
import styles from "./programs.module.css";

type Row = { summary: ProgramSummary; detail: Result<ProgramDetail> };

const COLUMNS = ["Program", "Kind", "Phase", "Round", "Anchor", "Escrow coverage", "Largest payout", "Commits", "Payouts"];

function ProgramRow({ summary: s, detail }: Row) {
  const d = detail.ok ? detail.data : null;
  const kind = programKind(s.name || d?.meta?.name, d?.meta?.description);
  const largest = d ? largestPayout(d.rules?.payouts_raw) : null;
  const coverage = d ? coverageOf(d.rules?.escrow_coverage, s.ledger?.available, largest) : null;
  const chainId = s.anchor?.chain_id ?? d?.rules?.anchor?.chain_id;
  const block = s.anchor?.block ?? d?.rules?.anchor?.block;
  return (
    <tr>
      <Cell label="Program" lead>
        <Link href={`/programs/${s.program_id}`} className={styles.progLink}>
          <span className={styles.progId}>Program {s.program_id}</span>
          <span className={styles.progName}>{s.name || `Program ${s.program_id}`}</span>
        </Link>
        <span className={styles.sub}>{s.project}</span>
        {!detail.ok ? <span className={styles.sub}>Details unavailable right now ({detail.error}).</span> : null}
      </Cell>
      <Cell label="Kind">
        <ProgramKindBadge kind={kind} size="sm" />
      </Cell>
      <Cell label="Phase">
        <PhaseBadge phase={s.phase} size="sm" />
      </Cell>
      <Cell label="Round">
        <span className="num">{typeof s.round === "number" ? s.round : "—"}</span>
      </Cell>
      <Cell label="Anchor">
        <div className={styles.stack6}>
          {d ? <AnchorBadge check={d.rules?.anchor?.check} size="sm" /> : <span className="muted">—</span>}
          {chainId && block ? (
            <span className="xsmall muted num">
              {chainName(chainId)} block {block}
            </span>
          ) : null}
        </div>
      </Cell>
      <Cell label="Escrow coverage">
        <CoverageBar coverage={coverage} available={s.ledger?.available} largest={largest} />
      </Cell>
      <Cell label="Largest payout">
        <span className="num">{largest ? wnear(largest) : "—"}</span>
      </Cell>
      <Cell label="Commits">
        <span className="num">{typeof s.commits === "number" ? s.commits : "—"}</span>
      </Cell>
      <Cell label="Payouts">
        <span className="num">{typeof d?.record?.paid_findings === "number" ? d.record.paid_findings : "—"}</span>
        {s.ledger?.paid && !isZero(s.ledger.paid) ? <span className={styles.sub}>{wnear(s.ledger.paid)} in all</span> : null}
      </Cell>
    </tr>
  );
}

function ProgramsTable({ rows, caption }: { rows: Row[]; caption: string }) {
  return (
    <div className={`table-wrap ${styles.cardsWrap}`}>
      <table className={`table ${styles.rtable}`}>
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
          title="Earlier runs on the same locked deployment"
          lede="The same story, run before the recorded one on the same contracts. Their records stay on NEAR; the recorded results cover programs 5, 6 and 7 only."
        >
          <ProgramsTable rows={earlier} caption="Programs from earlier runs on the same locked deployment" />
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
