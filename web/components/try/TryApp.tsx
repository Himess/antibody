"use client";

import Link from "next/link";
import { type FormEvent, useEffect, useId, useRef, useState, useSyncExternalStore } from "react";
import { Icon } from "@/components/ui/Icon";
import { Pill } from "@/components/ui/Pill";
import { sourceUrl } from "@/lib/config";
import { relative, utc } from "@/lib/format";
import { type SavedTry, suggestName, type TrySnapshot } from "@/lib/practice/flow";
import { checkName } from "@/lib/practice/near-tx";
import { nextHourMs } from "@/lib/practice/vending";
import { BeforeStart, type PreState, type Readiness, readiness, resetAt } from "./BeforeStart";
import { BrowserLedger } from "./BrowserLedger";
import type { TryInitial } from "./initial";
import { RecordedTries } from "./RecordedTries";
import { SealedProof } from "./SealedProof";
import { StepList } from "./StepList";
import { tabStorage, tryStore } from "./store";
import { T2Row, TryResult } from "./TryResult";
import { useNow } from "./useNow";
import styles from "./try.module.css";

const SUFFIX = ".practice.antibody.testnet";

function nameProblem(name: string): string | null {
  if (!name) return "Pick a name: 1 to 32 of a–z, 0–9, - and _.";
  try {
    checkName(name);
    return null;
  } catch (e) {
    return e instanceof Error ? `${e.message.charAt(0).toUpperCase()}${e.message.slice(1)}.` : "Not a practice name.";
  }
}

/**
 * The try this tab kept. One stopped before its vend left no account: forgotten. The vending contract lets
 * anyone reclaim an account an hour after its vend, so an older one is offered for closing only.
 */
function loadSaved(): { record: SavedTry; stale: boolean } | null {
  const record = tabStorage.load();
  if (!record) return null;
  if (!record.closed && !record.vendSent && record.vault === undefined) {
    tabStorage.clear();
    return null;
  }
  return { record, stale: Date.now() - record.startedAt > 50 * 60_000 };
}

function StreamPill({ s }: { s: TrySnapshot }) {
  if (s.phase !== "running") return null;
  if (s.stream === "live") return <Pill tone="ok" icon="bolt" size="sm">live stream</Pill>;
  if (s.stream === "connecting") return <Pill tone="pending" size="sm">connecting to the stream</Pill>;
  if (s.stream === "polling") return <Pill tone="warn" icon="refresh" size="sm">polling NEAR</Pill>;
  return null;
}

/** What to do after a failure, by its kind. */
function FailureHelp({ s }: { s: TrySnapshot }) {
  const kind = s.error?.kind;
  if (kind === "capacity")
    return (
      <p className="small">
        This hour&apos;s practice accounts are vended, whichever client asks (the CLI too). The count resets at the top of the hour (UTC); meanwhile the recorded tries
        below show the same loop.
      </p>
    );
  if (kind === "faucet")
    return <p className="small">The practice faucet cannot pay for a try now; the operator refills it. Meanwhile: the recorded tries below, or the CLI with your own testnet account.</p>;
  if (kind === "rpc") return <p className="small">The public NEAR RPCs did not answer: they rate-limit. Wait a minute and start again with a new name, or use the CLI below.</p>;
  if (kind === "verdict") return <p className="small">The run did not give a verdict. Start again with a new name; the loop is the same.</p>;
  return <p className="small">Start again with a new name, or run the same try from the CLI below.</p>;
}

/** "program 6", once the vending contract has named it. */
const programShort = (r: Readiness) => (r.data ? `program ${r.data.programId}` : "the practice program");

/**
 * When attestation (T1) may not come before: the last of the worker images still inside their delay. It sits
 * in running text, so the stamp is kept whole (never its date on one line and its clock on the next).
 */
function T1Until({ r, now }: { r: Readiness; now: number | null }) {
  const last = r.pending[r.pending.length - 1];
  return (
    <>
      <time className="nowrap" dateTime={new Date(last.countsFrom * 1000).toISOString()}>{utc(last.countsFrom)}</time>
      {now !== null ? ` (${relative(last.countsFrom, now)})` : ""}
    </>
  );
}

