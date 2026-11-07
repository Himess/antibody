/**
 * The contracts this page's transactions go to, fixed in code from deployments/near-testnet.json (the locked
 * testnet deployment). They are never taken from a server's answer, so a compromised API cannot redirect a
 * deposit: the page compares them with the API's /health and with the code on chain before it signs.
 */

export const WALLET_NETWORK = "testnet" as const;

/** near-antibody and the payout token, as deployments/near-testnet.json records them. */
export const RELEASE = {
  antibody: {
    account: "bounty.antibody.testnet",
    codeHash: "C9LGvUpq9P5j5zB8RyZdRaXHWf3tF4wRtb1PjBYKysaY",
    locked: true,
    role: "near-antibody",
  },
  token: {
    account: "wrap.testnet",
    codeHash: "DsbyUrKaSJAT91Pewfi71QExPLBooDp1z4sL9GUDhNHa",
    locked: false,
    role: "payout token (wNEAR)",
  },
  gitCommit: "2777a3eb0afc5765ca91246e85dcfdbee551261b",
} as const;

export const ANTIBODY = RELEASE.antibody.account;
export const WNEAR = RELEASE.token.account;

/** wNEAR and NEAR both have 24 decimals. */
export const DECIMALS = 24;

export const ONE_YOCTO = BigInt(1);
const TGAS = BigInt(10) ** BigInt(12);
export const tgas = (n: number) => BigInt(n) * TGAS;

/** Gas per call, as the CLI attaches it (crates/cli/src/cmd.rs). */
export const GAS = {
  storageDeposit: tgas(30),
  nearDeposit: tgas(30),
  ftTransferCall: tgas(300),
  withdraw: tgas(60),
  storageWithdraw: tgas(30),
  registerAgent: tgas(30),
} as const;

/**
 * The NEAR credit the CLI tops up to before a call that adds storage or requests a run (cmd.rs
 * `MIN_NEAR_CREDIT`, 0.25 NEAR): `deposit` and `agent register` keep at least this much.
 */
export const MIN_NEAR_CREDIT = BigInt("250000000000000000000000");

/** wrap.testnet's storage registration (NEP-145 `storage_balance_bounds().min`), used if the view fails. */
export const WNEAR_REGISTRATION_FALLBACK = BigInt("1250000000000000000000");

/** An agent's bond budget may not exceed 2^100 raw units (crates/rules types::bounds::MAX_AMOUNT). */
export const MAX_AMOUNT = BigInt(2) ** BigInt(100);

/** The fixed `msg` of `ft_transfer_call` that credits the tokens to the sender (TokenMsg::Deposit). */
export const DEPOSIT_MSG = JSON.stringify("deposit");

/** Repository paths that back each claim on the page (linked with sourceUrl). */
export const SOURCES = {
  contract: "contracts/near-antibody/src/lib.rs",
  engine: "crates/rules/src/engine.rs",
  cli: "crates/cli/src/cmd.rs",
  deployment: "deployments/near-testnet.json",
  threatModel: "docs/threat-model.md",
  cryptoSpec: "docs/crypto-spec.md",
} as const;
