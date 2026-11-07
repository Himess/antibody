import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { CommitSummary } from "@/components/commit/CommitSummary";
import {
  cleanKeys,
  DEFAULT_ANTIBODY,
  earlierClaim,
  evData,
  findT2,
  hasWrapD,
  kindWords,
  numOf,
  parseId,
  programInfo,
  runEvent,
  runOutcome,
  secondsBetween,
  shortState,
  slotName,
  sortedEvents,
  stateData,
  stateLabelOf,
  strOf,
  subject,
  topSeverity,
  unknownId,
  type ProgramInfo,
} from "@/components/commit/model";
import { Outcome } from "@/components/commit/Outcome";
import { ProofCard } from "@/components/commit/ProofCard";
import { Siblings } from "@/components/commit/Siblings";
import { Tiers, type Bound } from "@/components/commit/Tiers";
import { Timeline, type PausePermit } from "@/components/commit/Timeline";
import styles from "@/components/commit/commit.module.css";
import { KindBadge, SeverityBadge, StateBadge, TierBadge } from "@/components/ui/Badges";
import { PageHead, Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { ErrorState } from "@/components/ui/States";
import { api, t2NotYet, type CommitDetail, type IndexedEvent, type ProgramDetail, type VerdictKey } from "@/lib/api";
import { cap, seconds, stateLabel, wnear } from "@/lib/format";

type Props = { params: Promise<{ id: string }> };

const crumbs = (id: number, program?: number) => [
  { href: "/programs", label: "Programs" },
  ...(program !== undefined ? [{ href: `/programs/${program}`, label: `Program ${program}` }] : []),
  { href: `/commits/${id}`, label: `Commit ${id}` },
];

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const id = parseId((await params).id);
  if (id === null) return { title: "Commit not found" };
  const r = await api.commit(id);
  if (!r.ok) return { title: `Commit ${id}` };
  const label = r.data.row?.state || stateLabelOf(r.data.commit?.state);
  const kind = r.data.proof_card?.facts?.kind ?? r.data.row?.verdict?.kind ?? null;
  const title = `Commit ${id} · ${kindWords(kind)} · ${shortState(label)}`;
  const description = `Program ${r.data.commit?.program ?? "—"}, round ${r.data.commit?.round ?? "—"}: a sealed proof by ${r.data.commit?.hunter ?? "a hunter"} on NEAR testnet. Verdict ${kindWords(kind)}, tier ${r.data.proof_card?.facts?.tier ?? "—"}; state ${label || "—"}. Every step links its transaction.`;
  return {
    title,
    description,
    openGraph: { type: "website", siteName: "Antibody", title, description },
    twitter: { card: "summary_large_image", title, description },
  };
}

