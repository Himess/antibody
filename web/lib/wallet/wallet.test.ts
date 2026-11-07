import { describe, expect, it } from "vitest";
import { formatUnits, parseUnits } from "./amounts";
import { isImplicit, isTestnetAccount, validAccountId } from "./account";
import { classify, failureText } from "./connector";
import { ANTIBODY, MIN_NEAR_CREDIT, WNEAR } from "./contracts";
import { WALLET_CONNECT_SRC } from "./csp";
import { PINNED_WALLETS, WALLET_INFO } from "./manifest";
import { depositTokens, planDeposit, planRegisterAgent, planWithdraw, toConnectorActions, withdrawTokens } from "./tx";

const unit = BigInt(10) ** BigInt(24);

describe("amounts", () => {
  it("reads whole units exactly, as the CLI's parse_tokens", () => {
    expect(parseUnits("1")).toEqual({ ok: true, raw: unit });
    expect(parseUnits("0.5")).toEqual({ ok: true, raw: unit / BigInt(2) });
    expect(parseUnits(".25")).toEqual({ ok: true, raw: unit / BigInt(4) });
    expect(parseUnits("0.000000000000000000000001")).toEqual({ ok: true, raw: BigInt(1) });
    expect(parseUnits("1.0000000000000000000000001").ok).toBe(false);
    expect(parseUnits("1,5").ok).toBe(false);
    expect(parseUnits("-1").ok).toBe(false);
    expect(parseUnits("").ok).toBe(false);
    expect(parseUnits(".").ok).toBe(false);
    expect(parseUnits("1e3").ok).toBe(false);
  });

  it("writes raw units back without loss", () => {
    expect(formatUnits(unit * BigInt(3) + BigInt(1))).toBe("3.000000000000000000000001");
    expect(formatUnits("100000000000000000000000")).toBe("0.1");
    expect(formatUnits(BigInt(0))).toBe("0");
  });
});

describe("account ids", () => {
  it("accepts testnet names and implicit accounts only", () => {
    expect(isTestnetAccount("alice.testnet")).toBe(true);
    expect(isTestnetAccount("a".repeat(64).replace(/a/g, "f"))).toBe(true);
    expect(isTestnetAccount("alice.near")).toBe(false);
    expect(isTestnetAccount("Alice.testnet")).toBe(false);
    expect(validAccountId("agent.alice.testnet")).toBe(true);
    expect(validAccountId("a..b.testnet")).toBe(false);
    expect(isImplicit("0x" + "ab".repeat(20))).toBe(true);
  });
});

describe("transactions", () => {
  it("deposits with the CLI's message and one yoctoNEAR", () => {
    const tx = depositTokens(unit);
    expect(tx.receiver).toBe(WNEAR);
    const [a] = toConnectorActions(tx);
    expect(a.params.methodName).toBe("ft_transfer_call");
    expect(a.params.args).toEqual({ receiver_id: ANTIBODY, amount: unit.toString(), msg: '"deposit"' });
    expect(a.params.deposit).toBe("1");
    expect(a.params.gas).toBe("300000000000000");
  });

  it("withdraws with one yoctoNEAR", () => {
    const [a] = toConnectorActions(withdrawTokens(unit));
    expect(a.params).toEqual({ methodName: "withdraw", args: { amount: unit.toString() }, gas: "60000000000000", deposit: "1" });
  });

  it("plans a deposit: wrap what is missing, top the NEAR credit up, then transfer", () => {
    const plan = planDeposit({ account: "you.testnet", amount: unit * BigInt(2), walletTokens: unit, nearCredit: BigInt(0), registered: false, registration: BigInt(125), wrapMissing: true });
    if ("error" in plan) throw new Error(plan.error);
    expect(plan.txs.map((t) => [t.receiver, t.calls.map((c) => c.method)])).toEqual([
      [WNEAR, ["storage_deposit", "near_deposit"]],
      [ANTIBODY, ["storage_deposit"]],
      [WNEAR, ["ft_transfer_call"]],
    ]);
    expect(plan.txs[0].calls[1].deposit).toBe(unit);
    expect(plan.txs[1].calls[0].deposit).toBe(MIN_NEAR_CREDIT);
    expect(plan.cli).toBe("antibody --account you.testnet deposit --amount 2");
  });

  it("refuses a deposit larger than the wallet holds unless it may wrap", () => {
    const plan = planDeposit({ account: "you.testnet", amount: unit * BigInt(2), walletTokens: unit, nearCredit: MIN_NEAR_CREDIT, registered: true, registration: BigInt(0), wrapMissing: false });
    expect("error" in plan).toBe(true);
  });

  it("registers with wrap.testnet before a withdrawal when needed", () => {
    expect(planWithdraw({ account: "you.testnet", amount: unit, registered: false, registration: BigInt(125) }).txs.map((t) => t.calls[0].method)).toEqual(["storage_deposit", "withdraw"]);
    expect(planWithdraw({ account: "you.testnet", amount: unit, registered: true, registration: BigInt(125) }).txs).toHaveLength(1);
  });

  it("registers an agent as the CLI does", () => {
    const plan = planRegisterAgent({ owner: "you.testnet", agent: "bot.you.testnet", budget: unit * BigInt(10), cap: unit * BigInt(2), programs: [5, 6], nearCredit: MIN_NEAR_CREDIT, update: false });
    expect(plan.txs).toHaveLength(1);
    expect(plan.txs[0].calls[0].args).toEqual({ agent: "bot.you.testnet", bond_budget: (unit * BigInt(10)).toString(), per_commit_cap: (unit * BigInt(2)).toString(), programs: [5, 6] });
    expect(plan.cli).toBe("antibody --account you.testnet agent register --agent bot.you.testnet --budget 10 --per-commit-cap 2 --program 5 --program 6");
  });
});

describe("wallet manifest and CSP", () => {
  it("allows every pinned script by its exact path, and nothing on those hosts beyond it", () => {
    for (const w of PINNED_WALLETS) {
      const info = WALLET_INFO.find((i) => i.id === w.id);
      expect(info?.script.url).toBe(w.executor);
      expect(WALLET_CONNECT_SRC).toContain(w.executor);
      expect(w.features.testnet).toBe(true);
      expect(w.permissions.walletConnect).toBeFalsy();
    }
    for (const i of WALLET_INFO) for (const c of i.contacts) if (/^(https|wss):/.test(c.origin)) expect(WALLET_CONNECT_SRC).toContain(c.origin);
    expect(WALLET_CONNECT_SRC.some((s) => s === "https://raw.githubusercontent.com")).toBe(false);
  });

  it("keeps icons inline", () => {
    for (const w of PINNED_WALLETS) expect(w.icon.startsWith("data:image/svg+xml,")).toBe(true);
  });
});

describe("errors", () => {
  it("reads a contract's panic message", () => {
    const f = JSON.stringify({ ActionError: { index: 0, kind: { FunctionCallError: { ExecutionError: "Smart contract panicked: forbidden: an agent cannot withdraw" } } } });
    expect(failureText(f)).toBe("forbidden: an agent cannot withdraw");
  });

  it("sorts wallet errors by what a person can do", () => {
    expect(classify(new Error("User rejected")).kind).toBe("rejected");
    expect(classify("Wallet closed").kind).toBe("rejected");
    expect(classify({ message: "Failed to load executor: 429" }).kind).toBe("load");
    expect(classify("Popup blocked").kind).toBe("popup-blocked");
    expect(classify("something else").kind).toBe("wallet");
  });
});
