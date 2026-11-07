"use client";

import { useState } from "react";
import { config } from "@/lib/config";
import { evmRead, nearRpc } from "@/lib/near/client";
import { Command } from "./Command";
import { Icon } from "./Icon";
import { Pill } from "./Pill";
import styles from "./LiveCheck.module.css";

/**
 * A check the browser runs itself, showing the raw response: an API record, a NEAR view call or an EVM
 * read. `expect` states what the claim says the answer is; the result says whether it holds.
 */
export type CheckSpec =
  | { kind: "api"; path: string }
  | { kind: "near-view"; contract: string; method: string; args?: Record<string, unknown> }
  | { kind: "evm"; chainId: number; method: string; params: unknown[] };

export type Expect = {
  /** A dotted path into the JSON answer ("frozen.commit_id", "chains.0.state"). */
  path?: string;
  equals?: string | number | boolean | null;
  contains?: string;
  endsWith?: string;
  /** Plain words for what the claim expects. */
  says: string;
};

function at(v: unknown, path?: string): unknown {
  if (!path) return v;
  return path.split(".").reduce<unknown>((cur, k) => {
    if (cur === null || cur === undefined) return undefined;
    if (Array.isArray(cur)) return cur[Number(k)];
    if (typeof cur === "object") return (cur as Record<string, unknown>)[k];
    return undefined;
  }, v);
}

function holds(answer: unknown, e?: Expect): boolean | null {
  if (!e) return null;
  const v = at(answer, e.path);
  const s = typeof v === "string" ? v : JSON.stringify(v);
  if (e.equals !== undefined) return v === e.equals || s === String(e.equals);
  if (e.contains !== undefined) return (s ?? "").toLowerCase().includes(e.contains.toLowerCase());
  if (e.endsWith !== undefined) return (s ?? "").toLowerCase().replace(/"$/, "").endsWith(e.endsWith.toLowerCase());
  return null;
}

function describe(c: CheckSpec): string {
  if (c.kind === "api") return `GET ${config.apiBase}${c.path}`;
  if (c.kind === "near-view") return `view ${c.contract}.${c.method}(${JSON.stringify(c.args ?? {})})`;
  return `${c.method} on chain ${c.chainId}`;
}

export function LiveCheck({ check, expect, cli, label = "Run this check" }: { check: CheckSpec; expect?: Expect; cli?: string; label?: string }) {
  const [state, setState] = useState<{ status: "idle" | "running" | "done" | "error"; answer?: unknown; error?: string; ms?: number }>({ status: "idle" });
  const run = async () => {
    setState({ status: "running" });
    const t0 = performance.now();
    try {
      let answer: unknown;
      if (check.kind === "api") {
        const res = await fetch(`${config.apiBase}${check.path}`, { cache: "no-store", signal: AbortSignal.timeout(30_000) });
        answer = await res.json();
        if (!res.ok) throw new Error((answer as { error?: string })?.error ?? `HTTP ${res.status}`);
      } else if (check.kind === "near-view") {
        answer = (await nearRpc().viewFunction(check.contract, check.method, check.args ?? {}, "final")).json();
      } else {
        answer = await evmRead(check.chainId, check.method, check.params);
      }
      setState({ status: "done", answer, ms: performance.now() - t0 });
    } catch (e) {
      setState({ status: "error", error: e instanceof Error ? e.message : String(e), ms: performance.now() - t0 });
    }
  };
  const verdict = state.status === "done" ? holds(state.answer, expect) : null;
  const shown = state.status === "done" ? JSON.stringify(state.answer, null, 2) : "";
  return (
    <div className={styles.check}>
      <div className={styles.head}>
        <button type="button" className="btn btn-sm" onClick={run} disabled={state.status === "running"} aria-busy={state.status === "running"}>
          <Icon name={state.status === "running" ? "refresh" : "play"} size={13} />
          {state.status === "running" ? "Checking…" : state.status === "idle" ? label : "Run again"}
        </button>
        <span className={`${styles.what} mono`}>{describe(check)}</span>
      </div>
      {expect ? <p className="xsmall muted">Expected: {expect.says}</p> : null}
      <div aria-live="polite">
        {state.status === "done" ? (
          <div className={styles.result}>
            <div className="row" style={{ ["--gap" as string]: "8px" }}>
              {verdict === true ? <Pill tone="ok" size="sm">holds</Pill> : verdict === false ? <Pill tone="bad" size="sm">differs</Pill> : <Pill size="sm">answered</Pill>}
              <span className="xsmall muted num">{Math.round(state.ms ?? 0)} ms · read by your browser</span>
            </div>
            <pre className={styles.raw} tabIndex={0} aria-label="Raw response">
              {shown.length > 6000 ? shown.slice(0, 6000) + "\n…" : shown}
            </pre>
          </div>
        ) : state.status === "error" ? (
          <div className={styles.result}>
            <Pill tone="warn" size="sm">could not check</Pill>
            <p className="xsmall mono">{state.error}</p>
            <p className="xsmall muted">Public RPCs rate-limit; try again in a few seconds, or run the command below.</p>
          </div>
        ) : null}
      </div>
      {cli ? <Command label="CLI command">{cli}</Command> : null}
    </div>
  );
}