/** The lede: what happened to this commit, in one or two plain sentences. */
function summary(x: {
  c: CommitDetail;
  label: string;
  kind: string | null;
  keys: VerdictKey[];
  program: ProgramDetail | null;
  events: IndexedEvent[];
  baseNs: number | null;
  t1: boolean;
}): string {
  const S = subject(x.c.commit.hunter);
  const d = stateData(x.c.commit.state);
  const slots = x.keys.map((k) => slotName(x.program, k.slot)).join(" and ");
  const sev = topSeverity(x.program, x.keys);
  const paid = x.events.find((e) => e.event === "paid");
  const gap = x.events.find((e) => e.event === "gap_promoted");
  const promoted = x.events.find((e) => e.event === "promoted");
  const of = numOf(evData(x.events.find((e) => e.event === "duplicate")).of) ?? numOf(d?.of);
  switch (x.label) {
    case "Done:Paid": {
      const amount = wnear(strOf(d?.amount) ?? strOf(evData(paid).amount));
      if (gap) return `${S}'s proof broke no rule but harmed a protected account: refunded and recorded, then paid ${amount} when round ${numOf(evData(gap).round) ?? "—"} added a rule it breaks.`;
      if (promoted)
        return `${S}'s proof was a duplicate until round ${numOf(evData(promoted).round) ?? "—"}'s fix stopped the earlier claim but not this one: promoted, and paid ${amount}.`;
      const s = secondsBetween(x.baseNs, paid?.block_ts_ns);
      return `${S}'s sealed proof broke ${sev ? `a ${cap(sev)} rule: ` : ""}${slots || "a rule"}. Verified in the TEE${x.t1 ? ", T1-confirmed" : ""} and paid ${amount}${d?.instant === true ? " instantly, under the cap" : " after its challenge window"}${s !== null ? `, ${seconds(s)} from its commit's block` : ""}.`;
    }
    case "Duplicate":
    case "Done:DuplicateConfirmed":
      return `${S}'s proof broke the same rule at the same entry point as ${of !== null ? `commit ${of}` : "an earlier commit"}, which came first: recorded with its own proof, never paid, and its bond returned.${x.label === "Done:DuplicateConfirmed" ? " The next round confirmed it a duplicate." : ""}`;
    case "Done:Known":
      return `${S}'s proof matches a known issue the project declared, sealed, before opening: never paid, and its bond returned.`;
    case "Done:Forfeited":
      return `${S}'s proof ran and broke no rule (${kindWords(x.kind ?? "NoViolation")}): nothing is paid, and its bond went to the program's escrow.`;
    case "Done:Expired":
      return `${S}'s commit was never revealed in time: its bond went to the program's escrow.`;
    case "Holder":
      return `${S}'s proof broke ${slots || "a rule"}: verified and allocated, awaiting payment.`;
    case "GapClaim":
      return `${S}'s proof broke no rule but harmed a protected account: refunded and recorded as a gap claim.`;
    case "Verifying":
      return `${S}'s proof is revealed and in the TEE.`;
    case "Committed":
      return `${S} committed a sealed proof; it is not revealed yet.`;
    default:
      return `${S}'s commit: ${stateLabel(x.label).text}.`;
  }
}

