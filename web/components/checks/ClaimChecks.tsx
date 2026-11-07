"use client";

import { createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode } from "react";
import { StateBadge, TierBadge } from "@/components/ui/Badges";
import { Command } from "@/components/ui/Command";
import { Icon } from "@/components/ui/Icon";
import lc from "@/components/ui/LiveCheck.module.css";
import { Pill } from "@/components/ui/Pill";
import { Md } from "./Md";
import type { ClaimCheck } from "./judges";
import { checkedLine, describe, runCheck, type CheckOutcome } from "./run-check";
import styles from "./checks.module.css";

type CheckState = { status: "idle" } | { status: "running" } | CheckOutcome;
const IDLE: CheckState = { status: "idle" };
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

interface Checks {
  state: (id: string) => CheckState;
  run: (c: ClaimCheck) => Promise<CheckState>;
}

const ChecksContext = createContext<Checks | null>(null);

/**
 * The judges page's checks, by id: a claim's check run from the index shows its answer in the claim's
 * card too, and the other way round.
 */
export function ClaimChecksProvider({ children }: { children: ReactNode }) {
  const [states, setStates] = useState<Record<string, CheckState>>({});
  const running = useRef(new Set<string>());
  const run = useCallback(async (c: ClaimCheck): Promise<CheckState> => {
    if (running.current.has(c.id)) return { status: "running" };
    running.current.add(c.id);
    setStates((s) => ({ ...s, [c.id]: { status: "running" } }));
    const out = await runCheck(c.check, c.expect);
    running.current.delete(c.id);
    setStates((s) => ({ ...s, [c.id]: out }));
    return out;
  }, []);
  const value = useMemo<Checks>(() => ({ state: (id) => states[id] ?? IDLE, run }), [states, run]);
  return <ChecksContext.Provider value={value}>{children}</ChecksContext.Provider>;
}

/** A check's state from the page's shared store, or its own when it stands alone. */
function useCheck(c: ClaimCheck): [CheckState, () => void] {
  const shared = useContext(ChecksContext);
  const [own, setOwn] = useState<CheckState>(IDLE);
  if (shared) return [shared.state(c.id), () => void shared.run(c)];
  return [
    own,
    () => {
      setOwn({ status: "running" });
      void runCheck(c.check, c.expect).then(setOwn);
    },
  ];
}

/** LiveCheck, with its answer kept in the page's store: the same button, raw answer and CLI command. */
export function SharedCheck({ c }: { c: ClaimCheck }) {
  const [state, run] = useCheck(c);
  const verdict = state.status === "done" ? state.holds : null;
  const checked = state.status === "done" ? checkedLine(state.answer, c.expect, verdict) : null;
  const shown = state.status === "done" ? (JSON.stringify(state.answer, null, 2) ?? String(state.answer)) : "";
  return (
    <div className={lc.check}>
      <div className={lc.head}>
        <button type="button" className="btn btn-sm" onClick={run} disabled={state.status === "running"} aria-busy={state.status === "running"}>
          <Icon name={state.status === "running" ? "refresh" : "play"} size={13} />
          {state.status === "running" ? "Checking…" : state.status === "idle" ? (c.label ?? "Run this check") : "Run again"}
        </button>
        <span className={`${lc.what} mono`}>{describe(c.check)}</span>
      </div>
      {c.expect ? <p className="xsmall muted">Holds if: {c.expect.says}</p> : null}
      <div aria-live="polite">
        {state.status === "done" ? (
          <div className={lc.result}>
            <div className="row" style={{ ["--gap" as string]: "8px" }}>
              {verdict === true ? (
                <Pill tone="ok" size="sm">
                  holds
                </Pill>
              ) : verdict === false ? (
                <Pill tone="bad" size="sm">
                  differs
                </Pill>
              ) : (
                <Pill size="sm">answered</Pill>
              )}
              <span className="xsmall muted num">{Math.round(state.ms)} ms · read by your browser</span>
            </div>
            {checked ? (
              <p className={`xsmall ${lc.checked}`}>
                <span className="muted">Checked: </span>
                <code>{checked}</code>
              </p>
            ) : null}
            <pre className={lc.raw} tabIndex={0} aria-label="Raw response">
              {shown.length > 6000 ? shown.slice(0, 6000) + "\n…" : shown}
            </pre>
          </div>
        ) : state.status === "error" ? (
          <div className={lc.result}>
            <Pill tone="warn" size="sm">
              could not check
            </Pill>
            <p className="xsmall mono">{state.error}</p>
            <p className="xsmall muted">Public RPCs rate-limit; try again in a few seconds, or run the command below.</p>
          </div>
        ) : null}
      </div>
      {c.cli ? <Command label="CLI command">{c.cli}</Command> : null}
    </div>
  );
}

type Outcome = "idle" | "running" | "holds" | "differs" | "error" | "answered" | "partial";

/** A claim's result from its checks: it holds only when every one of them holds. */
function outcomeOf(states: CheckState[]): { kind: Outcome; done: number; total: number } {
  const total = states.length;
  const done = states.filter((s) => s.status === "done").length;
  let kind: Outcome = "idle";
  if (states.some((s) => s.status === "running")) kind = "running";
  else if (states.some((s) => s.status === "done" && s.holds === false)) kind = "differs";
  else if (states.some((s) => s.status === "error")) kind = "error";
  else if (done === 0) kind = "idle";
  else if (done < total) kind = "partial";
  else if (states.every((s) => s.status === "done" && s.holds === true)) kind = "holds";
  else kind = "answered";
  return { kind, done, total };
}

