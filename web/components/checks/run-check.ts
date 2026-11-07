import type { CheckSpec, Expect } from "@/components/ui/LiveCheck";
import { config } from "@/lib/config";
import { evmRead, nearRpc } from "@/lib/near/client";

/**
 * One check the browser runs, outside a component: the same reads and the same "holds" rule as
 * LiveCheck (components/ui/LiveCheck.tsx), so the judges page can run every claim's check from one
 * button and show each result in two places.
 */

/** What a check gave: the answer and whether it holds, or why it could not run. */
export type CheckOutcome = { status: "done"; answer: unknown; ms: number; holds: boolean | null } | { status: "error"; error: string; ms: number };

/** A dotted path into a JSON answer ("row.state", "chains.0.state"). */
export function at(v: unknown, path?: string): unknown {
  if (!path) return v;
  return path.split(".").reduce<unknown>((cur, k) => {
    if (cur === null || cur === undefined) return undefined;
    if (Array.isArray(cur)) return cur[Number(k)];
    if (typeof cur === "object") return (cur as Record<string, unknown>)[k];
    return undefined;
  }, v);
}

/** Whether the answer is what the claim expects (null when it states no expectation). */
export function holds(answer: unknown, e?: Expect): boolean | null {
  if (!e) return null;
  const v = at(answer, e.path);
  const s = typeof v === "string" ? v : JSON.stringify(v);
  if (e.equals !== undefined) return v === e.equals || s === String(e.equals);
  if (e.contains !== undefined) return (s ?? "").toLowerCase().includes(e.contains.toLowerCase());
  if (e.endsWith !== undefined) return (s ?? "").toLowerCase().replace(/"$/, "").endsWith(e.endsWith.toLowerCase());
  return null;
}

/** What the check read, in one line ("row.state = Done:Paid"). */
export function checkedLine(answer: unknown, e: Expect | undefined, ok: boolean | null): string | null {
  if (!e) return null;
  if (e.path) {
    const v = at(answer, e.path);
    const shown = v === undefined ? "(not in the answer)" : typeof v === "string" ? v : JSON.stringify(v);
    const line = `${e.path} = ${shown}`;
    return line.length > 160 ? `${line.slice(0, 160)}…` : line;
  }
  const needle = e.contains ?? e.endsWith;
  return needle !== undefined ? `the answer ${ok ? "contains" : "does not contain"} "${needle}"` : null;
}

/** The read, as the page prints it under the button. */
export function describe(c: CheckSpec): string {
  if (c.kind === "api") return `GET ${config.apiBase}${c.path}`;
  if (c.kind === "near-view") return `view ${c.contract}.${c.method}(${JSON.stringify(c.args ?? {})})`;
  return `${c.method} on chain ${c.chainId}`;
}

/** Runs one check from the browser: the API, a NEAR view call (with the client's backoff) or an EVM read. */
export async function runCheck(check: CheckSpec, expect?: Expect): Promise<CheckOutcome> {
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
    return { status: "done", answer, ms: performance.now() - t0, holds: holds(answer, expect) };
  } catch (e) {
    return { status: "error", error: e instanceof Error ? e.message : String(e), ms: performance.now() - t0 };
  }
}
