import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import {
  DEFAULT_ANTIBODY,
  evData,
  findT2,
  isObj,
  isVerdictMode,
  keyState,
  kindWords,
  modeName,
  numOf,
  parseId,
  programInfo,
  runEvent,
  runOfEvent,
  rulesText,
  runOutcome,
  sortedEvents,
  strOf,
  unknownId,
  type Answered,
} from "@/components/commit/model";
import { RawRun, RunFacts } from "@/components/commit/RunFacts";
import { Tiers, type Bound } from "@/components/commit/Tiers";
import { Timeline } from "@/components/commit/Timeline";
import { KindBadge, TierBadge } from "@/components/ui/Badges";
import { PageHead, Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { ErrorState } from "@/components/ui/States";
import { api, t2NotYet, type IndexedEvent, type ProgramDetail } from "@/lib/api";
import { LEVEL, levelOf } from "@/lib/levels";

type Props = { params: Promise<{ id: string }> };

const crumbs = (id: number, program: number | null, commit: number | null) => [
  { href: "/programs", label: "Programs" },
  ...(program !== null ? [{ href: `/programs/${program}`, label: `Program ${program}` }] : []),
  ...(commit !== null ? [{ href: `/commits/${commit}`, label: `Commit ${commit}` }] : []),
  { href: `/runs/${id}`, label: `Run ${id}` },
];

/** A round check's answer, from its facts' `RoundCheck` (spec §8.1). */
function roundCheck(a: Answered | null): { ok: boolean; canaries: boolean } | null {
  const rc = a && isObj(a.extra) && isObj(a.extra.RoundCheck) ? a.extra.RoundCheck : null;
  return rc ? { ok: rc.ok === true, canaries: rc.canaries_ok === true } : null;
}

/**
 * The round a round check's proposal opened, with its release gate (`program.rounds`): the proposal is the
 * `round_proposed` event that names this run, and a round opens with the last proposal for its number.
 */
function checkedRound(run: number, program: ProgramDetail | null, events: IndexedEvent[]): { round: number; holds: boolean } | null {
  const proposed = events.find((e) => e.event === "round_proposed" && runOfEvent(e) === run);
  const round = numOf(evData(proposed).round);
  if (round === null) return null;
  const last = events.filter((e) => e.event === "round_proposed" && numOf(evData(e).round) === round).at(-1);
  if (last && runOfEvent(last) !== run) return null;
  const opened = program?.program?.rounds?.find((r) => r.round === round);
  return opened && typeof opened.release_gate === "boolean" ? { round, holds: opened.release_gate } : null;
}

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const id = parseId((await params).id);
  if (id === null) return { title: "Run not found" };
  const r = await api.run(id);
  if (!r.ok) return { title: `Run ${id}`, description: `Verification run ${id} on NEAR testnet: what OutLayer's enclave answered, how far it is checked, and every transaction.` };
  const o = runOutcome(r.data.raw?.outcome);
  const mode = strOf(r.data.raw?.mode) ?? strOf(r.data.run?.mode);
  // A run that judges no proof answers with a fixed NoViolation: its title names no verdict.
  const kind =
    o.status === "failed"
      ? "Failed"
      : o.status === "pending"
        ? "In flight"
        : isVerdictMode(mode) || o.a.kind !== "NoViolation"
          ? kindWords(o.a.kind)
          : null;
  const tier = strOf(r.data.tier) ?? strOf(r.data.run?.tier);
  const title = `Run ${id} · ${modeName(mode)}${kind ? ` · ${kind}` : ""}${tier ? ` · ${LEVEL[levelOf(tier)].name}` : ""}`;
  const description = `A ${modeName(mode)} run on NEAR testnet${kind ? `: ${kind}` : ""}${tier ? `, ${LEVEL[levelOf(tier)].prose}` : ""}, program ${r.data.raw?.program ?? "—"}${r.data.raw?.commit !== null && r.data.raw?.commit !== undefined ? `, commit ${r.data.raw.commit}` : ""}.`;
  return {
    title,
    description,
    openGraph: { type: "website", siteName: "Antibody", title, description },
    twitter: { card: "summary_large_image", title, description },
  };
}