function OutcomePill({ o }: { o: { kind: Outcome; done: number; total: number } }) {
  if (o.kind === "running")
    return (
      <Pill tone="pending" size="sm">
        checking…
      </Pill>
    );
  if (o.kind === "holds")
    return (
      <Pill tone="ok" size="sm">
        {o.total > 1 ? `${o.total} of ${o.total} hold` : "holds"}
      </Pill>
    );
  if (o.kind === "differs")
    return (
      <Pill tone="bad" size="sm">
        differs
      </Pill>
    );
  if (o.kind === "error")
    return (
      <Pill tone="warn" size="sm">
        could not check
      </Pill>
    );
  if (o.kind === "answered") return <Pill size="sm">answered</Pill>;
  if (o.kind === "partial") return <Pill size="sm">{`${o.done} of ${o.total} read`}</Pill>;
  return null;
}

/** The claim's result in its card's header, once one of its checks has run (here or from the index). */
export function ClaimResult({ checks }: { checks: ClaimCheck[] }) {
  const shared = useContext(ChecksContext);
  if (!shared || !checks.length) return null;
  const o = outcomeOf(checks.map((c) => shared.state(c.id)));
  return o.kind === "idle" ? null : (
    <span className={styles.claimResult}>
      <OutcomePill o={o} />
    </span>
  );
}

export interface IndexRow {
  n: number;
  claim: string;
  tier?: string | null;
  state?: string | null;
  checks: ClaimCheck[];
}

/** Markdown links out of a title that sits inside a link. */
const unlink = (s: string) => s.replace(/\[([^\]]+)\]\([^)\s]+\)/g, "$1");

/**
 * Every claim on one screen: its recorded outcome, a check button and the live result, and one button
 * that runs every claim's check in turn (a failed read is tried again after a pause), with the tally.
 */
export function ClaimIndex({ rows }: { rows: IndexRow[] }) {
  const shared = useContext(ChecksContext);
  const [progress, setProgress] = useState<{ at: number; total: number } | null>(null);
  const checkable = rows.filter((r) => r.checks.length);
  const terminal = rows.length - checkable.length;
  const outcomes = rows.map((r) => outcomeOf(r.checks.map((c) => shared?.state(c.id) ?? IDLE)));
  const count = (k: Outcome) => outcomes.filter((o, i) => rows[i].checks.length && o.kind === k).length;
  const ran = checkable.length - count("idle");

  const runAll = async () => {
    if (!shared || progress) return;
    for (let i = 0; i < checkable.length; i++) {
      setProgress({ at: i + 1, total: checkable.length });
      for (const c of checkable[i].checks) {
        let out = await shared.run(c);
        for (let attempt = 1; out.status === "error" && attempt <= 2; attempt++) {
          await sleep(1500 * attempt);
          out = await shared.run(c);
        }
        await sleep(200);
      }
    }
    setProgress(null);
  };

  const tally = progress
    ? `Checking claim ${progress.at} of ${progress.total}…`
    : ran === 0
      ? `${checkable.length} claims, each with a check your browser runs${terminal ? `; ${terminal} only in a terminal` : ""}.`
      : [
          `${count("holds")} of ${checkable.length} hold`,
          count("differs") ? `${count("differs")} differ${count("differs") === 1 ? "s" : ""}` : "",
          count("error") ? `${count("error")} could not be checked` : "",
          count("answered") ? `${count("answered")} answered` : "",
          count("running") ? `${count("running")} checking` : "",
          count("partial") + count("idle") ? `${count("partial") + count("idle")} not run yet` : "",
          terminal ? `${terminal} terminal-only` : "",
        ]
          .filter(Boolean)
          .join(" · ");

  return (
    <div className={styles.index}>
      <div className={styles.indexHead}>
        <button type="button" className="btn btn-primary btn-sm" onClick={() => void runAll()} disabled={!shared || progress !== null} aria-busy={progress !== null}>
          <Icon name={progress ? "refresh" : "play"} size={13} />
          {progress ? "Checking…" : ran ? "Run every check again" : "Run every check"}
        </button>
        <span className={`small ${styles.tally}`} role="status" aria-live="polite">
          {tally}
        </span>
      </div>
      <p className="xsmall muted">
        One after another, your browser reads each claim&apos;s check now: the API record, a NEAR view call or an EVM read. A read that fails is
        tried again after a pause.
      </p>
      <ol className={styles.indexList} aria-label="The claims and their checks">
        {rows.map((r, i) => (
          <li key={r.n} className={styles.indexRow}>
            <span className={styles.indexNo}>{r.n}</span>
            <div className={styles.indexMain}>
              <a href={`#claim-${r.n}`} className={styles.indexTitle}>
                <Md text={unlink(r.claim)} />
              </a>
              {r.tier || r.state ? (
                <span className={styles.indexMeta}>
                  {r.tier ? <TierBadge tier={r.tier} size="sm" /> : null}
                  {r.state ? <StateBadge state={r.state} size="sm" /> : null}
                </span>
              ) : null}
            </div>
            <div className={styles.indexAct}>
              {r.checks.length ? (
                <>
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() => r.checks.forEach((c) => void shared?.run(c))}
                    disabled={!shared || progress !== null || outcomes[i].kind === "running"}
                    aria-label={`Run the check of claim ${r.n}`}
                  >
                    <Icon name="play" size={12} />
                    Check
                  </button>
                  {outcomes[i].kind === "idle" ? <span className="xsmall muted">not run</span> : <OutcomePill o={outcomes[i]} />}
                </>
              ) : (
                <span className="xsmall muted">in a terminal</span>
              )}
            </div>
          </li>
        ))}
      </ol>
    </div>
  );
}
