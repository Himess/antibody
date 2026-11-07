import { describe, expect, it } from "vitest";
import recorded from "@/data/moments.json";
import type { Moment, Moments } from "@/lib/api";
import { latestDrill, pausedBy, recordedMoment, stepOf } from "@/lib/moment";

const base = recorded as unknown as Moments;
const run = base.moments[0];
// A drill is the same loop, run later from its own results file (drill-YYYY-MM-DD.json).
const drill: Moment = { ...run, id: "drill-2026-11-10", kind: "drill", started_unix: run.started_unix + 6 * 86_400 };
const older: Moment = { ...drill, id: "drill-2026-11-09", started_unix: run.started_unix + 5 * 86_400 };

describe("the one moment and the drills", () => {
  it("keeps the recorded full run as the hero when drills are newer", () => {
    const m: Moments = { ...base, moments: [drill, older, run], latest: drill.id };
    expect(recordedMoment(m).id).toBe(run.id);
    expect(latestDrill(m)?.id).toBe(drill.id);
  });

  it("has no latest drill before one runs", () => {
    expect(latestDrill(base)).toBeNull();
    expect(recordedMoment(base).id).toBe(run.id);
  });

  it("reads a run's payout and its last pause", () => {
    expect(stepOf(run, "paid")?.seconds).toBe(71);
    expect(pausedBy(run)).toBe(90.6);
    expect(pausedBy({ ...run, paused_everywhere: false })).toBeNull();
  });
});