function lede(x: {
  id: number;
  mode: string | null;
  a: Answered | null;
  failed: string | null;
  commit: number | null;
  program: number | null;
  round: number | null;
  p: ProgramDetail | null;
  /** The run is the project's own known-issue declaration (`program.known[].run`). */
  knownIssue: boolean;
}) {
  const slots = x.a ? rulesText(x.p, x.a.keys.map((k) => k.slot)) : "";
  const verdict = x.failed ? `failed (${x.failed})` : x.a ? `${kindWords(x.a.kind)}${slots ? ` of ${slots}` : ""}` : "in flight";
  const c = x.commit !== null ? `commit ${x.commit}'s` : "a";
  switch (x.mode) {
    case "verify":
      if (x.knownIssue) return `The project's sealed declaration of a known issue for program ${x.program ?? "—"}, verified in the enclave before the program opened: ${verdict}.`;
      return `The enclave's verification of ${c} sealed proof: ${verdict}.`;
    case "second":
      return `A second, independent run of ${c} proof: ${verdict}.`;
    case "rerun":
      return `A re-run of ${c} proof at round ${x.round ?? "—"}'s code and rules: ${
        x.a?.kind === "NoViolation" ? "it breaks nothing, so the fix stops it" : x.a?.kind === "Violation" ? `it still breaks ${slots || "a rule"}` : verdict
      }.`;
    case "release_wrap":
      return `The release wrap for ${x.commit !== null ? `commit ${x.commit}` : "a paid finding"}: after payment, the proof's content key wrapped to the project's own key, so the project reads exactly what it paid for.`;
    case "activate":
      return `Program ${x.program ?? "—"}'s activation: the enclave derived the program's keys and output its sealing public key.`;
    case "check_round": {
      const rc = roundCheck(x.a);
      const said = x.failed
        ? verdict
        : rc
          ? rc.ok
            ? `a valid round${rc.canaries ? ", its canaries still succeeding" : ", but a canary fails"}`
            : "refused"
          : x.a && x.a.kind !== "NoViolation"
            ? kindWords(x.a.kind)
            : "in flight";
      return `The enclave's check of round ${x.round ?? "—"}'s proposed fix for program ${x.program ?? "—"}: ${said}.`;
    }
    case "disclose":
      return `A disclosure run for program ${x.program ?? "—"}: the enclave released the disclosure keys of the tags it was asked for.`;
    default:
      return `Run ${x.id} of program ${x.program ?? "—"}: ${verdict}.`;
  }
}

