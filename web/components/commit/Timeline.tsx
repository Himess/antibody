import Link from "next/link";
import type { ReactNode } from "react";
import { RECORDED_RUN } from "@/components/programs/helpers";
import { Hash } from "@/components/ui/Hash";
import { Empty } from "@/components/ui/States";
import { TxLink } from "@/components/ui/TxLink";
import type { IndexedEvent, ProgramDetail } from "@/lib/api";
import { chainName, sourceUrl } from "@/lib/config";
import { cap, isoOf, seconds, utc, wnear } from "@/lib/format";
import { LEVEL } from "@/lib/levels";
import {
  cleanKeys,
  evData,
  isVerdictMode,
  kindWords,
  modeName,
  modeOfEvent,
  numOf,
  offset,
  recordedNote,
  rulesText,
  runOfEvent,
  secondsBetween,
  strOf,
  usd,
  verdictWords,
  who,
} from "./model";
import styles from "./Timeline.module.css";

type Mark = "ok" | "accent" | "t1" | "warn" | "neutral";

interface Step {
  key: string;
  tone: "main" | "minor" | "pending";
  mark: Mark;
  title: ReactNode;
  detail?: ReactNode;
  ev?: IndexedEvent;
}

export interface PausePermit {
  chainId: number;
  status: string | null;
  tx: string | null;
  explorer: string | null;
}

export interface TimelineInput {
  commitId: number;
  hunter: string;
  /** The program's project account: named "the project" wherever it acts. */
  project: string | null;
  beneficiary: string;
  bond: string | null;
  round: number;
  /** The commit's own events (they name it), in seq order. */
  events: IndexedEvent[];
  /** The program's pause ticket for this commit, if it issued one (its event names the ticket, not the commit). */
  pauseEvents: IndexedEvent[];
  permits: PausePermit[];
  /** The commit's block time (ns): every offset is measured from it. */
  baseNs: number | null;
  label: string;
  hasKeys: boolean;
  disclosed: boolean;
  windows: { reveal: number | null; challenge: number | null; disclosure: number | null };
  program: ProgramDetail | null;
  /** The program's id, read even when its record did not load: it tells an earlier run's practice try. */
  programId?: number | null;
  /** On the practice program, a try may be sealed under a public content key: no "nobody can read it". */
  practice?: boolean;
  /** Transactions this commit shares with other commits (an allocation pass settles several): tx → their ids. */
  sharedTx?: Record<string, number[]>;
  /** One run's steps only (the run page): no promise about the commit's later states. */
  runOnly?: boolean;
  /** What the offsets are measured from (default: the commit's block; a program's own run: its request). */
  baseWhat?: string;
}

/**
 * Who acted, with the account kept on one line where it fits ("our fixture agent A (agent-a.antibody.testnet)").
 * `tail` is the punctuation after it ("." or ","), kept inside the account's box: a line may break right after
 * that inline-block, and a "." must not start the next line.
 */
function actor(project: string | null, program: number | null, account: string | null | undefined, capital: boolean, tail = ""): ReactNode {
  if (!account) return `${capital ? "The hunter" : "the hunter"}${tail}`;
  const w = project && account === project ? "the project" : who(account, program);
  const acct = (
    <span className={styles.acct}>
      {w ? `(${account})` : account}
      {tail}
    </span>
  );
  return w ? (
    <>
      {capital ? cap(w) : w} {acct}
    </>
  ) : (
    acct
  );
}

const programOf = (x: TimelineInput) => x.programId ?? x.program?.program?.id ?? null;

const actorWith =
  (x: TimelineInput) =>
  (account: string | null | undefined, capital = false, tail = "") =>
    actor(x.project, programOf(x), account, capital, tail);

/** This commit is the recorded run's (the program and the hunter match, as for its note under "What it settled"). */
const recordedRun = (x: TimelineInput) => {
  const program = programOf(x);
  return program !== null && recordedNote(x.commitId, program, x.hunter) !== null;
};

