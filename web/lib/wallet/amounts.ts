/**
 * Exact decimal amounts for transactions: whole units typed by a person to raw integer units (24 decimals
 * for both wNEAR and NEAR), as the CLI's `parse_tokens` reads them, and back. No floating point anywhere.
 */
import { DECIMALS } from "./contracts";

export type Parsed = { ok: true; raw: bigint } | { ok: false; error: string };

const TEN = BigInt(10);
const ZERO = BigInt(0);

/** "1.5" → 1500000000000000000000000n. Digits and one dot; at most `decimals` fraction digits. */
export function parseUnits(input: string, decimals = DECIMALS, unit = "wNEAR"): Parsed {
  const s = input.trim();
  if (!s) return { ok: false, error: `Enter an amount in ${unit}.` };
  if (!/^\d*\.?\d*$/.test(s) || s === ".") return { ok: false, error: `Use digits and one dot, as in 0.5 (${unit}).` };
  const [i = "", f = ""] = s.split(".");
  if (f.length > decimals) return { ok: false, error: `At most ${decimals} digits after the dot.` };
  const raw = BigInt(i || "0") * TEN ** BigInt(decimals) + BigInt((f || "0").padEnd(decimals, "0").slice(0, decimals) || "0");
  return { ok: true, raw };
}

/** Raw units as an exact decimal string with no grouping and no trailing zeros ("1.5", "0.000001"). */
export function formatUnits(raw: bigint | string, decimals = DECIMALS): string {
  let v = typeof raw === "bigint" ? raw : BigInt(raw);
  const neg = v < ZERO;
  if (neg) v = -v;
  const base = TEN ** BigInt(decimals);
  const int = (v / base).toString();
  const frac = (v % base).toString().padStart(decimals, "0").replace(/0+$/, "");
  return `${neg ? "-" : ""}${int}${frac ? "." + frac : ""}`;
}

/** A raw amount field from the API or a view (decimal string), or null when absent or malformed. */
export function rawOf(v: unknown): bigint | null {
  if (typeof v === "bigint") return v;
  if (typeof v === "number" && Number.isSafeInteger(v)) return BigInt(v);
  if (typeof v === "string" && /^-?\d+$/.test(v)) return BigInt(v);
  return null;
}

export const max = (a: bigint, b: bigint) => (a > b ? a : b);
export const min = (a: bigint, b: bigint) => (a < b ? a : b);
