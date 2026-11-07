/**
 * The rest of the vending contract's `get_config` (contracts/practice-vending `get_config`, Borsh):
 * `(owner, antibody, token, program_id: u32, vaults: u32, terms: Terms, vended: u32)`, where `Terms` is
 * four u128 amounts (gas funds, the key allowance, the bond budget, the per-commit cap) and `per_hour: u32`.
 * The CLI reads only the program id (near-tx.ts `parseVendingConfig`); the try page also shows the hourly
 * vend limit, so the later fields are read here, leniently: a missing field is undefined, never an error.
 * How many accounts are left this hour is in the contract's own state, not in `get_config` (`parseVendingHour`).
 */
import { ByteReader } from "./bytes";
import { parseVendingConfig, type VendingConfig } from "./near-tx";

export interface VendingTerms {
  /** yoctoNEAR sent to each practice account for its storage and gas (`close` returns what is left). */
  gasFunds: bigint;
  /** yoctoNEAR: the practice key's allowance. */
  allowance: bigint;
  /** Raw token units of the vending contract's credit a practice account may bond, in all and per commit. */
  bondBudget: bigint;
  perCommitCap: bigint;
  /** Practice accounts vended per clock hour (UTC), at most. */
  perHour: number;
}

export interface VendingConfigFull extends VendingConfig {
  vaults?: number;
  terms?: VendingTerms;
  /** Practice accounts vended so far. */
  vended?: number;
}

function u128(r: ByteReader): bigint {
  const b = r.take(16);
  let n = BigInt(0);
  for (let i = 15; i >= 0; i--) n = (n << BigInt(8)) | BigInt(b[i]);
  return n;
}

/** `get_config` in full: the program id exactly as the CLI reads it, then the later fields when present. */
export function parseVendingConfigFull(b: Uint8Array): VendingConfigFull {
  const base = parseVendingConfig(b);
  const r = new ByteReader(b);
  const out: VendingConfigFull = { ...base };
  try {
    for (let i = 0; i < 3; i++) r.take(r.u32le());
    r.u32le(); // the program id, read above
    out.vaults = r.u32le();
    const terms: VendingTerms = { gasFunds: u128(r), allowance: u128(r), bondBudget: u128(r), perCommitCap: u128(r), perHour: r.u32le() };
    out.terms = terms;
    out.vended = r.u32le();
  } catch {
    // A shorter config (an older contract) still names its program: the rest stays unknown.
  }
  return out;
}

/** The start of the next clock hour (UTC), in ms: when the vending contract's hourly count resets. */
export function nextHourMs(nowMs: number): number {
  return (Math.floor(nowMs / 3_600_000) + 1) * 3_600_000;
}

/** The `STATE` key, base64: where near-sdk keeps a contract's own fields (a `view_state` prefix). */
export const STATE_KEY_BASE64 = "U1RBVEU=";

/** The vending contract's hourly count: the clock hour of its latest vend, and how many it vended in that hour. */
export interface VendingHour {
  /** Hours since the epoch (UTC). */
  hour: number;
  inHour: number;
}

/**
 * The hourly count from the vending contract's own state (`STATE`, Borsh, read with a `view_state` query): the
 * fields `get_config` returns, in the same order, then `hour: u64` and `in_hour: u32`. No view returns these
 * two; the contract is locked, so its layout is fixed. Undefined when the state does not parse, or does not
 * match `get_config` (another contract's layout).
 */
export function parseVendingHour(state: Uint8Array, cfg?: VendingConfigFull): VendingHour | undefined {
  let full: VendingConfigFull;
  try {
    full = parseVendingConfigFull(state);
  } catch {
    return undefined;
  }
  if (!full.terms || full.vended === undefined) return undefined;
  if (cfg && (cfg.programId !== full.programId || (cfg.terms !== undefined && cfg.terms.perHour !== full.terms.perHour))) return undefined;
  const r = new ByteReader(state);
  try {
    for (let i = 0; i < 3; i++) r.take(r.u32le());
    // program_id, vaults, the terms (four u128 and per_hour) and vended, read above
    r.take(4 + 4 + 4 * 16 + 4 + 4);
    const hour = Number(r.u64le());
    const inHour = r.u32le();
    return Number.isSafeInteger(hour) && inHour <= full.terms.perHour ? { hour, inHour } : undefined;
  } catch {
    return undefined;
  }
}

/** Practice accounts left in the current clock hour: `vend` starts a new count when a new hour begins. */
export function vendsLeft(perHour: number, h: VendingHour, nowMs: number): number {
  return h.hour >= Math.floor(nowMs / 3_600_000) ? Math.max(0, perHour - h.inHour) : perHour;
}
