import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import recordedMoments from "@/data/moments.json";
import { AnchorSection } from "@/components/programs/AnchorSection";
import { ChainsSection } from "@/components/programs/ChainsSection";
import { CommitFeed } from "@/components/programs/CommitFeed";
import { contractsOf, getProgram, getPrograms, getT2 } from "@/components/programs/data";
import { GuardSection } from "@/components/programs/GuardSection";
import { parseProgramId, programKind, RECORDED_RUN, runGroup, slotsOf, type ProgramKind } from "@/components/programs/helpers";
import { KnownIssues } from "@/components/programs/KnownIssues";
import { LedgerSection } from "@/components/programs/LedgerSection";
import { OnThisPage } from "@/components/programs/OnThisPage";
import { Overview } from "@/components/programs/Overview";
import { PracticeSection } from "@/components/programs/PracticeSection";
import { AnchorBadge, PhaseBadge, ProgramKindBadge, RunBadge } from "@/components/programs/ProgramBadges";
import { GUARD } from "@/components/programs/recorded";
import { RoundsSection } from "@/components/programs/RoundsSection";
import { RulesSection } from "@/components/programs/RulesSection";
import { TermsSection } from "@/components/programs/TermsSection";
import { Icon } from "@/components/ui/Icon";
import { PageHead } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { ErrorState } from "@/components/ui/States";

type Props = { params: Promise<{ id: string }> };

const DESCRIBE: Record<ProgramKind, string> = {
  fixture: "Acme Lend, our deliberately vulnerable fixture on NEAR testnet",
  practice: "The practice program, a tutorial on NEAR testnet",
  fork: "The practice program in fork mode, a tutorial on NEAR testnet",
  program: "A program on NEAR testnet",
};

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const { id } = await params;
  const pid = parseProgramId(id);
  if (pid === null) return { title: "No such program" };
  const r = await getProgram(pid);
  if (!r.ok) return { title: /does not exist/i.test(r.error) ? "No such program" : `Program ${pid}` };
  const name = r.data?.meta?.name || `Program ${pid}`;
  const kind = programKind(name, r.data?.meta?.description);
  return {
    title: `Program ${pid}: ${name}`,
    description: `${DESCRIBE[kind]}: its rules as code, terms, anchor, known issues, chains, sealed feed, rounds and ledger, each with a check you can run.`,
  };
}

export default async function ProgramPage({ params }: Props) {
  const { id } = await params;
  const pid = parseProgramId(id);
  if (pid === null) notFound();
  const crumbs = [
    { href: "/programs", label: "Programs" },
    { href: `/programs/${pid}`, label: `Program ${pid}` },
  ];

  const [r, contracts, t2] = await Promise.all([getProgram(pid), contractsOf(), getT2()]);
  if (!r.ok || !r.data?.program) {
    if (!r.ok && /does not exist/i.test(r.error)) notFound();
    const list = await getPrograms();
    if (list.ok && !(list.data?.programs ?? []).some((p) => p.program_id === pid)) notFound();
    return (
      <>
        <PageHead crumbs={crumbs} eyebrow={`Program ${pid}`} title={`Program ${pid}`} />
        <section className="page block">
          <ErrorState what={`program ${pid}`} error={r.ok ? "the API answered without the program" : r.error} />
        </section>
      </>
    );
  }

  const d = r.data;
  const name = d.meta?.name || `Program ${pid}`;
  const kind = programKind(name, d.meta?.description);
  const practice = kind === "practice" || kind === "fork";
  const slots = slotsOf(d, kind);
  const group = runGroup(pid);
  const moment = recordedMoments.moments.some((m) => m.program_id === pid);
  const guard = pid === GUARD.watched_program;
  const toc = [
    { id: "overview", label: "Overview" },
    ...(practice ? [{ id: "try", label: "Try it" }] : []),
    { id: "rules", label: "Rules" },
    { id: "terms", label: "Terms" },
    { id: "anchor", label: "Anchor" },
    { id: "known", label: "Known issues" },
    { id: "chains", label: "Chains" },
    { id: "feed", label: practice ? "Tries" : "Sealed feed" },
    { id: "rounds", label: "Rounds" },
    { id: "ledger", label: "Ledger" },
    ...(guard ? [{ id: "guard", label: "Who acts on it" }] : []),
  ];

  return (
    <>
      <PageHead crumbs={crumbs} eyebrow={`Program ${pid}`} title={name} lede={d.meta?.description || undefined}>
        <ProgramKindBadge kind={kind} long />
        <PhaseBadge phase={d.program?.phase} />
        <Pill>round {typeof d.program?.round === "number" ? d.program.round : "—"}</Pill>
        <AnchorBadge check={d.rules?.anchor?.check} />
        <RunBadge id={pid} />
        {practice ? (
          <Link href="/try" className="btn btn-sm">
            Try it, no account <Icon name="arrow" size={12} />
          </Link>
        ) : null}
      </PageHead>

      <div className="page stack" style={{ ["--gap" as string]: "10px" }}>
        {kind === "fixture" ? (
          <div className="callout callout-warn" role="note">
            <Icon name="alert" size={16} />
            <span>
              Acme Lend is our deliberately vulnerable fixture: a fictional lending protocol whose planted bugs are textbook
              classes, labelled in its code. Nothing here is a finding in a real protocol, and wNEAR on testnet has no value.
            </span>
          </div>
        ) : null}
        {group === "earlier" ? (
          <div className="callout" role="note">
            <Icon name="clock" size={16} />
            <span>
              An earlier run on the same locked deployment, kept on NEAR. The recorded run ({RECORDED_RUN.date}) is programs{" "}
              <Link href="/programs/5">5 (Acme Lend)</Link>, <Link href="/programs/6">6 (practice)</Link> and{" "}
              <Link href="/programs/7">7 (fork mode)</Link>.
            </span>
          </div>
        ) : null}
        {moment ? (
          <div className="callout" role="note">
            <Icon name="bolt" size={16} />
            <span>
              The one moment happened here: agent A&apos;s sealed proof was verified, paid, and paused the fixture on both chains.
              It is recorded, not live. <Link href="/moment">Every step, timed</Link>.
            </span>
          </div>
        ) : null}
      </div>

      <OnThisPage items={toc} />

      <Overview d={d} kind={kind} id={pid} antibody={contracts.antibody} />
      {practice ? <PracticeSection d={d} id={pid} fork={kind === "fork"} practiceProgram={contracts.practiceProgram} /> : null}
      <RulesSection d={d} kind={kind} slots={slots} id={pid} />
      <TermsSection d={d} slots={slots} />
      <AnchorSection d={d} id={pid} lightClient={contracts.lightClient} />
      <KnownIssues d={d} slots={slots} id={pid} antibody={contracts.antibody} />
      <ChainsSection d={d} id={pid} kind={kind} moment={moment} />
      <CommitFeed d={d} id={pid} kind={kind} slots={slots} t2={t2} />
      <RoundsSection d={d} id={pid} slots={slots} antibody={contracts.antibody} />
      <LedgerSection d={d} id={pid} antibody={contracts.antibody} />
      {guard ? <GuardSection guard={contracts.verdictGuard} /> : null}
    </>
  );
}
