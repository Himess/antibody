/**
 * What the try page reads on the server (the API's /v1/status, /v1/practice, /v1/stats and /v1/t2) and
 * hands to the browser: only the plain values the page shows. Every field may be missing: the API may
 * be down or omit one, and the page then says so instead of guessing.
 */
import type { Alert, Practice, Result, Stats, Status, T2Panel } from "@/lib/api";
import { t2NotYet } from "@/lib/api";

export interface PendingImage {
  /** Unix seconds when the image counts for T1 on programs created before it was listed. */
  countsFrom: number;
  countsFromUtc: string;
  /** Unix seconds when near-antibody listed it. */
  listedAt: number;
}

export interface TryInitial {
  status: {
    ok: boolean;
    error?: string;
    /** Worker images still inside the image delay. */
    pendingImages: PendingImage[];
    delaySecs?: number;
    /** The practice faucet (`components.practice`). */
    faucet?: { balanceNear?: number; allowanceNear?: number | null; creditNear?: number; program?: number; vending?: string };
    alerts: Alert[];
  };
  practice: { ok: boolean; error?: string; vending?: string; programId?: number; publicVendKey?: string; balanceYocto?: string | null };
  /** The event stream's replay start (inclusive), a little before the latest event the API counted. */
  streamAfter?: number;
  t2: "live" | "not-yet" | "unknown";
  /** When the server read these (ms). */
  readAt: number;
}

const rec = (v: unknown): Record<string, unknown> | undefined =>
  typeof v === "object" && v !== null && !Array.isArray(v) ? (v as Record<string, unknown>) : undefined;
const num = (v: unknown): number | undefined => (typeof v === "number" && Number.isFinite(v) ? v : undefined);
const str = (v: unknown): string | undefined => (typeof v === "string" ? v : undefined);

export function buildInitial(status: Result<Status>, practice: Result<Practice>, stats: Result<Stats>, t2: Result<T2Panel>): TryInitial {
  const components = status.ok ? rec(status.data.components) : undefined;
  const images = rec(components?.images);
  const pendingImages: PendingImage[] = (Array.isArray(images?.pending) ? images.pending : []).flatMap((i) => {
    const o = rec(i);
    const countsFrom = num(o?.counts_from);
    if (countsFrom === undefined) return [];
    return [{ countsFrom, countsFromUtc: str(o?.counts_from_utc) ?? "", listedAt: num(o?.listed_at) ?? 0 }];
  });
  const f = rec(components?.practice);
  const allowance = f?.public_vend_key_allowance_near;
  const events = stats.ok ? num(stats.data.latency?.events) : undefined;
  return {
    status: {
      ok: status.ok,
      error: status.ok ? undefined : status.error,
      pendingImages,
      delaySecs: num(images?.delay_secs),
      faucet: f
        ? {
            balanceNear: num(f.balance_near),
            allowanceNear: allowance === null ? null : num(allowance),
            creditNear: num(f.credit_near),
            program: num(f.program),
            vending: str(f.vending),
          }
        : undefined,
      alerts: status.ok && Array.isArray(status.data.alerts) ? status.data.alerts.filter((a) => a.component === "practice" || a.component === "images") : [],
    },
    practice: practice.ok
      ? {
          ok: true,
          vending: str(practice.data.vending),
          programId: num(practice.data.program_id),
          publicVendKey: str(practice.data.public_vend_key),
          balanceYocto: practice.data.vending_balance_yocto ?? null,
        }
      : { ok: false, error: practice.error },
    // The indexer numbers every event it writes from 0, so the count is the next event's number; replaying
    // a few before it costs nothing (the try matches its own commit and run).
    streamAfter: events !== undefined ? Math.max(0, Math.floor(events) - 50) : undefined,
    t2: t2.ok ? "live" : t2NotYet(t2) ? "not-yet" : "unknown",
    readAt: status.at,
  };
}

/** The faucet thresholds `/v1/status` alerts on (docs/api.md, Monitoring). */
export const FAUCET_MIN_BALANCE_NEAR = 0.5;
export const FAUCET_MIN_ALLOWANCE_NEAR = 0.05;
/** The vending contract's NEAR credit at near-antibody under which our status warns (status.rs `vending_credit_near`). */
export const FAUCET_MIN_CREDIT_NEAR = 0.1;
/** What one try takes from that credit: its verification run's deposit and storage (measured on testnet, 2026-11-07). */
export const TRY_CREDIT_NEAR = 0.048;
