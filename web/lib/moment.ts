import recorded from "@/data/moments.json";
import { api, type Moment, type MomentStep, type Moments } from "@/lib/api";
import { fixtureAgentLetter, wnear } from "@/lib/format";

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

const newest = (xs: Moment[]) => [...xs].sort((a, b) => b.started_unix - a.started_unix)[0];

/** The hero: the newest recorded full run. A drill runs the same loop every day and is listed under it, labelled. */
export const recordedMoment = (m: Moments): Moment => newest(m.moments.filter((x) => x.kind === "recorded")) ?? latestMoment(m);

/** The newest automated drill, when one has run. */
export const latestDrill = (m: Moments): Moment | null => newest(m.moments.filter((x) => x.kind === "drill")) ?? null;

/** A run's step, by key: \"verified\", \"t1\", \"paid\", \"paused_<chain id>\"... */
export const stepOf = (m: Moment, key: string) => m.steps.find((s) => s.key === key);

/** When the last chain paused, if every chain did. */
export const pausedBy = (m: Moment) => {
  const paused = m.steps.filter((s) => s.key.startsWith("paused_"));
  return m.paused_everywhere && paused.length ? Math.max(...paused.map((s) => s.seconds)) : null;
};

/** The landing's "Watch the one moment" fires this; the panel replays it (unless motion is reduced). */
export const PLAY_EVENT = "antibody:play-moment";

export type Lane = "near" | "evm:84532" | "evm:11155111" | string;

export const laneName = (lane: Lane) =>
  lane === "near" ? "NEAR testnet" : lane === "evm:84532" ? "Base Sepolia" : lane === "evm:11155111" ? "Sepolia" : lane;

/** The lanes a moment uses, NEAR first, then each chain in the order it paused. */
export function lanesOf(m: Moment): Lane[] {
  const out: Lane[] = ["near"];
  for (const s of m.steps) if (!out.includes(s.chain)) out.push(s.chain);
  return out;
}

/** Who committed, from the record: a fixture agent's letter ("A", from via "A (MCP)"), else the account; and over what. */
export function committer(m: Moment): { name: string; over: string | null } {
  const v = /^([A-Z])\s*\((.+)\)$/.exec(m.via ?? "");
  if (v) return { name: v[1], over: v[2] };
  return { name: (m.hunter ? fixtureAgentLetter(m.hunter) : undefined) ?? m.hunter ?? "the hunter", over: m.via || null };
}

export const possessive = (name: string) => `${name}'s`;

/** The pauses a moment recorded (its steps on EVM chains), and how to say how many chains that is. */
export const pausesOf = (m: Moment) => m.steps.filter((s) => s.chain.startsWith("evm:"));
export const chainsWord = (n: number) => (n === 1 ? "its chain" : n === 2 ? "both chains" : `all ${n} chains`);

/** The payment in words, from its record: the amount, who got it, and its lane. */
function paidBy(m: Moment, d: MomentStep["detail"]): string | undefined {
  const amount = typeof d.amount_raw === "string" ? d.amount_raw : null;
  const to = typeof d.beneficiary === "string" ? d.beneficiary : null;
  if (!amount || !to) return undefined;
  const lane = d.instant === true ? `an instant payout (${m.verdict?.tier || "T1"}, under the cap)` : "after its challenge window";
  return `${wnear(amount)} to ${to} from the program's escrow: ${lane}`;
}

/** The commit as step zero, then the recorded steps, each with a line saying what happened. */
export function stepsWithCommit(m: Moment) {
  const { name, over } = committer(m);
  const who = m.hunter ?? (name === "the hunter" ? name : `agent ${name}`);
  return [
    {
      key: "commit",
      label: "Committed",
      seconds: 0,
      seconds_exact: 0,
      chain: "near",
      tx: m.commit.tx,
      explorer: m.commit.explorer,
      detail: { by: `a sealed proof, committed by ${who}${over ? ` over ${over}` : ""}`, block: m.commit.block_height } as MomentStep["detail"],
    },
    ...m.steps.map((s) => (s.detail?.by || s.key !== "paid" ? s : { ...s, detail: { ...s.detail, by: paidBy(m, s.detail ?? {}) } })),
  ];
}

export type Step = ReturnType<typeof stepsWithCommit>[number];

/** How a step's seconds were taken, where that is not simply its NEAR block (the results file's `clock`). */
export function timedNote(s: Step): string | null {
  if (s.key === "pause_asked") return "from the indexer's pause_ticket event";
  if (s.chain.startsWith("evm:")) return "when the relayer got its receipt";
  return null;
}

/** What a step's link is, where one transaction serves two steps or the step lands in a later receipt. */
export function txNote(s: Step, steps: Step[], who: string): string | null {
  const first = s.tx ? steps.find((x) => x.tx === s.tx) : undefined;
  if (first && first.key !== s.key) return `same transaction as ${first.label}, a later receipt`;
  if (s.key === "verified") return `${possessive(who)} reveal; the answer lands in a later receipt`;
  // The commit's NEAR block is the clock's zero; each pause's block is on its own chain (the API gives it in hex).
  const raw = s.detail?.block;
  const block = typeof raw === "number" ? raw : typeof raw === "string" && /^0x[0-9a-f]+$/i.test(raw) ? parseInt(raw, 16) : null;
  if (block === null) return null;
  return s.key === "commit" ? `in block ${block}, the clock's zero` : `in block ${block}`;
}

export const momentEnd = (m: Moment) => Math.max(...m.steps.map((s) => s.seconds_exact), 1);
