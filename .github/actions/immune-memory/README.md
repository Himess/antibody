# Antibody immune memory (GitHub Action)

A prototype of roadmap item 1 ([docs/immune.md](../../../docs/immune.md)). On every pull request it re-runs
each paid proof of your Antibody program that the run can read against the pull request's compiled code,
with the verifier, locally, at the program's anchor. If one still breaks a committed rule, the pull request
re-opens a paid hole and the check fails.

What it does, in order:
1. downloads the pinned `antibody` release binary for the runner and checks its sha256 against the release's
   `.sha256` file (Linux x86_64, macOS arm64, Windows x86_64);
2. installs Foundry and runs `forge build`, with no secret in that step's environment;
3. runs `antibody immune` (view calls and local verifier runs: it sends nothing) and writes its report to
   the job summary;
4. fails the job when a paid proof still breaks a rule (and, by default, when a re-run is inconclusive).

`antibody immune` first ships in release v0.5.0, the default `version`; the release must be published.

## Use

Commit a mapping, `.antibody/immune.toml` (the format: [docs/immune.md](../../../docs/immune.md)), then:

```yaml
name: immune memory
on:
  pull_request:   # never pull_request_target
permissions:
  contents: read
jobs:
  immune:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: Himess/antibody/.github/actions/immune-memory@main   # pin a commit SHA in production
        with:
          program: "5"
          project-key: ${{ secrets.ANTIBODY_PROJECT_KEY }}   # optional
```

## Inputs

| Input | Default | What |
|---|---|---|
| `version` | `v0.5.0` | The antibody release whose binary runs |
| `repository` | `Himess/antibody` | The repository that publishes it |
| `program` | the mapping's | The program's id on near-antibody |
| `config` | `.antibody/immune.toml` | The mapping file, relative to `working-directory` |
| `project-key` | none | The project's key (its key file's JSON, or the secret in hex), as a repository secret |
| `network` | `testnet` | The NEAR network (the binary has the testnet deployment built in) |
| `working-directory` | `.` | The Foundry project |
| `foundry-version` | `v1.8.5` | The Foundry release that builds the contracts |
| `strict` | `false` | `true`: a paid proof that cannot be read here counts as inconclusive |
| `fail-on-inconclusive` | `true` | `false`: an inconclusive re-run is a warning, not a failure |
| `api` | none | An Antibody API that indexed the reveals, for blobs not committed under the mapping's `blobs` |

Outputs: `exit-code` (0: no re-run paid proof breaks a rule; 2: one still breaks a rule; 3: inconclusive;
1: the check could not run) and `report` (the JSON report's path).

## Security

- **Use `pull_request`, never `pull_request_target`.** `pull_request_target` runs with the base repository's
  secrets on the pull request's code, and `forge build` follows the pull request's `foundry.toml`.
- **The project key is a repository secret.** GitHub does not give secrets to workflows triggered by pull
  requests from forks, so a fork's pull request re-runs disclosed proofs only (and proofs whose content key
  was published); the summary lists every other paid proof as not re-run.
- The key is only in the environment of the step that runs the checksum-verified binary, never in the step
  that builds the pull request's code.
- A proof read with the project key is still sealed. The summary names the rule it breaks (its slot is public
  on chain) but not where: the entry point is what the proof's on-chain tag hides. Running the CLI locally
  with `--reveal-sealed` shows it.
- Sealed proofs are re-run here, outside a TEE, only by a runner that holds the project key. Re-running them
  inside the TEE, where the result is all anyone sees, is next, not built.
