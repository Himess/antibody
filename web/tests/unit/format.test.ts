import { describe, expect, it } from "vitest";
import { rulesText, verdictWords } from "@/components/commit/model";
import { practiceTry } from "@/components/programs/helpers";
import { amount, provenance, seconds, shortHash, stateLabel, utc, wnear } from "@/lib/format";
import { LEVEL, levelOf, plainLevels } from "@/lib/levels";
import { lanesOf, momentEnd, stepsWithCommit } from "@/lib/moment";
import recorded from "@/data/moments.json";
import type { Moments, ProgramDetail } from "@/lib/api/types";

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

describe("plain words for rules, levels and our own tries", () => {
  const program = {
    rules: {
      slots: [
        { bit: 0, name: "solvency", severity: "critical" },
        { bit: 1, name: "fees_to_treasury", severity: "high" },
      ],
    },
  } as unknown as ProgramDetail;

  it("names a rule in prose, with its slot number only where the program names none", () => {
    expect(rulesText(program, [0])).toBe("the solvency rule");
    expect(rulesText(program, [0, 1])).toBe("the solvency rule and the “fees to treasury” rule");
    expect(rulesText(program, [7])).toBe("rule 7");
    expect(verdictWords("Violation", rulesText(program, [0]))).toBe("violation of the solvency rule");
    expect(verdictWords("NoViolation", "")).toBe("no violation");
  });

  it("names the three levels in words, the codes kept for the tooltips", () => {
    expect([LEVEL.T0.name, LEVEL.T1.name, LEVEL.T2.name]).toEqual(["Enclave verdict", "Attested on NEAR", "Zero-knowledge proof"]);
    for (const l of ["T0", "T1", "T2"] as const) expect(LEVEL[l].title.startsWith(`Level ${l}:`)).toBe(true);
    expect([levelOf("T0"), levelOf("T1"), levelOf("T2"), levelOf("")]).toEqual(["T0", "T1", "T2", "T0"]);
  });

  it("says the API's own sentences without the levels' codes", () => {
    expect(plainLevels("6 recently listed worker image(s) count for existing programs only from 2026-11-08T13:15:22Z: a run on one gets no T1 before then")).toBe(
      "6 recently listed worker image(s) count for existing programs only from 2026-11-08T13:15:22Z: a run on one cannot be attested on NEAR before then",
    );
    expect(plainLevels("t1-relayer (relayer.antibody.testnet) has 0.359 NEAR liquid, under its 0.5 NEAR float")).toBe(
      "attestation relayer (relayer.antibody.testnet) has 0.359 NEAR liquid, under its 0.5 NEAR float",
    );
    expect(plainLevels("the TEE's answer (T0), in the block that resolved the run")).toBe("the enclave's result, in the block that resolved the run");
    expect(plainLevels("the TEE (OutLayer worker)")).toBe("the enclave (OutLayer worker)");
    expect(plainLevels("2 permit(s) not yet confirmed on their chain")).toBe("2 permit(s) not yet confirmed on their chain");
  });

  it("labels our own practice tries as ours", () => {
    expect(practiceTry("rel-linux-check.practice.antibody.testnet", 6)).toBe("our check of the release binary");
    expect(practiceTry("scrn-k7q2-1107.practice.antibody.testnet", 6)).toBe("our screener dry run");
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
  it("says every step in plain words: no level code, no TEE", () => {
    const steps = stepsWithCommit(m);
    expect(steps.find((s) => s.key === "t1")?.label).toBe("Attested on NEAR");
    for (const s of steps) expect(`${s.label} ${s.detail?.by ?? ""}`, s.key).not.toMatch(/\bT[012]\b|T1-confirmed|\bTEE\b/);
  });
});
