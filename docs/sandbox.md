# The whole story, offline

The story the testnet run tells (a program, the one moment, every kind of verdict, the rounds, the release)
also runs offline, in a sandbox: near-sandbox and anvil, with MOCK OutLayer, MOCK DCAP quotes (checked by the
real T1 code) and MOCK MPC. On Linux or macOS:

```bash
make serve       # builds everything, runs the whole story (every outcome asserted), then keeps the sandbox up
```

The first build takes a while (the verifier, the contracts, the CLI); the story then runs for about nine
minutes and ends with `--serve: near-sandbox http://127.0.0.1:3030 …`. Then, in a second terminal in the
repository (bash or zsh):

```bash
A() { ./target/release/antibody --network sandbox --rpc http://127.0.0.1:3030 --deployments target/demo/deployments.json "$@"; }
A judge                  # view calls only: locked status, T1 confirmations, payouts, permits, audits (--json: everything)
A try --name <you>       # a new lowercase name: the practice program (a tutorial), no account, no key; paid in seconds
```

`make demo` runs the same story once and stops (`docker build -t antibody-demo . && docker run --rm
antibody-demo` elsewhere). On testnet, a build of the CLI from this repository has the deployment built in
(`cargo build --release -p antibody-cli`): `./target/release/antibody judge`, and
`./target/release/antibody try --name <you>` for the practice program, with no account and no key.

The sandbox run (`target/demo/results.json`; the recorded one is in [results/](results/)) asserts every
outcome of the story:

| Step | What happens | Recorded |
|---|---|---|
| Program | Acme Lend's manifest (schema v2) and escrow on NEAR; the anchor checked by the light client; activation pins the base witness and the program's sealing key | T1 |
| The one moment | Agent A, over MCP, submits a sealed proof of a Critical. From its commit's block: verified at 7.9 s, T1-confirmed at 11.5 s, **both chains paused** by 24.5 s, and A **paid** at 24.9 s (the pause is asked for before the payment) | each moment's own block or receipt, sandbox (MOCK OutLayer, DCAP and MPC) |
| A NEAR app reacts | A DAO treasury that watches the program reads near-antibody and freezes on A's verified Critical | its next transfer is refused |
| B–G | B paid; C (a prompt injection in its report) breaks nothing and forfeits its bond; D a duplicate, confirmed at the fix; E a known issue; F paid; G harms a protected user without breaking a rule: refunded, recorded, and paid once round 1 adds the rule | every state |
| The project reads it | After payment, A's proof is wrapped to the project's NEP-413-derived key; `antibody program read` opens it and re-runs it locally, and near-antibody refuses to wrap D's duplicate | the local receipt hash equals the TEE's |
| Rounds | Round 1's partial fix: every paid proof re-run at it, two still break, so the release gate stays shut and no release permit is signed; round 2's final fix: all fail, the release permits upgrade both proxies on both chains, and B's exploit reverts | the release gate |
| Practice | Two judges with no account vend practice accounts and run the tutorial proof; it is sealed against the pinned base alone | Done:Paid |
| Fork mode | The practice program again, deployed only inside the verifier: nothing of it is on chain | Done:Paid |
| Audits | Every conservation equation, per program and for the contract | ok |

The site runs locally against the same recorded snapshots: [web/README.md](../web/README.md).
