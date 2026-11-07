/** Formatting shared by every page: amounts, times, hashes and the contract's state names. */

export const TOKEN = { symbol: "wNEAR", decimals: 24 };

/** A raw token amount (decimal string) as whole units, trailing zeros trimmed: "100000…000" → "0.1". */
export function amount(raw: string | number | bigint | null | undefined, decimals = TOKEN.decimals): string {
  if (raw === null || raw === undefined || raw === "") return "—";
  let v: bigint;
  try {
    v = BigInt(typeof raw === "number" ? Math.trunc(raw) : raw);
  } catch {
    return String(raw);
  }
  const neg = v < 0n;
  if (neg) v = -v;
  const base = 10n ** BigInt(decimals);
  const int = v / base;
  const frac = (v % base).toString().padStart(decimals, "0").replace(/0+$/, "");
  const intStr = int.toLocaleString("en-US");
  return `${neg ? "-" : ""}${intStr}${frac ? "." + frac.slice(0, 6) : ""}`;
}

export const wnear = (raw: string | number | bigint | null | undefined) => `${amount(raw)} ${TOKEN.symbol}`;

/** yoctoNEAR as NEAR with a few decimals. */
export const near = (yocto: string | number | null | undefined, digits = 3) => {
  if (yocto === null || yocto === undefined) return "—";
  const n = Number(yocto) / 1e24;
  return `${n.toLocaleString("en-US", { maximumFractionDigits: digits })} NEAR`;
};

export function seconds(s: number | null | undefined, digits = 1): string {
  if (s === null || s === undefined || !Number.isFinite(s)) return "—";
  if (s >= 3600) return `${(s / 3600).toFixed(1)} h`;
  if (s >= 600) return `${Math.round(s / 60)} min`;
  return `${s.toFixed(digits)} s`;
}

/** Unix seconds (or ns) as UTC: "2026-11-04 15:42:11 UTC". */
export function utc(t: number | null | undefined, unit: "s" | "ns" | "ms" = "s"): string {
  if (!t) return "—";
  const ms = unit === "s" ? t * 1000 : unit === "ns" ? t / 1e6 : t;
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return "—";
  return d.toISOString().replace("T", " ").replace(/\.\d+Z$/, " UTC");
}

export function isoOf(t: number, unit: "s" | "ns" | "ms" = "s"): string {
  const ms = unit === "s" ? t * 1000 : unit === "ns" ? t / 1e6 : t;
  return new Date(ms).toISOString();
}

/** "3 min ago", "in 2 h" relative to `now` (ms). */
export function relative(t: number, now: number, unit: "s" | "ns" | "ms" = "s"): string {
  const ms = unit === "s" ? t * 1000 : unit === "ns" ? t / 1e6 : t;
  const d = Math.round((ms - now) / 1000);
  const a = Math.abs(d);
  const fmt =
    a < 45
      ? `${a} s`
      : a < 2700
        ? `${Math.round(a / 60)} min`
        : a < 129600
          ? `${Math.round(a / 3600)} h`
          : `${Math.round(a / 86400)} d`;
  return d <= 0 ? `${fmt} ago` : `in ${fmt}`;
}

export function shortHash(h: string | null | undefined, head = 8, tail = 0): string {
  if (!h) return "—";
  const s = String(h);
  if (s.length <= head + tail + 1) return s;
  return tail ? `${s.slice(0, head)}…${s.slice(-tail)}` : `${s.slice(0, head)}…`;
}

/** The contract's commit states, in plain words (the raw label stays visible beside it). */
export function stateLabel(state: string | null | undefined): { text: string; tone: Tone } {
  const s = state ?? "";
  const map: Record<string, { text: string; tone: Tone }> = {
    "Done:Paid": { text: "Paid", tone: "ok" },
    "Done:Forfeited": { text: "No violation · bond forfeited", tone: "neutral" },
    "Done:DuplicateConfirmed": { text: "Duplicate · confirmed at the fix", tone: "neutral" },
    Duplicate: { text: "Duplicate", tone: "neutral" },
    "Done:Known": { text: "Known issue", tone: "neutral" },
    "Done:Refunded": { text: "Refunded", tone: "neutral" },
    "Done:GapRefunded": { text: "Gap claim · refunded", tone: "neutral" },
    Holder: { text: "Verified · awaiting payment", tone: "info" },
    Verifying: { text: "Verifying", tone: "pending" },
    Committed: { text: "Committed · sealed", tone: "pending" },
    Revealed: { text: "Revealed · in the TEE", tone: "pending" },
    GapClaim: { text: "Gap claim recorded", tone: "info" },
  };
  if (map[s]) return map[s];
  if (s.startsWith("Done:")) return { text: s.slice(5).replace(/([a-z])([A-Z])/g, "$1 $2"), tone: "neutral" };
  return { text: s || "—", tone: "neutral" };
}

export type Tone = "ok" | "bad" | "warn" | "caution" | "info" | "pending" | "neutral" | "accent" | "t1" | "t2";

/** Severity is a warm scale of its own: red, amber, amber outlined, grey. Blue stays the T1 tier's. */
export const severityTone: Record<string, Tone> = {
  critical: "bad",
  high: "warn",
  medium: "caution",
  low: "neutral",
};

export const cap = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

/** Our own fixture agents (a–g) and accounts, labelled as ours wherever they appear. */
export function provenance(account: string | null | undefined): "fixture-agent" | "practice" | "ours" | "other" {
  const a = account ?? "";
  if (/^agent-[a-g]\.antibody\.testnet$/.test(a)) return "fixture-agent";
  if (/\.practice\.antibody\.testnet$/.test(a) || a === "practice.antibody.testnet") return "practice";
  if (a.endsWith(".antibody.testnet") || a === "antibody.testnet") return "ours";
  return "other";
}

export const fixtureAgentLetter = (account: string) => /^agent-([a-g])\./.exec(account)?.[1]?.toUpperCase();