/** The rules a run's keys break, in prose: "the solvency rule". */
function rulesOf(program: ProgramDetail | null, v: unknown): string {
  return rulesText(
    program,
    cleanKeys(v).map((k) => k.slot),
  );
}

/** A level's name in a step's title, its code in the tooltip ("Enclave verdict", "Attested on NEAR"). */
const Lv = ({ level }: { level: keyof typeof LEVEL }) => <span title={LEVEL[level].title}>{LEVEL[level].name}</span>;

/** One event's step. Grouped events (a reveal and its run request, …) are built by `buildSteps`. */
function single(e: IndexedEvent, x: TimelineInput, rerunRound: Map<number, number>, promotedRound: number | null): Step {
  const by = actorWith(x);
  const d = evData(e);
  const run = runOfEvent(e);
  const mode = modeOfEvent(e);
  const key = `${e.seq}-${e.event}`;
  switch (e.event) {
    case "commit":
      return {
        key,
        tone: "main",
        mark: "neutral",
        ev: e,
        title: "Committed",
        detail: (
          <>
            A sealed proof{x.bond ? ` and a ${wnear(x.bond)} bond` : ""}, by {by(strOf(d.hunter) ?? x.hunter, false, x.round ? "," : ".")}
            {x.round ? ` in round ${x.round}.` : ""}
            {x.practice ? "" : " Nobody, the project included, can read it before its verdict."}
          </>
        ),
      };
    case "run_requested": {
      const payer = strOf(d.payer);
      if (mode === "release_wrap")
        return {
          key,
          tone: "minor",
          mark: "neutral",
          ev: e,
          title: `Release wrap requested · run ${run ?? "—"}`,
          detail: <>{payer ? <>{by(payer, true)} asked</> : "The project asked"} the enclave to wrap this proof&apos;s content key to the project&apos;s own key.</>,
        };
      if (mode === "second")
        return {
          key,
          tone: "minor",
          mark: "neutral",
          ev: e,
          title: `Second run requested · run ${run ?? "—"}`,
          detail: "Another run of the same proof: above the instant cap, a payout needs one that agrees.",
        };
      return {
        key,
        tone: mode === "verify" ? "main" : "minor",
        mark: "neutral",
        ev: e,
        title: `Run ${run ?? "—"} requested${mode && mode !== "verify" ? ` · ${modeName(mode)}` : ""}`,
        detail: payer ? <>Requested and paid for by {by(payer, false, ".")}</> : undefined,
      };
    }
    case "run_answered": {
      const kind = strOf(d.kind);
      const rules = rulesOf(x.program, d.keys);
      const receipt = strOf(d.receipt_sha256);
      // A run that judges no proof (an activation, a round check, …) answers with a fixed NoViolation: no verdict to name.
      if (!isVerdictMode(mode) && kind === "NoViolation")
        return {
          key,
          tone: "minor",
          mark: "neutral",
          ev: e,
          title: (
            <>
              {cap(modeName(mode))}: <Lv level="T0" />
            </>
          ),
          detail: "OutLayer's operator delivered the enclave's output to near-antibody: its output, not a verdict.",
        };
      if (mode === "rerun") {
        const r = run !== null ? rerunRound.get(run) : undefined;
        const at = r !== undefined ? `round ${r}'s fix` : "the fix";
        return {
          key,
          tone: "minor",
          mark: kind === "Violation" ? "accent" : "neutral",
          ev: e,
          title: `Re-run at ${at}: ${verdictWords(kind, rules)}`,
          detail:
            kind === "NoViolation"
              ? `At ${at} this proof breaks nothing: the fix stops it.`
              : kind === "Violation"
                ? `At ${at} it still breaks ${rules || "a rule"}.`
                : `The enclave's result at ${at}.`,
        };
      }
      if (mode === "second") return { key, tone: "minor", mark: "neutral", ev: e, title: `Second run answered: ${verdictWords(kind, rules)}` };
      return {
        key,
        tone: mode === "verify" ? "main" : "minor",
        mark: kind === "Violation" ? "accent" : "neutral",
        ev: e,
        title: (
          <>
            <Lv level="T0" />: {verdictWords(kind, rules)}
          </>
        ),
        detail: (
          <>
            OutLayer&apos;s operator delivered the enclave&apos;s output to near-antibody.
            {receipt ? (
              <>
                {" "}
                Receipt <Hash value={receipt} head={12} label="receipt sha256" />
              </>
            ) : null}
          </>
        ),
      };
    }
    case "t1_confirmed":
      if (mode && mode !== "verify")
        return {
          key,
          tone: "minor",
          mark: "t1",
          ev: e,
          title: (
            <>
              {cap(modeName(mode))} <span title={LEVEL.T1.title}>attested on NEAR</span> · run {run ?? "—"}
            </>
          ),
        };
      return {
        key,
        tone: "main",
        mark: "t1",
        ev: e,
        title: <Lv level="T1" />,
        detail: "The run's Intel TDX quote, verified on NEAR: it binds the pinned verifier, this run's request and the output near-antibody received.",
      };
    case "holder": {
      const amount = strOf(d.amount);
      const debt = strOf(d.debt);
      const n = Array.isArray(d.keys) ? d.keys.length : 0;
      return {
        key,
        tone: "main",
        mark: "ok",
        ev: e,
        title: "Allocated: first proof",
        detail: (
          <>
            The earliest final proof of {n === 1 ? "its key" : `${n || "its"} keys`} (a rule and an entry point)
            {amount ? `: ${wnear(amount)} reserved from the escrow` : ""}
            {debt && debt !== "0" ? `, ${wnear(debt)} owed` : ""}. The bond is returned.
          </>
        ),
      };
    }
    case "duplicate": {
      const of = numOf(d.of);
      const reserved = strOf(d.reserved);
      return {
        key,
        tone: "main",
        mark: "neutral",
        ev: e,
        title: of !== null ? `Duplicate of commit ${of}` : "Duplicate",
        detail: `Every key it proves was already held. Never paid; the bond is returned.${reserved ? ` ${wnear(reserved)} stays reserved in case a later fix promotes it.` : ""}`,
      };
    }
    case "duplicate_confirmed":
      return {
        key,
        tone: "main",
        mark: "neutral",
        ev: e,
        title: "Confirmed a duplicate",
        detail: "At the next round's opening: its reservation is released, and its proof opens at disclosure with the claim it duplicates.",
      };
    case "known_match":
      return {
        key,
        tone: "main",
        mark: "neutral",
        ev: e,
        title: "Known issue",
        detail: "It proves an issue the project declared, sealed, before opening. Never paid; the bond is returned.",
      };
    case "forfeited":
      return {
        key,
        tone: "main",
        mark: "warn",
        ev: e,
        title: "Bond forfeited",
        detail: `${kindWords(strOf(d.kind) ?? "NoViolation")}: the proof broke no rule. The bond goes to the program's escrow, where it pays future findings.`,
      };
    case "expired":
      return { key, tone: "main", mark: "warn", ev: e, title: "Expired", detail: "Not revealed in time: the bond goes to the program's escrow." };
    case "gap_claim":
      return {
        key,
        tone: "main",
        mark: "neutral",
        ev: e,
        title: "Gap claim recorded",
        detail: `No rule broke, but protected accounts lost ${usd(d.harm_usd_micro)} (fixture USD), above the program's threshold: the bond is returned and the claim is kept, sealed, at its priority.`,
      };
    case "gap_promoted":
      return {
        key,
        tone: "main",
        mark: "ok",
        ev: e,
        title: `Promoted at round ${numOf(d.round) ?? "—"}`,
        detail: "The round added a rule this proof breaks: the gap claim is paid at its original priority.",
      };
    case "promoted":
      return {
        key,
        tone: "main",
        mark: "ok",
        ev: e,
        title: `Promoted at round ${numOf(d.round) ?? "—"}`,
        detail: "Its re-run still breaks a key whose fix was released, so it proves a distinct bug: it becomes the key's holder and is paid at its original priority.",
      };
    case "paid": {
      const amount = strOf(d.amount);
      const fee = strOf(d.fee);
      const instant = d.instant === true;
      return {
        key,
        tone: "main",
        mark: "ok",
        ev: e,
        title: `Paid ${amount ? wnear(amount) : ""}${instant ? " · instant" : ""}`.trim(),
        detail: (
          <>
            {instant
              ? "Instant payout: attested on NEAR, under the instant cap and within its window's instant budget."
              : promotedRound !== null
                ? `At its original priority, after its promotion at round ${promotedRound}.`
                : "After its challenge window, with no challenge open."}{" "}
            Credited to {by(strOf(d.beneficiary) ?? x.beneficiary, false, fee && fee !== "0" ? ";" : ".")}
            {fee && fee !== "0" ? ` protocol fee ${wnear(fee)}.` : ""}
          </>
        ),
      };
    }
    case "frozen":
      return {
        key,
        tone: "minor",
        mark: "accent",
        ev: e,
        title: "A NEAR app acted on it",
        detail: (
          <>
            The verdict guard read this verdict from near-antibody and froze its treasury: attested on NEAR, a Critical rule, the first finding of its key (
            <a href={sourceUrl("docs/verdict-guard.md")} target="_blank" rel="noreferrer noopener">
              how it decides
            </a>
            ).
            {recordedRun(x) ? (
              <span className={styles.aside}>
                The guard was added on 2026-11-06, after the recorded run, and read its commits then (
                <a href={sourceUrl(RECORDED_RUN.results_file)} target="_blank" rel="noreferrer noopener">
                  results file
                </a>
                , <code>verdict_consumer.note</code>).
              </span>
            ) : null}
          </>
        ),
      };
    case "disclosed":
      return { key, tone: "main", mark: "ok", ev: e, title: "Disclosed", detail: "Its tag's disclosure key was released: anyone can open the proof and re-run it." };
    case "challenge":
      return { key, tone: "minor", mark: "warn", ev: e, title: "Challenged", detail: "Someone challenged the verdict within its window, with a challenge bond: re-execution settles it." };
    case "challenge_upheld":
      return { key, tone: "minor", mark: "neutral", ev: e, title: "Challenge: verdict upheld", detail: "The re-execution gave identical facts." };
    case "challenge_overturned":
      return { key, tone: "minor", mark: "warn", ev: e, title: "Challenge: verdict replaced", detail: "The re-execution gave different facts; they replace the verdict." };
    case "run_failed":
      return { key, tone: "minor", mark: "warn", ev: e, title: `Run ${run ?? ""} failed`.replace("  ", " "), detail: strOf(d.reason) ?? undefined };
    case "retry_available":
      return { key, tone: "minor", mark: "neutral", ev: e, title: "Retry available", detail: "The run may be requested again; the commit keeps its priority." };
    // A program's own runs (the run page): confirm_tee's relay, a round's proposal, a known issue's declaration.
    case "tee_confirm_requested":
      return {
        key,
        tone: "minor",
        mark: "neutral",
        ev: e,
        title: "TDX quote relayed",
        detail: "near-antibody checked that the attestation describes this run and sent its Intel TDX quote to NEAR's DCAP verifier.",
      };
    case "round_proposed":
      return {
        key,
        tone: "minor",
        mark: "neutral",
        ev: e,
        title: `Round ${numOf(d.round) ?? "—"} proposed`,
        detail: "The project proposed a fix; the enclave checks it in this run before any re-run.",
      };
    case "known_issue":
      return {
        key,
        tone: "main",
        mark: "neutral",
        ev: e,
        title: "Recorded as a known issue",
        detail:
          "Declared sealed before the program opened: a later proof with the same tag is never paid, and this proof stays sealed for good (a known issue's run carries no disclosure wrap).",
      };
    default:
      return { key, tone: "minor", mark: "neutral", ev: e, title: e.event.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase()) };
  }
}

