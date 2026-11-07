"use client";

import { useSyncExternalStore } from "react";
import { isoOf, relative, utc } from "@/lib/format";

/** "Now", to the half minute: a stable snapshot between ticks, and none on the server (no hydration mismatch). */
const TICK = 30_000;
const subscribe = (cb: () => void) => {
  const id = setInterval(cb, TICK);
  return () => clearInterval(id);
};
const getNow = () => Math.floor(Date.now() / TICK) * TICK;
const getServerNow = () => null;

/** A moment in UTC, with its relative time once the browser knows "now". */
export function Time({ t, unit = "s", showRelative = true }: { t: number | null | undefined; unit?: "s" | "ns" | "ms"; showRelative?: boolean }) {
  const now = useSyncExternalStore(subscribe, getNow, getServerNow);
  if (!t) return <span className="muted">—</span>;
  return (
    <time dateTime={isoOf(t, unit)} title={utc(t, unit)} className="num">
      <span>{utc(t, unit)}</span>
      {showRelative && now ? <span className="muted"> · {relative(t, now, unit)}</span> : null}
    </time>
  );
}