/** How long a try takes now, and what may stop it, said beside the Start button. */
function Expectation({ id, initial, r, now }: { id: string; initial: TryInitial; r: Readiness; now: number | null }) {
  const held = r.data?.held;
  return (
    <div id={id} className={styles.expect}>
      {r.pending.length ? (
        <p className={`callout callout-warn ${styles.expectLine}`}>
          <Icon name="clock" size={15} />
          <span>
            <strong>About 1.5 min now.</strong> Attestation on NEAR cannot come before <T1Until r={r} now={now} />, when the last of OutLayer&apos;s new
            worker images counts for {programShort(r)}, so the payout waits for the {r.challenge}&nbsp;s challenge window to end.
          </span>
        </p>
      ) : (
        <p className={styles.expectLine}>
          <Icon name="clock" size={15} />
          {initial.status.ok ? (
            <span>
              <strong>About 45 s,</strong> with the run attested on NEAR: the recorded tries took 43.7 s and 39.9 s from vend to payout.
            </span>
          ) : (
            <span>
              <strong>About 45 s</strong> when the run is attested on NEAR within seconds, about 1.5 min when it is not (paid when the challenge window ends): our
              status could not be read, so which is unknown.
            </span>
          )}
        </p>
      )}
      {r.closed ? (
        <p className={`callout callout-warn ${styles.expectLine}`}>
          <Icon name="alert" size={15} />
          <span>
            <strong>
              {programShort(r).charAt(0).toUpperCase() + programShort(r).slice(1)} is {r.data?.phase}:
            </strong>{" "}
            a try cannot commit to it now.
          </span>
        </p>
      ) : null}
      {r.outOfCredit ? (
        <p className={`callout callout-warn ${styles.expectLine}`}>
          <Icon name="alert" size={15} />
          <span>
            <strong>The practice faucet cannot pay for a try now:</strong> the vending contract&apos;s credit at near-antibody is below one try&apos;s
            run and storage.
          </span>
        </p>
      ) : r.lowBalance || r.keyGone || r.lowAllowance || r.lowCredit ? (
        <p className={`callout callout-warn ${styles.expectLine}`}>
          <Icon name="alert" size={15} />
          <span>
            <strong>The practice faucet is low,</strong> so a try may be refused now.
          </span>
        </p>
      ) : null}
      {held ? (
        <p className={`callout callout-warn ${styles.expectLine}`}>
          <Icon name="clock" size={15} />
          <span>
            <strong>Your payout may wait until {utc(held.expiresAt)}:</strong> commit {held.commitId}, not yet revealed, holds the program&apos;s queue until then.
          </span>
        </p>
      ) : null}
    </div>
  );
}