function buildSteps(x: TimelineInput): Step[] {
  // The pause ticket's events name the ticket (the commit id), not `commit_id`: merge them in, once.
  const seen = new Set(x.events.map((e) => e.seq));
  const all = [...x.events, ...x.pauseEvents.filter((e) => !seen.has(e.seq))].sort((a, b) => (numOf(a.seq) ?? 0) - (numOf(b.seq) ?? 0));

  const promoted = all.find((e) => e.event === "gap_promoted" || e.event === "promoted");
  const promotedRound = promoted ? numOf(evData(promoted).round) : null;
  const by = actorWith(x);
  const rerunRound = new Map<number, number>();
  for (const e of all) {
    const r = runOfEvent(e);
    const round = numOf(evData(e).round);
    if (e.event === "rerun_requested" && r !== null && round !== null) rerunRound.set(r, round);
  }

  // Events of one transaction in one block form a bucket; some buckets read as one step.
  const buckets = new Map<string, IndexedEvent[]>();
  for (const e of all) {
    const k = `${e.tx}|${e.block_height}`;
    const b = buckets.get(k);
    if (b) b.push(e);
    else buckets.set(k, [e]);
  }

  const steps: Step[] = [];
  for (const bucket of buckets.values()) {
    const take = (name: string, pred?: (e: IndexedEvent) => boolean) => {
      const i = bucket.findIndex((e) => e.event === name && (!pred || pred(e)));
      return i >= 0 ? bucket.splice(i, 1)[0] : undefined;
    };
    const reveal = take("reveal");
    if (reveal) {
      const run = runOfEvent(reveal);
      const req = take("run_requested", (e) => runOfEvent(e) === run);
      const sha = strOf(evData(req).request_sha256);
      const payer = strOf(evData(req).payer);
      steps.push({
        key: `${reveal.seq}-reveal`,
        tone: "main",
        mark: "neutral",
        ev: reveal,
        title: `Revealed · run ${run ?? "—"} requested`,
        detail: (
          <>
            The sealed proof went to the enclave, in a request near-antibody built from its own records
            {sha ? (
              <>
                {" "}
                (request sha256 <Hash value={sha} head={12} label="request sha256" />)
              </>
            ) : null}
            .{payer && payer !== x.hunter ? <> The run was paid for by {by(payer, false, ".")}</> : null}
          </>
        ),
      });
    }
    const rerun = take("rerun_requested");
    if (rerun) {
      const run = runOfEvent(rerun);
      take("run_requested", (e) => runOfEvent(e) === run);
      const round = numOf(evData(rerun).round);
      steps.push({
        key: `${rerun.seq}-rerun`,
        tone: "minor",
        mark: "neutral",
        ev: rerun,
        title: `Re-run requested at round ${round ?? "—"}'s fix · run ${run ?? "—"}`,
        detail: "At a proposed fix, the keeper re-runs every paid proof (the release gate holds only if all of them fail against it) and every duplicate and gap claim (the fix may promote them).",
      });
    }
    const wrap = take("release_wrap");
    if (wrap) {
      const run = runOfEvent(wrap);
      take("run_answered", (e) => runOfEvent(e) === run);
      steps.push({
        key: `${wrap.seq}-wrap`,
        tone: "main",
        mark: "ok",
        ev: wrap,
        title: `Release wrap delivered · run ${run ?? "—"}`,
        detail: (
          <>
            The proof&apos;s content key, wrapped to the project&apos;s key: the project reads exactly what it paid for (
            <code>antibody program read</code> opens it and re-runs it locally). A duplicate is never wrapped.
          </>
        ),
      });
    }
    const ticket = take("pause_ticket");
    if (ticket) {
      take("permit_requested");
      const round = numOf(evData(ticket).round);
      steps.push({
        key: `${ticket.seq}-pause`,
        tone: "main",
        mark: "accent",
        ev: ticket,
        title: "Pause ticket",
        detail: (
          <>
            An attested first proof of a rule that pauses: near-antibody asked near-guardian for pause permits for every chain the program runs on
            {round !== null ? ` (round ${round})` : ""}; the MPC network signs them.
            {x.permits.length ? (
              <span className={styles.permits}>
                {x.permits.map((p) => (
                  <span key={p.chainId} className={styles.permit}>
                    {chainName(p.chainId)}: permit {p.status ?? "queued"}
                    {p.tx ? (
                      <>
                        {" "}
                        <TxLink hash={p.tx} chainId={p.chainId} href={p.explorer} />
                      </>
                    ) : null}
                  </span>
                ))}
              </span>
            ) : null}
          </>
        ),
      });
    }
    for (const e of bucket) steps.push(single(e, x, rerunRound, promotedRound));
  }

  // What has not happened yet, said plainly.
  const has = (name: string) => all.some((e) => e.event === name);
  const pending = (key: string, title: string, detail: string): Step => ({ key: `pending-${key}`, tone: "pending", mark: "neutral", title, detail });
  if (x.runOnly) {
    // `label` is the run's own status here: "" while it is in flight.
    if (x.label === "" && !has("run_answered") && !has("run_failed"))
      steps.push(pending("answer", LEVEL.T0.name, "The run is in flight: OutLayer's operator delivers the enclave's output to near-antibody."));
    return steps;
  }
  if (x.label === "Committed") {
    steps.push(
      pending(
        "reveal",
        "Reveal",
        `The hunter reveals the sealed proof${x.windows.reveal ? ` within ${seconds(x.windows.reveal, 0)} of the commit` : ""}; a commit never revealed expires and its bond goes to the escrow.`,
      ),
    );
  } else if (x.label === "Verifying") {
    if (!all.some((e) => e.event === "run_answered" && modeOfEvent(e) === "verify")) steps.push(pending("answer", LEVEL.T0.name, "The verification run is in flight."));
    else
      steps.push(
        pending(
          "final",
          "A final verdict",
          `Allocation waits for a final verdict: attested on NEAR under the instant cap, or the end of its challenge window${x.windows.challenge ? ` (${seconds(x.windows.challenge, 0)})` : ""}.`,
        ),
      );
  } else if (x.label === "Holder") {
    steps.push(pending("paid", "Payout", "Funded holders are paid on the keeper's next pass; an unfunded remainder is queued as debt, in commit order."));
  } else if (x.label === "Duplicate") {
    steps.push(pending("round", "The next round", "At the next round's opening it is confirmed a duplicate, or promoted if the fix stops the earlier claim but not this proof."));
  } else if (x.label === "GapClaim") {
    steps.push(pending("round", "A round that adds a rule", "If a later round adds a rule this proof breaks, the claim is paid at its original priority."));
  }
  // A known match has no holder whose payment could qualify its tag: no promise of disclosure for it.
  if (!x.disclosed && !has("disclosed") && x.hasKeys && x.label !== "Committed" && x.label !== "Done:Known") {
    steps.push(
      pending(
        "disclosure",
        "Disclosure",
        `Opens once its tag qualifies and anyone requests it: the fix ships through the release gate, or ${x.windows.disclosure ? seconds(x.windows.disclosure, 0) : "the program's disclosure delay"} after the tag's holder was paid.`,
      ),
    );
  }
  return steps;
}

