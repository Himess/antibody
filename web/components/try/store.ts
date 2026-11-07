/**
 * The page's try, outside React: one flow per tab, kept alive across in-app navigation (a visitor may
 * open the commit page and come back), with the browser's own dependencies: the public NEAR RPCs (with a
 * counter per host, for "your browser did everything"), the API's event stream, the platform CSPRNG and
 * the tab's sessionStorage. Components read it with `useSyncExternalStore`.
 */
import { config } from "@/lib/config";
import { NearRpc } from "@/lib/near/rpc";
import { type CommitView, commitStateLabel, type OpenStream, type SavedTry, type TryStorage, TryFlow, type TrySnapshot, VENDING } from "@/lib/practice/flow";
import { parseVendingConfigFull } from "@/lib/practice/vending";

export const STORAGE_KEY = "antibody.try.v1";

/** The tab's storage: a reload can resume the try, or at least close its account. Never sent anywhere. */
export const tabStorage: TryStorage = {
  load() {
    try {
      const s = window.sessionStorage.getItem(STORAGE_KEY);
      if (!s) return null;
      const v = JSON.parse(s) as SavedTry;
      return v && v.v === 1 && typeof v.seedHex === "string" && /^[0-9a-f]{64}$/.test(v.seedHex) && typeof v.account === "string" ? v : null;
    } catch {
      return null;
    }
  },
  save(t) {
    try {
      window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify(t));
    } catch {
      // private mode or a full store: the try goes on, only a reload cannot resume it
    }
  },
  clear() {
    try {
      window.sessionStorage.removeItem(STORAGE_KEY);
    } catch {
      // nothing kept
    }
  },
};

export type HostRole = "rpc" | "archival" | "api";

export interface HostUse {
  host: string;
  role: HostRole;
  requests: number;
  /** Event streams opened (the API). */
  streams: number;
}

export interface TryState {
  snapshot: TrySnapshot | null;
  hosts: HostUse[];
  streamEvents: number;
}

/** What the page reads before a try: the program the vending contract vends for, and its queue. */
export interface Preflight {
  programId: number;
  antibody: string;
  perHour?: number;
  vended?: number;
  /** yoctoNEAR the vending contract lends each practice account (`close` returns what is left). */
  gasFundsYocto?: string;
  programKey?: string;
  phase?: string;
  createdAt?: number;
  challengeSecs?: number;
  revealSecs?: number;
  /** An unrevealed commit at the program's cursor, committed a while ago: it holds the queue. */
  held?: { commitId: number; committedAt: number; expiresAt: number };
}

const EMPTY: TryState = { snapshot: null, hosts: [], streamEvents: 0 };

