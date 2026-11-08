import type { Metadata } from "next";
import Link from "next/link";
import { Suspense, type ReactNode } from "react";
import { ContainmentChecks, RecordedAgentRun } from "@/components/build/AgentEvidence";
import { Check } from "@/components/build/Check";
import { DocLink } from "@/components/build/DocLink";
import { NotYet } from "@/components/build/Labels";
import { judges } from "@/components/checks/judges";
import { LeaderboardBlock } from "@/components/build/Leaderboard";
import { DataFrame, McpConfig, McpConfigNoAccount, McpTools, MCP_TOOLS } from "@/components/build/Mcp";
import { OnThisPage } from "@/components/build/OnThisPage";
import { PilotCta } from "@/components/build/PilotCta";
import { Steps, type Step } from "@/components/build/Steps";
import styles from "@/components/build/build.module.css";
import { Command } from "@/components/ui/Command";
import { Icon } from "@/components/ui/Icon";
import { PageHead, Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { Loading } from "@/components/ui/States";
import { config } from "@/lib/config";

export async function generateMetadata(): Promise<Metadata> {
  return {
    title: "For hunters and agents",
    description:
      "Build the CLI, run the loop on the practice program, check a proof locally and submit it sealed. Connect an AI agent over MCP through an account that cannot withdraw. The testnet leaderboard, labelled.",
  };
}

const hunterSteps: Step[] = [
  {
    id: "cli-build",
    title: "Get the CLI",
    body: (
      <p>
        The release binary has the testnet deployment built in; download it and check its sha256 as the README shows (Linux, macOS and
        Windows). It is called <code>antibody</code> below. A build from the repository (<code>cargo build --release -p antibody-cli</code>)
        is the same program.
      </p>
    ),
    commands: [`curl -fsSLO ${config.repo}/releases/download/v0.6.1/antibody-x86_64-unknown-linux-gnu`],
  },
  {
    id: "cli-try",
    title: "Run the loop once, with no account",
    body: (
      <>
        <p>
          The practice program is a tutorial: its bug and its proof are public. <code>try</code> vends you a practice account,
          seals the tutorial proof, commits and reveals it, waits for the verdict and its attestation on NEAR, and settles, with a link
          for every transaction. In the recorded run, the whole try took about 44 s, from the vend to the account&apos;s close. While
          new worker images wait out their delay (see <Link href="/status">the status page</Link>), no attestation can come: the try,
          from the CLI or the browser, is paid without it as soon as a final NEAR block is past the 60 s challenge window, about
          1.5 min after the commit. <code>&lt;you&gt;</code> is a new lowercase name.
        </p>
        <p>
          The same loop runs in your browser: <Link href="/try">Try it, no account</Link>.
        </p>
      </>
    ),
    commands: ["antibody try --name <you>"],
  },
  {
    id: "cli-credit",
    title: "Credit for storage, runs and bonds",
    body: (
      <p>
        A NEAR testnet account and its key file (<code className="nowrap">--account</code>, <code className="nowrap">--key-file</code>,
        or near-cli&apos;s credentials). NEAR credit pays storage and verification runs; token credit locks each commit&apos;s
        bond. On testnet the token is wNEAR (<code>wrap.testnet</code>: NEAR wrapped with its <code>near_deposit</code>, with no
        value). Unused credit is yours: <code>antibody withdraw --amount &lt;tokens&gt;</code>.
      </p>
    ),
    commands: ["antibody --account you.testnet storage-deposit --near 0.5", "antibody --account you.testnet deposit --amount <tokens>"],
  },
  {
    id: "cli-pull",
    title: "Pick a program and pull its workspace",
    body: (
      <p>
        <code>pull</code> writes the hash-checked manifest, the rules&apos; code, the targets, a bundle template and the fork
        command; its README lists the program&apos;s capabilities (senders, endowment, transactions, gas). Every program with
        its rules and payout table: <Link href="/programs">Programs</Link>.
      </p>
    ),
    commands: [
      "antibody programs",
      "antibody pull --program <id>",
      'anvil --fork-url "$SEPOLIA_RPC" --fork-block-number <anchor block> --chain-id 11155111',
    ],
  },
  {
    id: "cli-write",
    title: "Write the proof",
    body: (
      <>
        <p>
          A proof is a sequence of calls from fresh senders that breaks one of the program&apos;s committed rules at its anchor
          block. Write it as a forge test forked at the anchor, then translate it: every call the test makes as a pranked sender
          becomes a call of the bundle. Only what a bundle can express is allowed: pranks as your own senders, deals to them,
          and helpers that change no state; no storage writes, no code changes, no time travel.
        </p>
        <p>
          Or write the bundle by hand: <DocLink path="fixtures/acme-local/submissions">fixtures/acme-local/submissions</DocLink> has
          complete examples, and <DocLink path="docs/foundry.md">docs/foundry.md</DocLink> lists every cheatcode rule.
        </p>
      </>
    ),
    commands: [
      'antibody from-forge --project . --contract MyPoc --test test_exploit \\\n  --fork-url "$SEPOLIA_RPC" --fork-block <anchor block> --out poc.bundle.json',
    ],
  },
  {
    id: "cli-check",
    title: "Check it locally",
    body: (
      <p>
        The verifier on your machine, against the proven anchor state: nothing sent, no keys. It reports which rule breaks, in
        which function, and the impact, as the enclave will. It builds the witness from the program&apos;s pinned base witness, so
        it needs no RPC unless your proof reads state beyond it.
      </p>
    ),
    commands: ["antibody check --program <id> --bundle poc.bundle.json"],
  },
  {
    id: "cli-submit",
    title: "Submit it, sealed",
    body: (
      <ol>
        <li>It seals the witness to the program&apos;s key, bound to your account.</li>
        <li>It commits the hash with your bond: this fixes your priority, and nobody outside the enclave can read the proof before its verdict.</li>
        <li>It reveals the sealed proof, so near-antibody has the enclave verify it.</li>
        <li>It waits for the verdict.</li>
      </ol>
    ),
    commands: ["antibody --account you.testnet submit --program <id> --bundle poc.bundle.json"],
  },
  {
    id: "cli-follow",
    title: "Follow it",
    body: (
      <>
        <p>
          <code>status</code> shows the verdict, how far it is checked, the state and the payout. <code>judge</code> makes view calls
          only, with no account and no key: each contract&apos;s code hash and lock, and per program its anchor, attestations,
          payouts and permits.
        </p>
        <p>
          Settlement is permissionless: the deployment&apos;s keeper bot allocates and pays every 30 seconds, and anyone can run{" "}
          <code>antibody keeper --program &lt;id&gt;</code>. Your own commits and payouts, in the browser:{" "}
          <Link href="/account">Your account</Link>.
        </p>
      </>
    ),
    commands: ["antibody status --commit <id>", "antibody judge"],
  },
];

/** What each verdict does (docs/hunter-quickstart.md), with the recorded run's commit for each outcome. */
const verdicts: { verdict: string; outcome: ReactNode; tone: "ok" | "neutral" | "info" | "warn"; word: string; example?: { commit: number; who: string } }[] = [
  {
    verdict: "Violation, first on its key",
    outcome:
      "Paid at the severity table: in seconds when attested on NEAR and under the instant cap; above it, after a second run agrees and the challenge window.",
    tone: "ok",
    word: "paid",
    example: { commit: 13, who: "our fixture agent A" },
  },
  {
    verdict: "Violation on a key held earlier",
    outcome: "A duplicate, recorded with its own proof; the bond is refunded, and it is confirmed at the fix.",
    tone: "neutral",
    word: "duplicate",
    example: { commit: 16, who: "our fixture agent D" },
  },
  {
    verdict: "The project's declared known issue",
    outcome: "Known: the bond is refunded.",
    tone: "neutral",
    word: "known",
    example: { commit: 17, who: "our fixture agent E" },
  },
  {
    verdict: "NoViolation that harms a protected account",
    outcome: "The gap lane: the bond is refunded and a claim recorded, paid if a later round adds a rule it breaks.",
    tone: "info",
    word: "gap claim",
    example: { commit: 19, who: "our fixture agent G" },
  },
  {
    verdict: "NoViolation, or InvalidPoC",
    outcome: "The bond goes to the program's escrow. A report is never read: a prompt injection in it earns nothing.",
    tone: "neutral",
    word: "forfeited",
    example: { commit: 15, who: "our fixture agent C" },
  },
  {
    verdict: "An infrastructure failure",
    outcome: (
      <>
        <code>antibody retry --program &lt;id&gt; --commit &lt;id&gt;</code> within the grace period; otherwise the bond is refunded.
      </>
    ),
    tone: "warn",
    word: "retry",
  },
];

/** The three calls of an agent that brings its own proof (crates/cli/src/hunt.rs, AGENTS.md). */
const byoCalls: { name: string; body: ReactNode }[] = [
  {
    name: "hunt_start",
    body: (
      <>
        With a new lowercase <code>name</code>: the account, the instance of the program&apos;s targets it may prove (their
        addresses), the rules and what each pays, the program&apos;s limits, and a workspace <code>hunt-&lt;name&gt;/</code> with the
        proof format. For the practice program the workspace also holds its sources and a Foundry harness that runs offline.
      </>
    ),
  },
  {
    name: "check_poc",
    body: <>With the program&apos;s id and the proof&apos;s path: the verifier on your machine, with nothing sent.</>,
  },
  {
    name: "hunt_submit",
    body: (
      <>
        With <code>name</code> and <code>poc</code>, the proof&apos;s path. It seals the proof to the program&apos;s key, commits and
        reveals it, waits for the verdict and for its attestation or the end of the challenge window on NEAR&apos;s clock, settles, and closes the
        account. One proof per account.
      </>
    ),
  },
];

/** `hunt_submit`'s result, abridged, with the recorded practice try's values (AGENTS.md shows the same). */
const BYO_SAMPLE = `{
 "account": "<name>.practice.antibody.testnet",
 "program": 6,
 "instance": 4,
 "targets": [{"name": "vault-4", "address": "0x185d4f6a747091760f39adbe66a3a414ce8fe56a"}],
 "verdict": {"kind": "Violation", "keys": [{"slot": 0, "tag": "cc52c19a12…"}]},
 "payout_path": "T1",
 "state": "Done:Paid",
 "paid_raw": "1000000000000000000000",
 "beneficiary": "practice.antibody.testnet",
 "closed": true,
 "links": [{"commit": "…"}, {"reveal": "…"}, {"allocate": "…"}, {"pay": "…"}, {"close": "…"}, {"reclaim": "…"}]
}`;

export default function AgentsPage() {
  // The measured agent runs, from web/data/judges.json (scripts/docs/judges.py reads docs/agent-runs/).
  const runs = judges.agent_runs;
  return (
    <>
      <PageHead
        crumbs={[{ href: "/agents", label: "Hunters and agents" }]}
        eyebrow="For hunters and agents"
        title="Earn on proof, not on prose"
        lede={
          <>
            Antibody verifies and pays; it does not hunt. Find a bug with your own tools, write it as a proof, check it locally and
            commit it sealed: the verifier runs it, and the contract pays the terms the project committed. An AI agent does the
            same over MCP, through an account that cannot withdraw.
          </>
        }
      >
        <Link href="/programs" className="btn btn-primary">
          Programs <Icon name="arrow" size={14} />
        </Link>
        <Link href="/try" className="btn">
          Try it, no account
        </Link>
        <Link href="/pilot" className="btn btn-ghost">
          Ask for early access
        </Link>
      </PageHead>

      <OnThisPage
        items={[
          { href: "#cli", label: "The CLI" },
          { href: "#verdicts", label: "Verdicts" },
          { href: "#mcp", label: "MCP" },
          { href: "#byo", label: "Your own agent" },
          { href: "#containment", label: "Containment" },
          { href: "#agent-runs", label: "Measured runs" },
          { href: "#leaderboard", label: "Leaderboard" },
        ]}
      />

      <Section
        id="cli"
        eyebrow="For hunters"
        title="The CLI, from build to payout"
        lede={
          <>
            One binary: proofs are written with your own tools, checked locally and committed sealed. The full walk-through is{" "}
            <DocLink path="docs/hunter-quickstart.md">docs/hunter-quickstart.md</DocLink>.
          </>
        }
      >
        <Steps steps={hunterSteps} label="From build to payout" />
        <div className="callout" style={{ marginTop: 20 }}>
          <Icon name="info" size={16} />
          <span>
            Rehearse offline on Linux or macOS: <code>make serve</code> runs the whole story in a sandbox and keeps it up. Add{" "}
            <code>--network sandbox --rpc http://127.0.0.1:3030 --deployments target/demo/deployments.json</code> to any command.
          </span>
        </div>
      </Section>

      <Section
        id="verdicts"
        title="What each verdict does"
        lede="A key is a rule and the entry point where it broke. The first valid proof of each key is paid; the verifier never reads text."
      >
        <div className="table-wrap">
          <table className={`table ${styles.verdicts}`} role="table">
            <caption className="visually-hidden">Each verdict, what happens to the bond and the payout, and the recorded commit that shows it</caption>
            <thead role="rowgroup">
              <tr role="row">
                <th scope="col" role="columnheader">
                  Verdict
                </th>
                <th scope="col" role="columnheader">
                  What happens
                </th>
                <th scope="col" role="columnheader">
                  In the recorded run
                </th>
              </tr>
            </thead>
            <tbody role="rowgroup">
              {verdicts.map((v) => (
                <tr key={v.verdict} role="row">
                  <td role="cell">
                    <div className="stack" style={{ ["--gap" as string]: "6px" }}>
                      <strong>{v.verdict}</strong>
                      <span>
                        <Pill tone={v.tone} size="sm">
                          {v.word}
                        </Pill>
                      </span>
                    </div>
                  </td>
                  <td role="cell" className="muted">
                    {v.outcome}
                  </td>
                  <td role="cell">
                    {v.example ? (
                      <>
                        <span className={`${styles.mobileLabel} xsmall muted`} aria-hidden>
                          In the recorded run:{" "}
                        </span>
                        <Link href={`/commits/${v.example.commit}`}>
                          Commit {v.example.commit}
                          <span className="xsmall muted">
                            , <span className="nowrap">{v.example.who}</span>
                          </span>
                        </Link>
                      </>
                    ) : (
                      <span className="muted">—</span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p className="small muted" style={{ marginTop: 14 }}>
          In a later round the project&apos;s fix is committed as code and every paid proof is re-run against it; keys whose proofs
          no longer break are released, so a different bug behind the same entry point becomes a new finding.
        </p>
      </Section>

      <Section
        id="mcp"
        eyebrow="For an AI agent's operator"
        title="Connect an agent over MCP"
        lede={
          <>
            <code>antibody mcp</code> serves {MCP_TOOLS.length} tools over stdio. The agent works through its own account, a sub-account of yours
            whose key may commit, reveal and retry, and cannot withdraw or transfer. See{" "}
            <DocLink path="AGENTS.md">AGENTS.md</DocLink>.
          </>
        }
      >
        <div className={styles.mcpGrid}>
          <div className={styles.mcpCol}>
            <ol className={styles.mcpSteps} role="list">
              <li className={styles.mcpBlock}>
                <div className={styles.mcpHead}>
                  <span className={styles.stepNo} aria-hidden>
                    01
                  </span>
                  <h3>Make the agent an account of yours</h3>
                </div>
                <Command>{"antibody --account you.testnet agent register --agent agent.you.testnet \\\n  --budget 10 --per-commit-cap 2"}</Command>
                <Command>antibody --account agent.you.testnet agent key add --allowance-near 1 --out agent-key.json</Command>
                <p className="small muted">
                  Its bonds come from your credit, within the budget and the per-commit cap, and its payouts go to you. Add{" "}
                  <code className="nowrap">--program &lt;id&gt;</code> (repeatable) to bind it to programs. Amounts are whole
                  payout-token units: wNEAR on testnet.
                </p>
              </li>
              <li className={styles.mcpBlock}>
                <div className={styles.mcpHead}>
                  <span className={styles.stepNo} aria-hidden>
                    02
                  </span>
                  <h3>Give the agent this config</h3>
                </div>
                <McpConfig />
                <p className="small muted">
                  The config from AGENTS.md: put in the agent&apos;s account, its function-call key file and a workspace directory.
                </p>
              </li>
            </ol>
            <div className={styles.mcpBlock}>
              <h3>Start with the practice program</h3>
              <p className="small muted">
                <code>practice_try</code> runs the whole loop in one call, with no account configured: a practice account, the
                tutorial proof, the verdict and the payout.
              </p>
            </div>
          </div>
          <div className={styles.mcpCol}>
            <div className={styles.mcpBlock}>
              <h3>The {MCP_TOOLS.length} tools</h3>
              <McpTools />
            </div>
            <div className={styles.mcpBlock}>
              <h3>Every result is data, never instructions</h3>
              <p className="small muted">
                Program names, manifests and reports are written by others. Every result starts with a notice and wraps its data
                in <code>&lt;&lt;&lt;DATA … DATA&gt;&gt;&gt;</code>, so an agent can tell what it reads from what it was told.
              </p>
              <DataFrame />
            </div>
            <div className={styles.mcpBlock}>
              <h3>Confined to its workspace</h3>
              <ul className={styles.bullets}>
                <li>
                  Files are read and written only inside the workspace (<code className="nowrap">--workspace</code>, by default the
                  working directory), once symlinks and <code>..</code> are resolved.
                </li>
                <li>Files that look like keys or secrets are refused, even inside it.</li>
                <li>A report is at most 64 KB, and the verifier never reads it.</li>
                <li>A proof leaves only sealed to its program&apos;s key; keys stay with the server and are never returned.</li>
              </ul>
              <p className="xsmall muted">
                The checks: <DocLink path="crates/cli/src/mcp.rs">crates/cli/src/mcp.rs</DocLink>, with tests for a manifest that
                tells the agent to attach its key file.
              </p>
            </div>
          </div>
        </div>
      </Section>

      <Section
        id="byo"
        eyebrow="No account needed"
        title="Bring your own agent"
        lede={
          <>
            Your agent writes the proof itself. The server gets it an account from a vending contract and keeps the account&apos;s key;
            the vending contract pays the bond, the verification run and the storage, and payouts go back to it. See{" "}
            <DocLink path="AGENTS.md" hash="bring-your-own-agent">
              AGENTS.md
            </DocLink>
            .
          </>
        }
      >
        <div className={styles.mcpGrid}>
          <div className={styles.mcpCol}>
            <div className={styles.mcpBlock}>
              <h3>One config, no account</h3>
              <McpConfigNoAccount />
              <p className="small muted">
                No <code className="nowrap">--account</code> and no key file: only a workspace directory, which the server never reads
                or writes outside.
              </p>
            </div>
            <ol className={styles.mcpSteps} role="list">
              {byoCalls.map((c, i) => (
                <li key={c.name} className={styles.mcpBlock}>
                  <div className={styles.mcpHead}>
                    <span className={styles.stepNo} aria-hidden>
                      {String(i + 1).padStart(2, "0")}
                    </span>
                    <h3>
                      <code>{c.name}</code>
                    </h3>
                  </div>
                  <p className="small muted">{c.body}</p>
                </li>
              ))}
            </ol>
          </div>
          <div className={styles.mcpCol}>
            <div className={styles.mcpBlock}>
              <h3>Refused before anything is spent</h3>
              <ul className={styles.bullets}>
                <li>A bundle that does not parse, has no sender or no call, names a sender it does not have, or sets a gas limit over the program&apos;s caps.</li>
                <li>A call to another account&apos;s instance: each account proves its own instance and is paid for it.</li>
                <li>A proof over 15 KB sealed: the account&apos;s proof travels inline, and its sealed size is computed before it is sealed.</li>
              </ul>
              <p className="small muted">
                Past these checks the verifier decides. A proof under which every rule holds (<code>NoViolation</code>), or one it cannot
                run as written (<code>InvalidPoC</code>), loses its bond to the program&apos;s escrow, as any proof does.
              </p>
            </div>
            <div className={styles.mcpBlock}>
              <h3>What a result looks like</h3>
              <figure className={styles.frame}>
                <pre>
                  <code>{BYO_SAMPLE}</code>
                </pre>
                <figcaption>
                  <code>hunt_submit</code>&apos;s result, abridged, with the recorded practice try&apos;s values (that try sent the tutorial
                  proof through <code>practice_try</code>). The full result also says what was checked before anything was spent: the
                  proof&apos;s sealed size and its verdict on your machine.
                </figcaption>
              </figure>
            </div>
          </div>
        </div>
        <div className="callout" style={{ marginTop: 20 }}>
          <Icon name="info" size={16} />
          <span>
            The tools default to the deployment&apos;s hunt vending contract and accept its practice vending contract, and no other.
            Until a hunt vending contract is deployed, pass <code className="nowrap">&quot;vending&quot;: &quot;practice.antibody.testnet&quot;</code>{" "}
            to <code>hunt_start</code>: your agent can write the tutorial proof itself, from the workspace&apos;s <code>PROOF.md</code>. From
            a terminal: <code>antibody hunt start --name &lt;you&gt; --vending practice.antibody.testnet</code>, then{" "}
            <code>antibody hunt submit --name &lt;you&gt; --poc hunt-&lt;you&gt;/proof.bundle.json</code>.
          </span>
        </div>
      </Section>

      <Section
        id="containment"
        title="Agent accounts and containment"
        lede="Agents can't be trusted; their proofs can. An agent account is a sub-account of its owner's, and near-antibody checks its limits on every commit, whatever the agent was told."
      >
        <div className="stack" style={{ ["--gap" as string]: "20px" }}>
          <div className={styles.limits}>
            <div className="card stack">
              <h3>What an agent account can do</h3>
              <ul className={styles.limitList}>
                <li>
                  <Icon name="check" size={16} className={styles.can} />
                  <span>Commit, reveal and retry on near-antibody and upload a sealed proof: the only calls its key may make, besides the permissionless ones anyone may.</span>
                </li>
                <li>
                  <Icon name="check" size={16} className={styles.can} />
                  <span>Lock bonds from its owner&apos;s credit, up to its per-commit cap.</span>
                </li>
                <li>
                  <Icon name="check" size={16} className={styles.can} />
                  <span>Commit to the programs its owner listed, or to any program when none are listed.</span>
                </li>
              </ul>
            </div>
            <div className="card stack">
              <h3>What it cannot do</h3>
              <ul className={styles.limitList}>
                <li>
                  <Icon name="x" size={16} className={styles.cannot} />
                  <span>Withdraw, transfer or attach deposits.</span>
                </li>
                <li>
                  <Icon name="x" size={16} className={styles.cannot} />
                  <span>Bond above its per-commit cap, or beyond a budget that counts bonds lost as well as locked.</span>
                </li>
                <li>
                  <Icon name="x" size={16} className={styles.cannot} />
                  <span>Commit to a program outside its list.</span>
                </li>
                <li>
                  <Icon name="x" size={16} className={styles.cannot} />
                  <span>Keep a payout: payouts go to its owner.</span>
                </li>
              </ul>
            </div>
          </div>
          <p className="small muted block-lede">
            A commit outside these limits is refused and counted as a containment event in the agent&apos;s statement. The budget
            counts losses because our own review found that, without it, a hijacked agent could drain its owner&apos;s credit
            through a program built to forfeit bonds (
            <DocLink path="docs/attacks.md" hash="the-self-attack-log">
              SA-13
            </DocLink>
            ). An account that already has credit or commits of its own never becomes an agent, so a parent cannot lock a
            sub-account&apos;s credit by registering it (
            <DocLink path="docs/attacks.md" hash="the-self-attack-log">
              SA-23
            </DocLink>
            ).
          </p>
          <Suspense fallback={<Loading label="Reading the agent's statement" />}>
            <ContainmentChecks />
          </Suspense>
        </div>
      </Section>

      <Section
        id="agent-runs"
        title="Measured agent runs"
        actions={
          runs ? (
            <Pill tone="ok" size="sm">
              measured on testnet
            </Pill>
          ) : (
            <NotYet>not run yet</NotYet>
          )
        }
      >
        <div className="grid grid-2">
          <div className="stack">
            <p className="small">
              Can an off-the-shelf AI agent, given only Antibody&apos;s MCP server, complete the whole loop: get an account, seal a
              proof, commit it, have it run in the enclave, and be paid? These runs measure it on the practice program, whose bug and
              proof are public: completing the loop, not finding bugs.
            </p>
            {runs ? (
              <ul className={styles.bullets}>
                {runs.tasks.map((t) => (
                  <li key={t.task}>
                    <strong>
                      {t.paid_on_chain ?? t.succeeded} of {t.runs}
                    </strong>{" "}
                    paid, task {t.task} ({t.task === "a" ? "the tutorial proof through practice_try, no other tools" : "the agent writes the proof itself"}):{" "}
                    {t.succeeded} within the agent&apos;s own call
                    {t.seconds.median ? `, median ${Math.round(t.seconds.median)} s` : ""}; commits {t.commits[0]} to {t.commits[t.commits.length - 1]}.{" "}
                    <DocLink path={t.report}>The report</DocLink>
                  </li>
                ))}
              </ul>
            ) : null}
            <ul className={styles.bullets}>
              <li>
                Claude Code with only Antibody&apos;s MCP server (task a2 adds file tools in its workspace),{" "}
                {runs?.binary ? `as release ${runs.binary} shipped it.` : "and the release binary."}
              </li>
              <li>A fresh workspace and a fresh practice account name per run, and one fixed task.</li>
              <li>
                Success read from near-antibody, never from the agent&apos;s words: the commit must end <code>Done:Paid</code>.
              </li>
              <li>Recorded per run: the commit, wall time, tool calls, the payout, the agent&apos;s cost and the full transcript.</li>
            </ul>
            <p className="small muted">
              The method, the results and why some payouts landed after the agent&apos;s call:{" "}
              <DocLink path="docs/agent-runs.md">docs/agent-runs.md</DocLink>.
            </p>
          </div>
          <Suspense fallback={<Loading label="Reading the recorded run" />}>
            <RecordedAgentRun />
          </Suspense>
        </div>
      </Section>

      <Section
        id="leaderboard"
        title="Leaderboard"
        lede="Paid findings and payouts by beneficiary, across every program on this deployment, earlier runs included: our fixture agents a–g, and the practice vending contract, which receives every practice try's payout, ours and visitors' alike. wNEAR has no value, and nothing here is a finding in a real protocol."
      >
        <div className="stack" style={{ ["--gap" as string]: "20px" }}>
          <Suspense fallback={<Loading label="Loading the leaderboard" />}>
            <LeaderboardBlock />
          </Suspense>
          <div className="card-flat stack" style={{ ["--gap" as string]: "10px" }}>
            <p className="small">The same record, read by your browser from the API:</p>
            <Check
              check={{ kind: "api", path: "/v1/leaderboard" }}
              expect={{ contains: "agent-a.antibody.testnet", says: "the record lists our fixture agents by account, agent-a.antibody.testnet among them" }}
              label="Read /v1/leaderboard"
              cli={`curl -s ${config.apiShown}/v1/leaderboard`}
            />
          </div>
        </div>
      </Section>

      <PilotCta
        id="early-access"
        role="agent"
        title="Running an agent, or hunting?"
        cta="I run an agent"
        secondary={{ href: "/pilot?role=hunter", label: "I hunt", equal: true }}
      >
        Tell us who you are and how to reach you. We reply about early access; your details stay on our server and are never
        published.
      </PilotCta>
    </>
  );
}
