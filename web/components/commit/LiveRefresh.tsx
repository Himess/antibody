"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { Pill } from "@/components/ui/Pill";
import { streamUrl } from "@/lib/api";

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const num = (v: unknown) => (typeof v === "number" ? v : typeof v === "string" && /^\d+$/.test(v) ? Number(v) : null);

/**
 * Keeps an unsettled commit's page current. It follows the API's live stream (server-sent events) and,
 * when an event names this commit, its pause ticket or one of its runs, renders the page again from the
 * server. The server keeps its reads for about ten seconds and serves a stale one while it refreshes it,
 * so each such event brings refreshes until that cache has turned over twice. Without a stream, it reads
 * the page again every 20 seconds while the tab is visible.
 */
export function LiveRefresh({ commitId, runs, after }: { commitId: number; runs: string; after: number }) {
  const router = useRouter();
  useEffect(() => {
    const runIds = new Set(runs.split(",").filter(Boolean).map(Number));
    const timers = new Set<ReturnType<typeof setTimeout>>();
    const refreshSoon = () => {
      for (const ms of [1_500, 12_000, 24_000]) {
        const t = setTimeout(() => {
          timers.delete(t);
          router.refresh();
        }, ms);
        timers.add(t);
      }
    };
    const names = (data: unknown) => {
      if (!isObj(data)) return false;
      if (num(data.commit_id) === commitId || num(data.ticket) === commitId) return true;
      const run = num(data.run_id);
      return run !== null && runIds.has(run);
    };
    let poll: ReturnType<typeof setInterval> | null = null;
    const startPolling = () => {
      if (poll) return;
      poll = setInterval(() => {
        if (document.visibilityState === "visible") router.refresh();
      }, 20_000);
    };
    let es: EventSource | null = null;
    if (typeof EventSource === "undefined") {
      startPolling();
    } else {
      es = new EventSource(streamUrl(after));
      es.addEventListener("antibody", (m) => {
        try {
          const ev = JSON.parse((m as MessageEvent<string>).data) as unknown;
          if (isObj(ev) && names(ev.data)) refreshSoon();
        } catch {
          // an event that is not JSON is skipped
        }
      });
      // A dropped connection reconnects by itself; a refused one (CLOSED) falls back to reading the page.
      es.onerror = () => {
        if (es?.readyState === EventSource.CLOSED) startPolling();
      };
    }
    return () => {
      es?.close();
      if (poll) clearInterval(poll);
      timers.forEach(clearTimeout);
    };
  }, [router, commitId, runs, after]);
  return (
    <Pill tone="info" icon="refresh" title="This page follows the live event stream and updates as the commit settles">
      live · updates as it settles
    </Pill>
  );
}