export default async function RunPage({ params }: Props) {
  const id = parseId((await params).id);
  if (id === null) notFound();

  const [rr, health, t2] = await Promise.all([api.run(id), api.health(), api.t2()]);
  if (!rr.ok) {
    if (unknownId(rr)) notFound();
    return (
      <>
        <PageHead crumbs={crumbs(id, null, null)} eyebrow="Run" title={`Run ${id}`} />
        <div className="page">
          <ErrorState what={`run ${id}`} error={rr.error} />
        </div>
      </>
    );
  }

  const d = rr.data;
  const raw = d.raw;
  const programId = numOf(raw?.program);
  const commitId = numOf(raw?.commit) ?? numOf(d.run?.commit_id);
  const [pr, cr] = await Promise.all([
    programId !== null ? api.program(programId) : Promise.resolve(null),
    commitId !== null ? api.commit(commitId) : Promise.resolve(null),
  ]);
  const program = pr && pr.ok ? pr.data : null;
  const commit = cr && cr.ok ? cr.data : null;
  const commitEvents = commit ? sortedEvents(commit.events) : [];
  const programEvents = program ? sortedEvents(program.events) : [];
  const requested = runEvent(commitEvents, "run_requested", id) ?? runEvent(programEvents, "run_requested", id);
  // A program's own run (an activation, a round check) has no commit: its later events are read from the
  // event log right after its request. A failed read leaves only the request, and the Timeline says why.
  const near =
    commitId === null && requested ? await api.events(Math.max(0, (numOf(requested.seq) ?? 1) - 1), 60) : null;
  const logEvents = near && near.ok ? sortedEvents(near.data?.events) : [];
  const answered = runEvent(commitEvents, "run_answered", id) ?? runEvent(logEvents, "run_answered", id);
  const t1Ev = runEvent(commitEvents, "t1_confirmed", id) ?? runEvent(logEvents, "t1_confirmed", id);

  const o = runOutcome(raw?.outcome);
  const a = o.status === "answered" ? o.a : null;
  const failed = o.status === "failed" ? o.reason : null;
  const mode = strOf(raw?.mode) ?? strOf(d.run?.mode);
  const verdict = isVerdictMode(mode);
  // A known issue is declared like a submission (a verify run) but with no commit: the program names its run.
  const declared = program?.program?.known;
  const knownIssue = commitId === null && mode === "verify" && Array.isArray(declared) && declared.some((k) => numOf(k?.run) === id);
  const round = numOf(raw?.round);
  const teeTier = a?.tier ?? strOf(d.run?.tier);
  const t2find = findT2(id, d.t2, t2, t2NotYet(t2));
  const t2Confirmed = t2find.record?.outcome === "confirmed";
  const tierShown = t2Confirmed ? "T2" : teeTier;
  const contract = (health.ok && strOf(health.data.contracts?.antibody)) || DEFAULT_ANTIBODY;
  const wasmSha = health.ok ? (strOf(health.data.contracts?.release?.verifier?.wasm_hash) ?? null) : null;
  const requestSha = strOf(raw?.request_sha256);
  const info = programId !== null ? programInfo(programId, program) : null;
  const keys = keyState({
    disclosed: commitEvents.some((e) => e.event === "disclosed"),
    commitId,
    runId: id,
    programId,
    receipt: a?.receipt,
    practice: info?.practice ?? false,
  });

  // The run's own steps, timed from its commit's block (a program's own run: from its request).
  const commitEv = commitEvents.find((e) => e.event === "commit");
  const runSteps: IndexedEvent[] =
    commitId !== null
      ? commitEvents.filter((e) => e.event === "commit" || runOfEvent(e) === id)
      : [...(requested ? [requested] : []), ...logEvents.filter((e) => runOfEvent(e) === id && e.seq !== requested?.seq)];
  const baseNs = commitEv?.block_ts_ns ?? requested?.block_ts_ns ?? null;

  const r0 = round ?? 0;
  const bound: Bound = {
    requestSha,
    runId: id,
    round,
    commitId,
    blobSha: commit?.commit?.blob_sha256 ?? null,
    baseSha:
      r0 === 0 ? (program?.meta?.base?.sha256 ?? null) : (strOf((program?.meta?.round_bases as { sha256?: string }[] | undefined)?.[r0 - 1]?.sha256) ?? null),
    manifestSha: r0 === 0 ? (program?.program?.manifest_sha256 ?? null) : (program?.program?.rounds?.find((x) => x.round === r0)?.manifest_sha256 ?? null),
    anchor: program?.rules?.anchor
      ? { chainId: numOf(program.rules.anchor.chain_id), block: numOf(program.rules.anchor.block), hash: strOf(program.rules.anchor.hash) }
      : null,
    programKey: strOf(program?.program?.key),
  };

  const gate = mode === "check_round" ? checkedRound(id, program, programEvents) : null;

  // A known issue's run carries no disclosure wrap: a zero-knowledge proof, which re-runs a disclosed proof, has no "yet" for it.
  const t2Note = t2Confirmed ? (
    <Pill tone="t2" size="sm" title={LEVEL.T2.title}>
      ZK proof confirmed
    </Pill>
  ) : t2find.record ? (
    <Pill tone="bad" size="sm">
      ZK proof contradicts
    </Pill>
  ) : t2find.notYet && verdict && !knownIssue ? (
    <Pill tone="pending" size="sm">
      ZK proof: not yet on testnet
    </Pill>
  ) : null;

  return (
    <>
      <PageHead
        crumbs={crumbs(id, programId, commitId)}
        eyebrow={`Run · ${modeName(mode)}${programId !== null ? ` · program ${programId}` : ""}`}
        title={`Run ${id}`}
        lede={lede({ id, mode, a, failed, commit: commitId, program: programId, round, p: program, knownIssue })}
      >
        {failed ? (
          <Pill tone="warn">failed</Pill>
        ) : a ? (
          verdict || a.kind !== "NoViolation" ? (
            <KindBadge kind={a.kind} />
          ) : (
            <Pill tone="ok">answered</Pill>
          )
        ) : (
          <Pill tone="pending">in flight</Pill>
        )}
        <TierBadge tier={tierShown} />
        {gate && programId !== null ? (
          <Link href={`/programs/${programId}#rounds`} className="small tap">
            Round {gate.round}: release gate {gate.holds ? "holds" : "shut"}
          </Link>
        ) : null}
      </PageHead>

      {pr && !pr.ok ? (
        <div className="page" style={{ marginBottom: 16 }}>
          <ErrorState what={`program ${programId}`} error={pr.error} partial>
            <span className="small">Rule names come from the program; the run&apos;s own record below stands without it.</span>
          </ErrorState>
        </div>
      ) : null}

      <Section
        id="facts"
        title="What the enclave answered"
        lede={
          verdict
            ? "The run as near-antibody records it: every hash below is checkable against NEAR."
            : `The run as near-antibody records it: ${/^[aeiou]/.test(modeName(mode)) ? "an" : "a"} ${modeName(mode)}'s output, not a verdict. Every hash below is checkable against NEAR.`
        }
      >
        <RunFacts
          mode={mode}
          a={a}
          failed={failed}
          tier={tierShown}
          t2Note={t2Note}
          requestSha={requestSha}
          startedAt={numOf(raw?.started_at)}
          commitId={commitId}
          programId={programId}
          round={round}
          payer={strOf(raw?.payer)}
          signer={strOf(raw?.signer)}
          program={program}
          requested={requested}
          answered={answered}
        />
      </Section>

      {runSteps.length ? (
        <Section
          id="timeline"
          title="Timeline"
          lede={`Every step is a NEAR transaction, timed by block from ${commitId !== null ? `commit ${commitId}'s block` : "the run's request"}.`}
        >
          <div className="stack">
            {/* A program's own run reads its later steps from the event log: without that read the Timeline
                shows the request alone, which would pass for a run that never got further. */}
            {near && !near.ok ? (
              <ErrorState partial what="this run's later steps (/v1/events)" error={near.error}>
                <span className="small">
                  Only its request is shown: its answer and attestation steps come from the event log, which this page reads again on its next
                  load. The run&apos;s own record and its checks stand without them; only the links to those transactions are missing.
                </span>
              </ErrorState>
            ) : null}
            <Timeline
              commitId={commitId ?? 0}
              hunter={commit?.commit?.hunter ?? ""}
              project={strOf(program?.program?.project)}
              beneficiary={commit?.commit?.beneficiary ?? ""}
              bond={commit?.commit?.bond ?? null}
              round={numOf(commit?.commit?.round) ?? 0}
              events={runSteps}
              pauseEvents={[]}
              permits={[]}
              baseNs={baseNs}
              label={o.status === "pending" ? "" : o.status}
              hasKeys={false}
              disclosed={keys.kind === "disclosed"}
              windows={{ reveal: null, challenge: null, disclosure: null }}
              program={program}
              programId={programId}
              practice={info?.practice ?? false}
              runOnly
              baseWhat={commitId !== null ? "the commit's block" : "the run's request"}
            />
          </div>
        </Section>
      ) : null}

      <Section
        id="tiers"
        title="How it is checked"
        lede="Three levels, each a check you can run yourself. The first two decide payouts; the zero-knowledge proof sits beside them."
      >
        <Tiers
          runId={id}
          tier={teeTier}
          kind={a?.kind ?? null}
          receipt={a?.receipt ?? null}
          failed={failed}
          answered={answered}
          t1={t1Ev}
          requested={requested}
          signer={strOf(requested?.signer) ?? strOf(raw?.signer)}
          contract={contract}
          wasmSha={wasmSha}
          t2={t2find}
          bound={bound}
          mode={mode}
          keyState={keys}
        />
      </Section>

      <Section id="raw" title="The raw record" lede="Exactly what the API read from near-antibody's get_run view, for your own tools.">
        <RawRun id={id} raw={raw} />
      </Section>
    </>
  );
}
