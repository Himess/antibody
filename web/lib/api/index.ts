import { config } from "@/lib/config";
import type {
  Account,
  CommitDetail,
  FeedRow,
  Health,
  IndexedEvent,
  LeaderRow,
  Mainnet,
  Moments,
  Practice,
  ProgramDetail,
  ProgramSummary,
  Reflex,
  RunDetail,
  Stats,
  Status,
  T2Panel,
  TrustActor,
} from "./types";

export * from "./types";

/** A read that may fail: pages render an error state rather than throw. */
export type Result<T> = { ok: true; data: T; at: number } | { ok: false; error: string; status?: number; at: number };

/**
 * How long one read may take in all, retry included. A run or commit the API has not read for a while can
 * take over 12 s to answer the first time; a page shows a read as not loaded only after this.
 */
const BUDGET_MS = { server: 25_000, browser: 20_000 };

/** A failure that is not an answer (no connection, a 5xx) is worth one more try; a 4xx is the answer. */
const retryable = (r: Result<unknown>) => !r.ok && (r.status === undefined || r.status >= 500);

/**
 * GET an API path. On the server, answers are kept in the Data Cache for `revalidate` seconds (pages read
 * the API with short revalidation); in the browser, they are always fresh. A failed read is tried once
 * more while its time budget lasts.
 */
export async function get<T>(path: string, revalidate = 15): Promise<Result<T>> {
  const server = typeof window === "undefined";
  if (typeof window === "undefined") await atRequestTime();
  const at = Date.now();
  const budget = server ? BUDGET_MS.server : BUDGET_MS.browser;
  const first = await getOnce<T>(path, revalidate, server, budget, at);
  const left = budget - (Date.now() - at) - 400;
  if (!retryable(first) || left < 3_000) return first;
  await new Promise((r) => setTimeout(r, 400));
  return getOnce<T>(path, revalidate, server, left, at);
}

/**
 * On the server, a read waits for a request. During a build's prerender, `connection()` stops the render
 * here: every page that reads the API renders at request time anyway (the nonce CSP needs that), and the
 * build never waits on the API. A cold API once made a build's prerender of /programs pass 60 s. The Data
 * Cache still keeps each answer for `revalidate` seconds.
 */
async function atRequestTime() {
  const { connection } = await import("next/server");
  await connection();
}

async function getOnce<T>(path: string, revalidate: number, server: boolean, timeoutMs: number, at: number): Promise<Result<T>> {
  const url = `${config.apiBase}${path}`;
  try {
    // Our own signal also opts the request out of fetch deduplication, so a retry is a fresh request.
    const res = await fetch(url, {
      headers: { accept: "application/json" },
      ...(server ? { next: { revalidate } } : { cache: "no-store" as const }),
      signal: AbortSignal.timeout(timeoutMs),
    });
    const text = await res.text();
    let body: unknown = null;
    try {
      body = text ? JSON.parse(text) : null;
    } catch {
      body = null;
    }
    if (!res.ok) {
      const msg = (body as { error?: string } | null)?.error ?? `HTTP ${res.status}`;
      return { ok: false, error: msg, status: res.status, at };
    }
    return { ok: true, data: body as T, at };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e), at };
  }
}

export const api = {
  health: () => get<Health>("/health", 300),
  status: () => get<Status>("/v1/status", 15),
  stats: () => get<Stats>("/v1/stats", 30),
  programs: () => get<{ programs: ProgramSummary[] }>("/v1/programs", 30),
  program: (id: number) => get<ProgramDetail>(`/v1/programs/${id}`, 15),
  feed: (id: number) => get<{ program_id: number; submissions: FeedRow[] }>(`/v1/programs/${id}/feed`, 10),
  reflex: (id: number) => get<Reflex>(`/v1/programs/${id}/reflex`, 30),
  commit: (id: number) => get<CommitDetail>(`/v1/commits/${id}`, 10),
  run: (id: number) => get<RunDetail>(`/v1/runs/${id}`, 10),
  account: (id: string) => get<Account>(`/v1/accounts/${encodeURIComponent(id)}`, 10),
  leaderboard: () => get<{ leaderboard: LeaderRow[] }>("/v1/leaderboard", 30),
  events: (after = 0, limit = 200) => get<{ events: IndexedEvent[] }>(`/v1/events?scope=antibody&after=${after}&limit=${limit}`, 10),
  guardian: () => get<Record<string, unknown>>("/v1/guardian", 30),
  judge: () => get<Record<string, unknown>>("/v1/judge", 300),
  trust: () => get<{ actors: TrustActor[] }>("/v1/trust", 3600),
  practice: () => get<Practice>("/v1/practice", 15),
  mainnet: () => get<Mainnet>("/v1/mainnet", 600),
  t2: () => get<T2Panel>("/v1/t2", 30),
  moments: () => get<Moments>("/v1/moments", 300),
};

/** near-t2 is not on testnet until the API names it: `/v1/t2` answers this error until then. */
export const t2NotYet = (r: Result<unknown>) => !r.ok && /no near-t2/i.test(r.error);

/** The live event stream (server-sent events) of Antibody's own events. */
export const streamUrl = (after?: number) => `${config.apiBase}/v1/stream?scope=antibody${after ? `&after=${after}` : ""}`;