/** "and", for short lists of links: "14, 16 and 17". */
function andList(items: ReactNode[]): ReactNode[] {
  return items.flatMap((it, i) => (i === 0 ? [it] : [i === items.length - 1 ? " and " : ", ", it]));
}

export function Timeline(props: TimelineInput) {
  const steps = buildSteps(props);
  if (!steps.some((s) => s.ev)) {
    return <Empty>No indexed events for this commit yet: the indexer may still be catching up. The commit itself is read from NEAR.</Empty>;
  }
  const items: ReactNode[] = [];
  let prev: number | null = null;
  let prevDay: string | null = null;
  for (const s of steps) {
    const at = s.ev?.block_ts_ns ?? null;
    const gap = secondsBetween(prev, at);
    if (gap !== null && gap > 3600) {
      items.push(
        <li key={`gap-${s.key}`} className={styles.gap} aria-hidden="true">
          <span className={styles.gapText}>{seconds(gap)} later</span>
        </li>,
      );
    }
    if (at) prev = at;
    // Each step prints its UTC time; the date too on the first step and whenever the day changes.
    const when = at ? utc(at, "ns") : null;
    const day = when ? when.slice(0, 10) : null;
    const showDay = day !== null && day !== prevDay;
    if (day) prevDay = day;
    const shared = s.ev && s.ev.event !== "commit" ? props.sharedTx?.[s.ev.tx] : undefined;
    items.push(
      <Row
        key={s.key}
        step={s}
        baseNs={props.baseNs}
        baseWhat={props.baseWhat ?? "the commit's block"}
        day={showDay ? day : null}
        time={when ? when.slice(11) : null}
        shared={shared}
      />,
    );
  }
  return (
    <ol
      className={styles.list}
      aria-label={`Every step of this ${props.runOnly ? "run" : "commit"}, with its NEAR transaction and its time from ${props.baseWhat ?? "the commit's block"}`}
    >
      {items}
    </ol>
  );
}

