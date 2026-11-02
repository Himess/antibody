# Drafting invariants (`antibody invariants draft`)

A program pays when a PoC breaks one of its committed invariants. The hardest part of opening a
program is therefore writing invariants that hold for every honest use and are precise enough to pay
for real breaks. `antibody invariants draft` is a defensive helper for that step. It writes a draft;
the project reviews it, edits it and approves it. Nothing is binding until the project commits the
runtime code in its manifest.

```bash
antibody invariants draft --source contracts/ --out draft/          # deterministic, offline
antibody invariants draft --source contracts/ --out draft/ --ai     # plus Claude's extension (ANTHROPIC_API_KEY)
```

## What it writes

| File | Content |
|---|---|
| `DraftInvariants.sol` | A compiling `IAntibodyInvariants` contract with the invariants the scan can express completely |
| `slots.toml` | `[[invariants.slots]]` with a suggested severity for each bit |
| `REVIEW.md` | What each invariant assumes ("holds only if …"), templates that need your numbers, and the validation steps |
| `noop.bundle.json` | A bundle that changes nothing. `antibody check` with it must report `NoViolation`, which shows that every invariant holds at the anchor |
| `DraftInvariants.ai.sol`, `invariants.ai.json` | With `--ai` only: Claude's extension, with a rationale and the assumptions for each invariant |

## The deterministic scan

It reads the Solidity sources: contracts, inheritance, functions, and public state variables, telling
apart those that can change from `immutable` and `constant` ones. It drafts only what it can express
without guessing:
- **Share price never falls (medium).** For an ERC-4626-style vault (`totalAssets` + `totalSupply`). It
  holds only if the vault can never realize a loss or charge an exit fee.
- **A mutable `owner()` / `admin()` never changes (critical).** PoCs run from fresh senders, so a change
  means a broken access rule. Immutable roles are skipped because they cannot change.

Shapes that need your economics become templates in `REVIEW.md`:
- lending solvency at a reference price you commit;
- token custody against recorded liabilities.

Pausable contracts get a note: pausing is not a break.

## The AI pass (optional)

`--ai` sends the sources and the deterministic draft to the Claude API:
- model `claude-opus-5-5`, effort `high`;
- structured JSON output via `output_config.format`;
- server-side refusal fallback `fallbacks: "default"`.

The system prompt asks only for properties that must always hold, as view-only, gas-bounded,
non-reverting Solidity that keeps the deterministic draft. It explicitly asks for no description of
how a property could be violated and no attack code. Doubtful properties come back as questions in
`review_notes`. Credentials come from `ANTHROPIC_API_KEY` (or an OAuth token in
`ANTHROPIC_AUTH_TOKEN`) and are never printed. The output is a draft like the deterministic one.

## Validate before committing

1. Compile the draft with your target addresses and take the runtime code from a simulated deployment.
2. Put the runtime code, its keccak256 and the slots into `antibody.toml`, then run `antibody manifest`.
3. Run `antibody check --manifest manifest.json --bundle noop.bundle.json`. It must report
   `NoViolation`; the verifier reports `ProgramMisconfigured` if `check(snapshot()) != 0` at the anchor.
4. Run your known-issue PoCs and declare them at t0.

## Evidence

- **Rust tests (`crates/cli/src/draft.rs`).** On the Acme Lend sources the scan:
  - rediscovers the program's share-price rule (`AcmeInvariants` bit 2);
  - offers solvency, bit 0, as a template for `AcmeLend` and `AcmeLendFixed`;
  - drafts no role invariant, because Acme's admins are immutable.

  The output must equal the golden file `contracts/evm/test/drafts/DraftInvariants.sol`.
- **Foundry (`contracts/evm/test/drafts/DraftInvariants.t.sol`).** The golden draft compiles and holds at
  the fixture's anchor. It catches the known issue (share rounding) and does not report solvency.
- **The AI pass.** The request shape (model, structured output schema, fallbacks, no thinking budget)
  and the handling of `end_turn`, `refusal` and `max_tokens` responses are unit-tested. It has **not
  been run live** here, because no Claude API credentials were available in this environment.
