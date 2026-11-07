/**
 * The rest of the vending contract's `get_config` (contracts/practice-vending `get_config`, Borsh):
 * `(owner, antibody, token, program_id: u32, vaults: u32, terms: Terms, vended: u32)`, where `Terms` is
 * four u128 amounts (gas funds, the key allowance, the bond budget, the per-commit cap) and `per_hour: u32`.
 * The CLI reads only the program id (near-tx.ts `parseVendingConfig`); the try page also shows the hourly
 * vend limit, so the later fields are read here, leniently: a missing field is undefined, never an error.
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
