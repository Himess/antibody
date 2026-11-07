import recorded from "@/data/moments.json";
import { api, type Moment, type Moments } from "@/lib/api";

/**
 * The one moment's runs: the API's `/v1/moments` (read from the results files), and the bundled snapshot
 * of the same endpoint when the API cannot be reached (`npm run sync-data` refreshes it). A later
 * scheduled drill appears in the list with no change here.
 */
export async function getMoments(): Promise<{ data: Moments; live: boolean }> {
  const r = await api.moments();
  if (r.ok && r.data.moments?.length) return { data: r.data, live: true };
  return { data: recorded as unknown as Moments, live: false };
}

export const latestMoment = (m: Moments): Moment => m.moments.find((x) => x.id === m.latest) ?? m.moments[0];

export type Lane = "near" | "evm:84532" | "evm:11155111" | string;

export const laneName = (lane: Lane) =>
  lane === "near" ? "NEAR testnet" : lane === "evm:84532" ? "Base Sepolia" : lane === "evm:11155111" ? "Sepolia" : lane;

/** The lanes a moment uses, NEAR first, then each chain in the order it paused. */
export function lanesOf(m: Moment): Lane[] {
  const out: Lane[] = ["near"];
  for (const s of m.steps) if (!out.includes(s.chain)) out.push(s.chain);
  return out;
}

/** The commit as step zero, then the recorded steps. */
export function stepsWithCommit(m: Moment) {
  return [
    {
      key: "commit",
      label: "Committed",
      seconds: 0,
      seconds_exact: 0,
      chain: "near",
      tx: m.commit.tx,
      explorer: m.commit.explorer,
      detail: { by: `a sealed proof, committed by ${m.hunter ?? "the hunter"}${m.via ? ` (${m.via})` : ""}` },
    },
    ...m.steps,
  ];
}

export const momentEnd = (m: Moment) => Math.max(...m.steps.map((s) => s.seconds_exact), 1);