function Row({
  step,
  baseNs,
  baseWhat,
  day,
  time,
  shared,
}: {
  step: Step;
  baseNs: number | null;
  baseWhat: string;
  /** The UTC date, on the first step and when the day changes. */
  day: string | null;
  /** The UTC time of day ("15:42:29 UTC"). */
  time: string | null;
  shared?: number[];
}) {
  const e = step.ev;
  const s = e ? secondsBetween(baseNs, e.block_ts_ns) : null;
  const isBase = e?.event === "commit" || (s !== null && s === 0);
  return (
    <li className={`${styles.step} ${styles[step.tone]} ${styles[step.mark]}`}>
      <span className={`${styles.time} num`}>
        {e ? (
          <time dateTime={isoOf(e.block_ts_ns, "ns")}>
            {isBase ? "0.0 s" : offset(s)}
            <span className="visually-hidden">{isBase ? ` (${baseWhat})` : ` after ${baseWhat}`}</span>
          </time>
        ) : (
          <span className={styles.waiting}>next</span>
        )}
      </span>
      <span className={styles.rail} aria-hidden="true">
        <span className={styles.dot} />
      </span>
      <div className={styles.body}>
        <p className={styles.title}>
          {step.title}
          {step.tone === "pending" ? <span className="visually-hidden"> (not yet)</span> : null}
        </p>
        {step.detail ? <div className={styles.detail}>{step.detail}</div> : null}
        {shared?.length ? (
          <div className={styles.detail}>
            One allocation transaction settled this commit and commit{shared.length > 1 ? "s" : ""}{" "}
            {andList(
              shared.map((id) => (
                <Link key={id} href={`/commits/${id}`}>
                  {id}
                </Link>
              )),
            )}
            : the same hash appears on each.
          </div>
        ) : null}
      </div>
      <div className={styles.tx}>
        {e ? (
          <>
            <TxLink hash={e.tx} href={e.explorer} />
            <span className={`xsmall muted num ${styles.meta}`}>
              {day ? <span className={styles.nowrap}>{day}</span> : null}
              <span className={styles.nowrap}>
                {time ? `${time} · ` : ""}block {Number(e.block_height).toLocaleString("en-US")}
              </span>
            </span>
          </>
        ) : (
          <span className="xsmall muted">no transaction yet</span>
        )}
      </div>
    </li>
  );
}
