import { describe, expect, it } from "vitest";
import { amount, provenance, seconds, shortHash, stateLabel, utc, wnear } from "@/lib/format";
import { lanesOf, momentEnd, stepsWithCommit } from "@/lib/moment";
import recorded from "@/data/moments.json";
import type { Moments } from "@/lib/api/types";

describe("format", () => {
  it("shows raw wNEAR amounts as whole units", () => {
    expect(amount("100000000000000000000000")).toBe("0.1");
    expect(amount("1000000000000000000000")).toBe("0.001");
    expect(amount("240000000000000000000000")).toBe("0.24");
    expect(amount("1394000000000000000000000")).toBe("1.394");
    expect(amount("0")).toBe("0");
    expect(amount(null)).toBe("—");
    expect(wnear("30000000000000000000000")).toBe("0.03 wNEAR");
  });

  it("names the contract's states in plain words", () => {
    expect(stateLabel("Done:Paid")).toEqual({ text: "Paid", tone: "ok" });
    expect(stateLabel("Done:DuplicateConfirmed").text).toMatch(/Duplicate/);
    expect(stateLabel("Done:Forfeited").text).toMatch(/forfeited/);
    expect(stateLabel("Done:SomethingNew").text).toBe("Something New");
  });

  it("labels provenance", () => {
    expect(provenance("agent-a.antibody.testnet")).toBe("fixture-agent");
    expect(provenance("judge-1-p6.practice.antibody.testnet")).toBe("practice");
    expect(provenance("keeper.antibody.testnet")).toBe("ours");
    expect(provenance("alice.testnet")).toBe("other");
  });

  it("times and hashes", () => {
    expect(seconds(18.524)).toBe("18.5 s");
    expect(utc(1793806931)).toBe("2026-11-04 15:42:11 UTC");
    expect(shortHash("CynCsx8PNsFAYnYp6Zk5HjpSg3fWUrHSKfJ9CD9HQ3nP", 8)).toBe("CynCsx8P…");
  });
});

describe("the recorded one moment (data/moments.json, a copy of /v1/moments)", () => {
  const m = (recorded as unknown as Moments).moments[0];
  it("has the README's numbers, each step with a transaction", () => {
    const at = Object.fromEntries(m.steps.map((s) => [s.key, s.seconds]));
    expect(at).toMatchObject({ verified: 18.5, t1: 29.1, permits_signed: 62.7, paid: 71, paused_84532: 87.2, paused_11155111: 90.6 });
    for (const s of m.steps) expect(s.tx, s.key).toBeTruthy();
    expect(m.commit.tx).toBe("CynCsx8PNsFAYnYp6Zk5HjpSg3fWUrHSKfJ9CD9HQ3nP");
  });
  it("asks for the pause before the payment", () => {
    const at = Object.fromEntries(m.steps.map((s) => [s.key, s.seconds_exact]));
    expect(at.pause_asked).toBeLessThan(at.paid);
  });
  it("lays out three lanes from the commit", () => {
    expect(lanesOf(m)).toEqual(["near", "evm:84532", "evm:11155111"]);
    expect(stepsWithCommit(m)[0].key).toBe("commit");
    expect(momentEnd(m)).toBeCloseTo(90.58, 1);
  });
});