export default async function CommitPage({ params }: Props) {
  const id = parseId((await params).id);
  if (id === null) notFound();

  const [cr, health, t2, moments] = await Promise.all([api.commit(id), api.health(), api.t2(), api.moments()]);
  if (!cr.ok) {
    if (unknownId(cr)) notFound();
    return (
      <>
        <PageHead crumbs={crumbs(id)} eyebrow="Commit" title={`Commit ${id}`} />
        <div className="page">
          <ErrorState what={`commit ${id}`} error={cr.error} />
        </div>
      </>
    );
  }

  const c = cr.data;
  const commit = c.commit;
  const programId = commit.program;
  const runId = numOf(commit.run);
  const [pr, fr, rr, rx] = await Promise.all([
    api.program(programId),
    api.feed(programId),
    runId !== null ? api.run(runId) : Promise.resolve(null),
    commit.ticket ? api.reflex(programId) : Promise.resolve(null),
  ]);

  const program = pr.ok ? pr.data : null;
  const info: ProgramInfo = programInfo(programId, program);
  const events = sortedEvents(c.events);
  const programEvents = program ? sortedEvents(program.events) : [];
  const pauseEvents = programEvents.filter(
    (e) => (e.event === "pause_ticket" || e.event === "permit_requested") && numOf(evData(e).ticket) === id && numOf(evData(e).program_id) === programId,
  );
  const label = c.row?.state || stateLabelOf(commit.state);
  const keys = cleanKeys(c.proof_card?.facts?.keys ?? c.row?.verdict?.keys);
  const kind = c.proof_card?.facts?.kind ?? c.row?.verdict?.kind ?? null;
  const teeTier = c.proof_card?.facts?.tier ?? c.row?.verdict?.tier ?? null;
  const contract = (health.ok && strOf(health.data.contracts?.antibody)) || DEFAULT_ANTIBODY;
  const wasmSha = health.ok ? (strOf(health.data.contracts?.release?.verifier?.wasm_hash) ?? null) : null;

  const commitEv = events.find((e) => e.event === "commit");
  const baseNs = commitEv?.block_ts_ns ?? (commit.committed_at ? commit.committed_at * 1e9 : null);
  const requested = runEvent(events, "run_requested", runId);
  const answered = runEvent(events, "run_answered", runId);
  const t1Ev = runEvent(events, "t1_confirmed", runId);

  const run = rr && rr.ok ? rr.data : null;
  const outcome = run ? runOutcome(run.raw?.outcome) : null;
  const wrapD = outcome?.status === "answered" ? hasWrapD(outcome.a.extra) : null;
  const t2find = findT2(runId, run?.t2 ?? null, t2, t2NotYet(t2));
  const t2Confirmed = t2find.record?.outcome === "confirmed";
  const t2Contradicts = Boolean(t2find.record) && !t2Confirmed;
  const tierShown = t2Confirmed ? "T2" : teeTier;

  const disclosedEv = events.find((e) => e.event === "disclosed");
  const sealed = c.proof_card?.sealed_until_disclosure !== false && !disclosedEv;
  const rows = fr.ok && Array.isArray(fr.data?.submissions) ? fr.data.submissions : Array.isArray(program?.commits) ? program.commits : null;
  const feedError = fr.ok ? null : fr.error;

  // A duplicate shows the claim it duplicates: read that commit too.
  const dupOf = numOf(evData(events.find((e) => e.event === "duplicate")).of) ?? numOf(stateData(commit.state)?.of);
  const isDuplicate = label === "Duplicate" || label === "Done:DuplicateConfirmed" || dupOf !== null;
  const claim = isDuplicate && rows ? earlierClaim(rows, id, keys, dupOf) : null;
  const earlier = claim ? await api.commit(claim.row.commit_id) : null;

  const permits: PausePermit[] =
    rx && rx.ok && Array.isArray(rx.data?.chains)
      ? rx.data.chains.flatMap((ch) =>
          (Array.isArray(ch.permits) ? ch.permits : [])
            .filter((p) => p?.body?.Pause?.ticket === id)
            .map((p) => ({ chainId: ch.chain_id, status: p.status ?? null, tx: p.tx ?? null, explorer: p.explorer ?? null })),
        )
      : [];
  const moment = moments.ok && Array.isArray(moments.data?.moments) ? moments.data.moments.some((m) => m.commit_id === id && m.program_id === programId) : false;

  const windows = program?.program?.params?.windows;
  const round = numOf(commit.round) ?? 0;
  const baseSha =
    round === 0 ? (program?.meta?.base?.sha256 ?? null) : (strOf((program?.meta?.round_bases as { sha256?: string }[] | undefined)?.[round - 1]?.sha256) ?? null);
  const manifestSha =
    round === 0 ? (program?.program?.manifest_sha256 ?? null) : (program?.program?.rounds?.find((r) => r.round === round)?.manifest_sha256 ?? null);
  const bound: Bound = {
    requestSha: strOf(run?.raw?.request_sha256) ?? strOf(evData(requested).request_sha256),
    runId,
    round,
    commitId: id,
    blobSha: commit.blob_sha256 ?? null,
    baseSha,
    manifestSha,
    anchor: program?.rules?.anchor
      ? { chainId: numOf(program.rules.anchor.chain_id), block: numOf(program.rules.anchor.block), hash: strOf(program.rules.anchor.hash) }
      : null,
    programKey: strOf(program?.program?.key),
  };
  const severity = topSeverity(program, keys);
  const lede = summary({ c, label, kind, keys, program, events, baseNs, t1: Boolean(t1Ev) || teeTier === "T1" });

  return (
    <>
      <PageHead crumbs={crumbs(id, programId)} eyebrow={`Commit · program ${programId} · round ${commit.round ?? "—"}`} title={`Commit ${id}`} lede={lede}>
        <StateBadge state={label} />
        {kind || label !== "Committed" ? <KindBadge kind={kind} /> : null}
        <TierBadge tier={tierShown} />
        {severity ? <SeverityBadge severity={severity} /> : null}
        {info.practice ? (
          <Pill tone="info" icon="info">
            practice
          </Pill>
        ) : null}
        {info.earlier ? <Pill icon="clock">earlier run</Pill> : null}
      </PageHead>
      <nav className={`page ${styles.jumpNav}`} aria-label="On this page">
        <ul className={styles.jump}>
          <li>
            <a href="#outcome">Outcome</a>
          </li>
          <li>
            <a href="#timeline">Timeline</a>
          </li>
          <li>
            <a href="#proof">The proof</a>
          </li>
          <li>
            <a href="#tiers">Tiers</a>
          </li>
          <li>
            <a href="#siblings">Other commits</a>
          </li>
        </ul>
      </nav>

      <CommitSummary
        c={c}
        label={label}
        kind={kind}
        keys={keys}
        tier={tierShown}
        t2Contradicts={t2Contradicts}
        severity={severity}
        program={program}
        info={info}
        moment={moment}
      />
      {!pr.ok ? (
        <div className={`page ${styles.spaced}`}>
          <ErrorState what={`program ${programId}`} error={pr.error}>
            <span className="small">Slot names, the pause ticket and the known issues come from the program; the rest of this page stands without it.</span>
          </ErrorState>
        </div>
      ) : null}

      <Section id="outcome" title="What it settled" lede={`The terms program ${programId} committed, applied to this verdict: the bond, the payout, and why.`}>
        <Outcome
          c={c}
          label={label}
          keys={keys}
          kind={kind}
          program={program}
          programEvents={programEvents}
          events={events}
          rows={rows}
          feedError={feedError}
          earlier={earlier}
          contract={contract}
        />
      </Section>

      <Section id="timeline" title="Timeline" lede="Every step is a NEAR transaction. Times are block times, measured from the commit's block; hover a time for its UTC.">
        <Timeline
          commitId={id}
          hunter={commit.hunter}
          project={strOf(program?.program?.project)}
          beneficiary={commit.beneficiary}
          bond={commit.bond ?? null}
          round={round}
          events={events}
          pauseEvents={pauseEvents}
          permits={permits}
          baseNs={baseNs}
          label={label}
          hasKeys={keys.length > 0}
          disclosed={!sealed}
          windows={{
            reveal: numOf(windows?.reveal),
            challenge: numOf(windows?.challenge),
            disclosure: numOf(windows?.disclosure_max_delay),
          }}
          program={program}
        />
      </Section>

      <Section id="proof" title="The proof" lede="What anyone can see of a sealed proof, and how to check each part of it.">
        <ProofCard
          commitId={id}
          runId={runId}
          receipt={c.proof_card?.receipt_sha256 ?? null}
          commitment={strOf(commit.commitment)}
          blobSha={commit.blob_sha256 ?? null}
          sealed={sealed}
          wrapD={wrapD}
          outcome={label === "Done:Paid" ? "paid" : isDuplicate ? "duplicate" : label === "Done:Known" ? "known" : "other"}
          checks={Array.isArray(c.proof_card?.check) ? c.proof_card.check.filter((s): s is string => typeof s === "string") : []}
          requested={requested}
          disclosed={disclosedEv}
          contract={contract}
        />
      </Section>

      <Section
        id="tiers"
        title="Who vouches for the verdict"
        lede="Each tier is a separate check you can run yourself. T0 and T1 decide payouts; T2 sits beside them."
      >
        {rr && !rr.ok ? (
          <div style={{ marginBottom: 16 }}>
            <ErrorState what={`run ${runId}`} error={rr.error}>
              <span className="small">The tiers below use the commit&apos;s own record and events.</span>
            </ErrorState>
          </div>
        ) : null}
        <Tiers
          runId={runId}
          tier={teeTier}
          kind={kind}
          receipt={c.proof_card?.receipt_sha256 ?? null}
          answered={answered}
          t1={t1Ev}
          requested={requested}
          signer={strOf(requested?.signer) ?? strOf(run?.raw?.signer) ?? commit.hunter}
          contract={contract}
          wasmSha={wasmSha}
          t2={t2find}
          bound={bound}
        />
      </Section>

      <Section id="siblings" title={`Other commits in program ${programId}`} lede="The sealed feed: who committed when, the state and the verdict. Never a proof's content.">
        <Siblings rows={rows} current={id} programId={programId} error={feedError} />
      </Section>
    </>
  );
}
