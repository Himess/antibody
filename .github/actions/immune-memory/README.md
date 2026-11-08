# Antibody immune memory (GitHub Action)

A prototype of roadmap item 1 ([docs/immune.md](../../../docs/immune.md)). On every pull request it re-runs
each paid proof of your Antibody program that the run can read against the pull request's compiled code,
with the verifier, locally, at the program's anchor. If one still breaks a committed rule, the pull request
re-opens a paid hole and the check fails.

What it does, in order:
1. downloads the pinned `antibody` release binary for the runner and checks its sha256 against the release's
   `.sha256` file (Linux x86_64, macOS arm64, Windows x86_64);
2. installs Foundry and runs `forge build` (neither with `build: false`: the artifacts then come from another
   job, [Two jobs](#two-jobs));
3. checks the binary's sha256 again, runs `antibody immune` (view calls and local verifier runs: it sends
   nothing) and writes its report to the job summary;
4. fails the job when a paid proof still breaks a rule (and, by default, when the result is inconclusive).

`antibody immune` first shipped in release v0.5.0; the default `version` is v0.5.1, which has the fixes below; the release must be published.

## Use

Commit a mapping, `.antibody/immune.toml` (the format: [docs/immune.md](../../../docs/immune.md)), with the
revealed blobs and the published content keys it names.

**`.antibody/` is also the antibody CLI's private state directory.** Run in your repository, `antibody` keeps
there the content keys and salts of your commits (`submissions/`, `known/`), account keys (`practice/`,
`hunt/`) and witnesses with their salts (`witness-cache/`). Publishing them opens sealed proofs and gives
away accounts. Let only the immune memory's files through, in `.gitignore`:

```gitignore
.antibody/*
!.antibody/immune.toml
!.antibody/proofs/
!.antibody/keys/
```

`.antibody/keys/` holds content keys already published, nothing else: the project key is a repository secret,
never a file in the repository. Then:

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
      - uses: actions/checkout@v4     # with `submodules: recursive` if forge build needs them
      - uses: Himess/antibody/.github/actions/immune-memory@v0.5.1   # or a commit SHA
        with:
          program: "5"
          project-key: ${{ secrets.ANTIBODY_PROJECT_KEY }}   # optional; read Security first
```

## Two jobs

`forge build` runs code from the pull request, and any code a job runs can reach a secret given to that job
([Security](#security)). The safer pattern builds in a job with no secret, then runs the checked binary with
the key in a job that runs none of the pull request's code: that job reads the mapping, the blobs and the
build's artifacts as data. `build` is new in v0.5.1: pin the Action to v0.5.1 or later.

```yaml
name: immune memory
on:
  pull_request:   # never pull_request_target
permissions:
  contents: read
jobs:
  build:          # the pull request's code is built here, with no secret
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: foundry-rs/foundry-toolchain@dba66c27052e63476d886f22216e119ba7e615bf # v1.9.2
        with:
          version: v1.8.5
      - run: forge build
      - uses: actions/upload-artifact@v4
        with:
          name: forge-out
          path: out/
  immune:         # the key is here, and nothing from the pull request runs
    needs: build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4     # the mapping and the blobs, read as data
      - uses: actions/download-artifact@v4
        with:
          name: forge-out
          path: out/
      - uses: Himess/antibody/.github/actions/immune-memory@<commit SHA>   # one with the `build` input
        with:
          program: "5"
          build: "false"
          project-key: ${{ secrets.ANTIBODY_PROJECT_KEY }}
```

## Inputs

| Input | Default | What |
|---|---|---|
| `version` | `v0.5.1` | The antibody release whose binary runs |
| `repository` | `Himess/antibody` | The repository that publishes it |
| `program` | the mapping's | The program's id on near-antibody |
| `config` | `.antibody/immune.toml` | The mapping file, relative to `working-directory` (the `.gitignore` rules above) |
| `project-key` | none | The project's key (its key file's JSON, or the secret in hex), as a repository secret |
| `build` | `true` | `false`: no Foundry and no `forge build`, for a job that takes the artifacts from a job with no secret |
| `network` | `testnet` | The NEAR network (the binary has the testnet deployment built in) |
| `working-directory` | `.` | The Foundry project |
| `foundry-version` | `v1.8.5` | The Foundry release that builds the contracts |
| `strict` | `false` | `true`: a paid proof that cannot be read here counts as inconclusive |
| `fail-on-inconclusive` | `true` | `false`: an inconclusive result is a warning, not a failure |
| `api` | none | An Antibody API that indexed the reveals, for blobs not committed under the mapping's `blobs` |

Outputs: `exit-code` (0: no re-run paid proof breaks a rule; 2: one still breaks a rule; 3: inconclusive;
1: the check could not run, also when the binary no longer matches its checked sha256) and `report` (the JSON
report's path; empty when none was written). The job summary has the report, or why the check could not run.

## Security

- **Use `pull_request`, never `pull_request_target`.** `pull_request_target` runs with the base repository's
  secrets on the pull request's code, and `forge build` follows the pull request's `foundry.toml`.
- **The project key is a repository secret.** GitHub does not give secrets to workflows triggered by pull
  requests from forks, so a fork's pull request re-runs disclosed proofs only (and proofs whose content key
  was published); the summary lists every other paid proof as not re-run.
- **Any code a job runs can reach a secret given to that job, the pull request's `forge build` included.**
  `forge build` follows the pull request's `foundry.toml`, which can name a program to run as the compiler.
  That program runs as the same user on the same runner, before the step that gets the key: it can replace
  the binary, or set variables and paths for the later steps (`$GITHUB_ENV`, `$GITHUB_PATH`). Keeping the key
  out of the build step's environment does not keep it out of the build's reach. The key's step checks the
  binary's sha256 again just before it runs it, which catches a replaced binary, not a runner the build took
  over. [Two jobs](#two-jobs) give the key only to a job that runs none of the pull request's code.
- **Anyone who can push a branch can read a repository secret.** On `pull_request`, the workflow file itself
  comes from the pull request. To keep the key to runs someone approved, make it an environment secret
  behind a required reviewer, and give that `environment:` to the job that holds the key.
- A proof read with the project key is still sealed. The summary names a rule it breaks only when the
  commit's keys on chain already name it (their slots are public); any other rule it breaks now shows as
  `another rule (withheld)`. It never says where: the entry point is what the proof's on-chain tag hides.
  Running the CLI locally with `--reveal-sealed` shows them.
- Sealed proofs are re-run here, outside a TEE, only by a runner that holds the project key. Re-running them
  inside the TEE, where the result is all anyone sees, is next, not built.