class Store {
  private state: TryState = EMPTY;
  private readonly listeners = new Set<() => void>();
  private flow: TryFlow | null = null;
  private clientsCache: { rpc: NearRpc; archival: NearRpc } | null = null;
  private readonly hosts = new Map<string, HostUse>();
  private events = 0;

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  };

  readonly get = (): TryState => this.state;

  readonly server = (): TryState => EMPTY;

  private notify(): void {
    this.state = { snapshot: this.flow?.snapshot ?? null, hosts: [...this.hosts.values()], streamEvents: this.events };
    for (const fn of this.listeners) fn();
  }

  private count(url: string, role: HostRole, stream = false): void {
    let host: string;
    try {
      // The host only: an RPC URL may carry a key in its path or query.
      host = new URL(url).host;
    } catch {
      return;
    }
    const h = this.hosts.get(host) ?? { host, role, requests: 0, streams: 0 };
    this.hosts.set(host, stream ? { ...h, streams: h.streams + 1 } : { ...h, requests: h.requests + 1 });
    this.notify();
  }

  /** A GET on our API from this tab (counted like the rest); null when it fails. */
  async apiGet<T>(path: string): Promise<T | null> {
    const url = `${config.apiBase}${path}`;
    this.count(url, "api");
    try {
      const res = await fetch(url, { cache: "no-store", headers: { accept: "application/json" }, signal: AbortSignal.timeout(20_000) });
      return res.ok ? ((await res.json()) as T) : null;
    } catch {
      return null;
    }
  }

  /** The NEAR RPC clients, in order of preference, counting each request per host. */
  clients(): { rpc: NearRpc; archival: NearRpc } {
    if (!this.clientsCache) {
      const counted =
        (role: HostRole) =>
        (url: string, init: RequestInit): Promise<Response> => {
          this.count(url, role);
          return fetch(url, init);
        };
      this.clientsCache = {
        // The reveal's answer waits for OutLayer's (about 15 s): a request may take that long.
        rpc: new NearRpc(config.nearRpcs, { timeoutMs: 30_000, attempts: 6, fetch: counted("rpc") }),
        archival: new NearRpc(config.nearArchivalRpcs, { timeoutMs: 30_000, attempts: 4, fetch: counted("archival") }),
      };
    }
    return this.clientsCache;
  }

  private readonly openStream: OpenStream = (url, on) => {
    this.count(url, "api", true);
    const es = new EventSource(url);
    es.onopen = () => on.open();
    es.addEventListener("antibody", (m) => {
      this.events += 1;
      try {
        on.event(JSON.parse((m as MessageEvent<string>).data) as Parameters<typeof on.event>[0]);
      } catch {
        // an event that is not JSON is skipped
      }
    });
    es.onerror = () => on.error(es.readyState === EventSource.CLOSED);
    return () => es.close();
  };

  private newFlow(streamAfter: number | undefined): TryFlow {
    const { rpc, archival } = this.clients();
    const flow = new TryFlow({
      rpc,
      archival,
      apiBase: config.apiBase,
      streamAfter,
      openStream: typeof EventSource === "undefined" ? null : this.openStream,
      now: () => Date.now(),
      sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
      storage: tabStorage,
      fetchUrl: async (url) => {
        this.count(url, "archival");
        const res = await fetch(url, { cache: "no-store" });
        if (!res.ok) throw new Error(`GET ${new URL(url).host}: HTTP ${res.status}`);
        return new Uint8Array(await res.arrayBuffer());
      },
    });
    flow.subscribe(() => this.notify());
    this.flow = flow;
    this.notify();
    return flow;
  }

  running(): boolean {
    return this.flow?.snapshot.phase === "running";
  }

  start(name: string, streamAfter?: number): void {
    if (this.running()) return;
    void this.newFlow(streamAfter).start(name);
  }

  resume(saved: SavedTry, streamAfter?: number): void {
    if (this.running()) return;
    void this.newFlow(streamAfter).resume(saved);
  }

  closeSaved(saved: SavedTry): void {
    if (this.running()) return;
    void this.newFlow(undefined).closeSaved(saved);
  }

  stop(): void {
    this.flow?.stopFollowing();
  }

  /** Back to the form, once a try is over. */
  reset(): void {
    if (this.running()) return;
    this.flow = null;
    this.notify();
  }

  /** practice.rs `target`: the program the vending contract vends for, and whether a stale commit holds its queue. */
  async preflight(nowS: number, vending = VENDING): Promise<Preflight> {
    const { rpc } = this.clients();
    const cfg = parseVendingConfigFull((await rpc.viewFunction(vending, "get_config", new Uint8Array(0))).bytes);
    const p = (await rpc.viewFunction(cfg.antibody, "get_program", { program_id: cfg.programId })).json<{
      key?: string;
      phase?: unknown;
      created_at?: number;
      cursor?: number;
      commits?: number;
      params?: { windows?: { challenge?: number; reveal?: number } };
    } | null>();
    const out: Preflight = { programId: cfg.programId, antibody: cfg.antibody, perHour: cfg.terms?.perHour, vended: cfg.vended, gasFundsYocto: cfg.terms?.gasFunds.toString() };
    if (!p) return out;
    const reveal = p.params?.windows?.reveal;
    Object.assign(out, {
      programKey: p.key,
      phase: typeof p.phase === "string" ? p.phase : Object.keys(p.phase ?? {})[0],
      createdAt: p.created_at,
      challengeSecs: p.params?.windows?.challenge,
      revealSecs: reveal,
    });
    if (typeof p.cursor === "number" && typeof p.commits === "number" && p.cursor < p.commits) {
      const list = (await rpc.viewFunction(cfg.antibody, "list_commits", { program_id: cfg.programId, from: p.cursor, limit: 1 })).json<CommitView[]>();
      const c = Array.isArray(list) ? list[0] : undefined;
      // A try in progress reveals within seconds; one unrevealed for a minute holds the queue.
      if (c && c.id !== undefined && c.committed_at !== undefined && commitStateLabel(c.state) === "Committed" && nowS - c.committed_at > 60) {
        out.held = { commitId: c.id, committedAt: c.committed_at, expiresAt: c.committed_at + (reveal ?? 600) };
      }
    }
    return out;
  }
}

export const tryStore = new Store();
