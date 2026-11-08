import { describe, expect, it } from "vitest";
import { isForkMode, KIND_TEXT, parseWindowName, programKind } from "@/components/programs/helpers";

// The names `antibody window open` gives (crates/cli/src/window.rs, `window_name`), read the same way here.
describe("pull-request windows", () => {
  it("reads a window's name back", () => {
    expect(parseWindowName("PR #12 window (antibody-immune-demo @ 1a2b3c4)")).toEqual({ pr: 12, repo: "antibody-immune-demo", head: "1a2b3c4" });
    expect(parseWindowName("PR #12 window (antibody-immune-demo)")).toEqual({ pr: 12, repo: "antibody-immune-demo", head: null });
    expect(parseWindowName("PR #3 window (Himess/antibody-immune-demo @ 0123456789abcdef0123456789abcdef01234567)")?.repo).toBe(
      "Himess/antibody-immune-demo",
    );
  });

  it("labels only what the CLI names a window", () => {
    for (const other of [
      "Acme Lend drill 2026-11-10",
      "Practice program in fork mode — a tutorial (testnet)",
      "PR #12 window",
      "PR #0 window (demo)",
      "PR #012 window (demo)",
      "PR #12 window (demo @ 1A2B3C4)",
      "PR #12 window (demo @ 1a2b3c)",
      "PR #12 window (demo @ 1a2b3c4) extra",
      "pr #12 window (demo)",
      null,
    ]) {
      expect(parseWindowName(other)).toBeNull();
    }
  });

  it("is its own kind of program, in fork mode", () => {
    expect(programKind("PR #12 window (antibody-immune-demo @ 1a2b3c4)", "Acme Lend at this pull request's build (DELIBERATELY VULNERABLE TEST FIXTURE)")).toBe("window");
    expect(programKind("Practice program in fork mode — a tutorial (testnet)")).toBe("fork");
    expect(programKind("Acme Lend drill 2026-11-10")).toBe("fixture");
    expect(KIND_TEXT.window.short).toBe("Pull-request window");
    expect(isForkMode("window") && isForkMode("fork") && !isForkMode("fixture")).toBe(true);
  });
});