export function TryApp({ initial }: { initial: TryInitial }) {
  const { snapshot: s, hosts, streamEvents } = useSyncExternalStore(tryStore.subscribe, tryStore.get, tryStore.server);
  const now = useNow(1000);
  const [name, setName] = useState("");
  const [touched, setTouched] = useState(false);
  const [saved, setSaved] = useState<{ record: SavedTry; stale: boolean } | null>(null);
  const [pre, setPre] = useState<PreState>({ status: "loading" });
  const inputId = useId();
  const hintId = useId();
  const errorId = useId();
  const expectId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const runHeading = useRef<HTMLHeadingElement>(null);
  const runShown = useRef(false);

  // In the browser only: a suggested name and the try this tab kept (a reload).
  useEffect(() => {
    const t = setTimeout(() => {
      setName((n) => n || suggestName());
      setSaved(loadSaved());
    }, 0);
    return () => clearTimeout(t);
  }, []);

  // The program, as the try will read it: from the vending contract, never a constant.
  useEffect(() => {
    let alive = true;
    tryStore.preflight(Math.floor(Date.now() / 1000)).then(
      (data) => {
        if (alive) setPre({ status: "ready", data });
      },
      (e: unknown) => {
        if (alive) setPre({ status: "error", error: e instanceof Error ? e.message : String(e) });
      },
    );
    return () => {
      alive = false;
    };
  }, []);

  const nameFailed = s?.phase === "failed" && s.error?.kind === "name" && s.signed === 0;
  const showForm = !s || s.phase === "idle" || nameFailed;
  const showRun = !showForm && s !== null;
  const r = readiness(initial, pre, now);

  // Move focus to the run as it starts, so a screen reader follows it.
  useEffect(() => {
    if (showRun && !runShown.current) runHeading.current?.focus();
    runShown.current = showRun;
  }, [showRun]);

  const localProblem = nameProblem(name);
  const fieldError = nameFailed ? (s?.error?.message ?? null) : touched ? localProblem : null;

  const onStart = (e: FormEvent) => {
    e.preventDefault();
    setTouched(true);
    if (localProblem) {
      inputRef.current?.focus();
      return;
    }
    tryStore.start(name, initial.streamAfter);
  };

  const again = () => {
    tryStore.reset();
    setTouched(false);
    setName(suggestName());
    setSaved(loadSaved());
  };

  const open = saved && !saved.record.closed && !s ? saved : null;
  const last = saved && saved.record.closed && !s ? saved.record : null;
  const waiting =
    s?.phase === "running" &&
    !s.stopped &&
    (s.steps.verdict.status === "running" || s.steps.t1.status === "running" || s.t0Note !== undefined || s.held !== undefined);

  return (
    <div className={`page ${styles.wrap}`}>
      {showForm ? (
        <div className={styles.top}>
          <section className={`card stack ${styles.start}`} aria-labelledby="start-title" style={{ ["--gap" as string]: "16px" }}>
            <div className="stack" style={{ ["--gap" as string]: "6px" }}>
              <h2 id="start-title">Start a try</h2>
              <p className="small muted">No wallet and no account: this tab does everything, with real testnet transactions.</p>
            </div>
            {open ? (
              <div className={`callout callout-warn ${styles.saved}`} role="status">
                <div className="stack" style={{ ["--gap" as string]: "8px" }}>
                  <p>
                    <strong>A try from this tab is still open:</strong> <span className={`mono ${styles.acct}`}>{open.record.account}</span>
                    {open.record.commitId !== undefined ? `, commit ${open.record.commitId}` : ", not yet committed"}. This tab kept its key, so it can{" "}
                    {open.stale ? "close the account" : "resume the try or close the account"}.
                  </p>
                  <div className={styles.actions}>
                    {!open.stale ? (
                      <button type="button" className="btn btn-sm btn-primary" onClick={() => tryStore.resume(open.record, initial.streamAfter)}>
                        <Icon name="play" size={13} /> Resume the try
                      </button>
                    ) : null}
                    <button type="button" className="btn btn-sm" onClick={() => tryStore.closeSaved(open.record)}>
                      Close the account
                    </button>
                  </div>
                </div>
              </div>
            ) : null}
            {last && last.commitId !== undefined ? (
              <p className="small muted">
                Your last try in this tab: <span className={`mono ${styles.acct}`}>{last.account}</span>, <Link href={`/commits/${last.commitId}`}>commit {last.commitId}</Link>
                {last.finalState ? `, ${last.finalState}` : ""}; its account is closed.
              </p>
            ) : null}
            <form className={styles.form} onSubmit={onStart} noValidate>
              <div>
                <label className="label" htmlFor={inputId}>
                  Practice account name
                </label>
                <div className={styles.nameRow}>
                  <input
                    ref={inputRef}
                    id={inputId}
                    className={styles.nameInput}
                    value={name}
                    onChange={(e) => {
                      setName(e.target.value.trim().toLowerCase());
                      setTouched(true);
                      // A new name: the "taken" answer about the last one no longer applies.
                      if (nameFailed) tryStore.reset();
                    }}
                    autoComplete="off"
                    autoCapitalize="none"
                    spellCheck={false}
                    maxLength={32}
                    inputMode="text"
                    aria-invalid={fieldError ? true : undefined}
                    aria-describedby={fieldError ? `${hintId} ${errorId}` : hintId}
                  />
                  <span className={styles.nameSuffix} aria-hidden>
                    {SUFFIX}
                  </span>
                </div>
                <p id={hintId} className="xsmall muted" style={{ marginTop: 6 }}>
                  Your account will be{" "}
                  <span className={`mono ${styles.acct}`}>
                    {name || "<name>"}
                    {SUFFIX}
                  </span>
                  : 1 to 32 of a–z, 0–9, - and _, used once (a closed account keeps its name).
                </p>
                {fieldError ? (
                  <p id={errorId} className={styles.fieldError} role="alert">
                    {fieldError}
                  </p>
                ) : null}
              </div>
              <div className={styles.actions}>
                {r.left === 0 ? (
                  <>
                    <a href="#recorded" className="btn btn-primary">
                      <Icon name="play" size={14} /> Watch a recorded try
                    </a>
                    <button type="submit" className="btn" disabled aria-describedby={expectId}>
                      Next accounts at {resetAt(now ?? initial.readAt)}
                    </button>
                  </>
                ) : (
                  <button type="submit" className="btn btn-primary" aria-describedby={expectId}>
                    <Icon name="play" size={14} /> Start the try
                  </button>
                )}
                <button
                  type="button"
                  className="btn btn-ghost btn-sm"
                  onClick={() => {
                    setName(suggestName());
                    setTouched(false);
                    if (nameFailed) tryStore.reset();
                  }}
                >
                  <Icon name="refresh" size={13} /> Suggest another name
                </button>
              </div>
              <Expectation id={expectId} initial={initial} r={r} now={now} />
            </form>
          </section>

          <section className={`stack ${styles.whatBlock}`} aria-labelledby="what-title">
            <h2 id="what-title">What will happen</h2>
            <ol className={styles.what}>
              <li>
                <span>
                  Your browser makes a throwaway key. The public vend key, deliberately public and allowed only to vend and reclaim, creates{" "}
                  <code className={styles.acct}>&lt;name&gt;{SUFFIX}</code> for it; the vending contract pays.
                </span>
              </li>
              <li>
                <span>It reads the practice program from the vending contract, builds the published tutorial proof against your own vault, seals it, then commits and reveals it.</span>
              </li>
              <li>
                <span>
                  OutLayer&apos;s enclave re-runs the proof on the program&apos;s pinned base and answers on NEAR. The answer is attested once the run&apos;s
                  TDX quote is verified on NEAR; if that has not happened when the challenge window ends, the try is paid then, without it.
                </span>
              </li>
              <li>
                <span>Your browser settles the claim (allocate and pay, both permissionless) and closes the account: its NEAR goes back to the vending contract.</span>
              </li>
            </ol>
            <p className={`small muted ${styles.whatFoot}`}>
              The practice program is a tutorial: its bug and its proof are public, and nothing here is a real vulnerability. NEAR testnet only: wNEAR has no value.{" "}
              <a href={sourceUrl("docs/practice.md")} target="_blank" rel="noreferrer noopener">
                How the practice program works <Icon name="external" size={11} style={{ display: "inline" }} />
              </a>
            </p>
          </section>

          <BeforeStart initial={initial} pre={pre} r={r} now={now} className={styles.before} />
        </div>
      ) : null}

      {showRun && s ? (
        <section className={styles.run} aria-labelledby="run-title">
          <div className={styles.runMain}>
            <div className={styles.runHead}>
              <h2 id="run-title" ref={runHeading} tabIndex={-1} className={styles.wrapAny}>
                Your try: <span className="mono">{s.account}</span>
              </h2>
              <StreamPill s={s} />
            </div>
            <div className={styles.announce} aria-live="polite" aria-atomic="true">
              <Icon name={s.phase === "running" ? "clock" : s.phase === "done" ? "check" : "alert"} size={15} className={s.phase === "running" ? styles.spin : undefined} />
              <span>{s.announce || "Starting the try"}</span>
            </div>
            {s.commitId !== undefined ? (
              <div className={styles.commitLink}>
                <span>
                  Commit <strong className="num">{s.commitId}</strong> is on chain.
                </span>
                <Link href={`/commits/${s.commitId}`}>Follow commit {s.commitId} on its page</Link>
                <span className="muted">You can leave and come back: this tab keeps the try going while it is open.</span>
              </div>
            ) : null}
            {s.phase === "failed" && s.error ? (
              <div className="callout callout-warn" role="alert">
                <Icon name="alert" size={16} />
                <div className="stack" style={{ ["--gap" as string]: "6px" }}>
                  <strong className={styles.wrapAny}>The try stopped: {s.error.message}</strong>
                  {s.error.closing ? <span className={`small ${styles.wrapAny}`}>{s.error.closing.charAt(0).toUpperCase() + s.error.closing.slice(1)}.</span> : null}
                  {s.error.kind === "capacity" && now !== null ? (
                    <span className="small">The next hour starts at {utc(nextHourMs(now), "ms").slice(11, 16)} UTC.</span>
                  ) : null}
                  <FailureHelp s={s} />
                </div>
              </div>
            ) : null}
            {s.phase === "done" ? <TryResult s={s} /> : null}
            {s.held && s.phase === "running" ? (
              <div className="callout callout-warn">
                <Icon name="clock" size={16} />
                <span className="small">
                  {s.held.message}
                  {s.held.expiresAt !== undefined ? ` Its reveal window ends at ${utc(s.held.expiresAt)}.` : ""}
                </span>
              </div>
            ) : null}
            {s.phase === "running" && s.steps.t1.status === "running" && !s.t0Note && r.pending.length ? (
              <div className="callout callout-warn">
                <Icon name="clock" size={16} />
                <span className="small">
                  Attestation on NEAR may not come before <T1Until r={r} now={now} />: OutLayer&apos;s new worker images do not all count for{" "}
                  {programShort(r)} until then. If it has not come when the {r.challenge}&nbsp;s challenge window ends, the try is paid then, without it.
                </span>
              </div>
            ) : null}
            {s.t0Note && s.phase === "running" ? (
              <div className="callout callout-warn">
                <Icon name="clock" size={16} />
                <span className="small">
                  {s.t0Note.charAt(0).toUpperCase() + s.t0Note.slice(1)}
                  {s.t0PayAfter !== undefined ? `; settling at ${utc(s.t0PayAfter, "ms")}.` : "."}
                </span>
              </div>
            ) : null}
            <StepList s={s} />
            <T2Row t2={initial.t2} runId={s.runId} over={s.phase === "done"} />
            {s.log.length ? (
              <div className="stack" style={{ ["--gap" as string]: "8px" }}>
                <h3 className="eyebrow">Notes</h3>
                <ul className={styles.log}>
                  {s.log.map((l, i) => (
                    <li key={`${l.at}-${i}`} className="small">
                      <Pill tone={l.level === "error" ? "bad" : l.level === "warn" ? "warn" : "info"} size="sm">
                        {l.level === "error" ? "error" : l.level === "warn" ? "note" : "info"}
                      </Pill>{" "}
                      <span className={styles.wrapAny}>{l.text}</span>
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
            <div className={styles.actions}>
              {waiting ? (
                <>
                  <button type="button" className="btn btn-sm" onClick={() => tryStore.stop()}>
                    Stop following and close the account
                  </button>
                  <span className="xsmall muted">The claim is still settled: the keeper allocates and pays it once it is final.</span>
                </>
              ) : null}
              {s.phase === "done" || s.phase === "failed" ? (
                <button type="button" className="btn btn-primary btn-sm" onClick={again}>
                  <Icon name="refresh" size={13} /> Start another try
                </button>
              ) : null}
            </div>
          </div>
          <aside className={styles.aside} aria-label="What your browser did">
            <SealedProof key={s.sealed?.sha256 ?? "none"} s={s} />
            <BrowserLedger s={s} hosts={hosts} streamEvents={streamEvents} />
          </aside>
        </section>
      ) : null}

      {showForm ? <BrowserLedger idle s={s} hosts={hosts} streamEvents={streamEvents} /> : null}

      <RecordedTries />
    </div>
  );
}
