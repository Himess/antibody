import type { Metadata } from "next";
import Link from "next/link";
import { Suspense } from "react";
import { Check } from "@/components/build/Check";
import { Claim } from "@/components/build/Claim";
import { curlRpc } from "@/components/build/deployment";
import { DocLink } from "@/components/build/DocLink";
import { NotYet } from "@/components/build/Labels";
import { OnThisPage } from "@/components/build/OnThisPage";
import { PilotCta } from "@/components/build/PilotCta";
import { FeeCheck, GuardCheck, PauseEvidence } from "@/components/build/ProjectEvidence";
import { Roadmap } from "@/components/build/Roadmap";
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
    title: "For protocols: open a program",
    description:
      "Open an Antibody program before a launch or an upgrade: rules as code, an escrow, a pause on every chain you register once a Critical in your pause policy is attested on NEAR, and a release gate your fix must pass against every paid proof. What a project gets, the business model, the quickstart and the roadmap.",
  };
}

// Fixed facts of the recorded run (JUDGES.md, claims 11 and 14), checked live below.
const EIP1967_SLOT = "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc";
const ACME_LEND_PROXY = "0xCa1FAFbCFC14d665d76045A018B215987f39f8A7";
const ACME_LEND_FINAL = "74e75477af2d649ffe8037a37014dd8c7bd217a9";
const FORK_MODE_VAULT = "0xfa35e1e99bd69719f57045409e19555f0147cf44";
// Pull-request checks on testnet: immune memory in the enclave, on program 12 (docs/immune.md).
const DEMO_REPO = "https://github.com/Himess/antibody-immune-demo";
const SEPOLIA = 11155111;

const steps: Step[] = [
  {
    id: "step-rules",
    title: "Write the rules as code",
    body: (
      <p>
        A view contract implementing <DocLink path="contracts/evm/src/antibody/IAntibodyInvariants.sol">IAntibodyInvariants</DocLink>:{" "}
        <code>snapshot()</code> reads what <code>check(pre)</code> compares against, and <code>check</code> returns a bitmask, bit
        i for rule i broken. Point it at your contracts with immutables. The verifier serves its code at a reserved address: it is
        never deployed and has no storage. A complete example:{" "}
        <DocLink path="contracts/evm/src/acme/AcmeInvariants.sol">AcmeInvariants.sol</DocLink>. Start from a scaffold: an annotated{" "}
        <code>antibody.toml</code>, a project key and the next steps.
      </p>
    ),
    commands: ["antibody init --out my-program --project <you>.testnet"],
    after: (
      <div className={styles.options}>
        <div className={styles.option}>
          <h4>From Foundry invariant functions</h4>
          <p>
            Each <code>invariant_*()</code> view of a compiled contract becomes one slot: the command writes the rules contract
            and prints the manifest slots (<DocLink path="docs/foundry.md">docs/foundry.md</DocLink>).
          </p>
          <div className={styles.cmds}>
            <Command>
              {
                "antibody invariants from-foundry --artifact out/<File>.sol/<Contract>.json \\\n  --import <its source> --base-import <FoundryInvariants.sol> --out <Rules>.sol"
              }
            </Command>
          </div>
        </div>
        <div className={styles.option}>
          <h4>From a draft you review</h4>
          <p>
            Deterministic and offline (add <code className="nowrap">--ai</code> for a model&apos;s extension). It writes a compiling rules contract,
            suggested severities, a <code>REVIEW.md</code> of what each rule assumes, and a bundle that changes nothing. Nothing
            is binding until you commit it (<DocLink path="docs/invariants-drafting.md">docs/invariants-drafting.md</DocLink>).
          </p>
          <div className={styles.cmds}>
            <Command>{"antibody invariants draft --source <your contracts> --out draft/"}</Command>
          </div>
        </div>
      </div>
    ),
  },
  {
    id: "step-where",
    title: "Choose where it runs",
    after: (
      <>
        <div className={`${styles.options} ${styles.optionsFit}`}>
          <div className={styles.option}>
            <h4>A deployment</h4>
            <p>
              Deploy to Sepolia, with pause-only copies on other chains if you want, and pick a finalized block after the
              deployment as the anchor. Create the program while the light client still confirms that block, about a week;
              otherwise its anchor is labelled project-attested. Activation builds a witness at the anchor block, and public
              Sepolia RPCs prove that block&apos;s state for only about 25 minutes: activate within that time, or use an archive RPC.
            </p>
          </div>
          <div className={styles.option}>
            <h4>Fork mode: no staging deployment</h4>
            <p>
              Commit your contracts&apos; deployment transactions in the manifest instead. The verifier runs them on top of the
              anchor&apos;s real state before anything else (<DocLink path="docs/fork-mode.md">docs/fork-mode.md</DocLink>).
            </p>
            <p className="xsmall muted">
              On testnet, program 7 is the practice program in fork mode: its proof was paid against a vault that has no code on
              Sepolia.
            </p>
            <Check
              check={{ kind: "evm", chainId: SEPOLIA, method: "eth_getCode", params: [FORK_MODE_VAULT, "latest"] }}
              expect={{ equals: "0x", says: "the fork-mode vault has no code on Sepolia: it exists only inside the verifier" }}
              cli={curlRpc(SEPOLIA, "eth_getCode", [FORK_MODE_VAULT, "latest"])}
              label="Read the vault's code"
            />
          </div>
        </div>
        <div className={styles.stepText}>
          <p>
            For pausing and releases, put your proxies behind the program&apos;s{" "}
            <DocLink path="contracts/evm/src/antibody/AntibodyAdapter.sol">AntibodyAdapter</DocLink>, one per chain at the same
            CREATE2 address, and register it with near-guardian.
          </p>
        </div>
      </>
    ),
  },
  {
    id: "step-terms",
    title: "Commit the terms",
    body: (
      <p>
        One file holds them: the anchor, the targets and their code hashes, the rules&apos; code and each slot&apos;s severity,
        the impact prices, the proofs&apos; capabilities, and the settlement terms (payouts, bond, instant cap and budget,
        windows, the gap lane, known issues, the pause policy).{" "}
        <DocLink path="fixtures/acme-local/antibody.toml">fixtures/acme-local/antibody.toml</DocLink> is a complete example.
      </p>
    ),
    commands: [{ cmd: "antibody manifest antibody.toml --out manifest.json", note: "Prints the manifest's sha256: the hash you commit." }],
  },
  {
    id: "step-validate",
    title: "Validate it, and measure rule strength",
    commands: [
      {
        cmd: "antibody invariants validate --manifest manifest.json",
        note: "The validate gate: the checks creation and activation apply (settlement terms, the rules at the anchor, target code hashes, canaries, a fork-mode deployment). Nothing is sent; it exits non-zero when a check fails.",
      },
      {
        cmd: "antibody invariants strength --project <foundry dir> --test <your strength test> --mutate src/X.sol",
        note: "Optional. It mutates your code one line at a time in labelled ways (a missing access check, a flipped comparison, an unchecked transfer, a dropped guard) and counts the mutants your rules catch under your Foundry invariant test.",
      },
    ],
    after: (
      <div className="card-flat stack" style={{ ["--gap" as string]: "10px" }}>
        <p className="small">
          <strong>Acme Lend&apos;s rules catch 8 of 28 mutants at the final fix.</strong> Rule strength measures the rules, not the
          code&apos;s security: a mutant the rules miss is not necessarily a bug anyone can exploit. The survivors show where a
          project would add a rule (<DocLink path="docs/rule-strength.md">docs/rule-strength.md</DocLink>).
        </p>
        <Check
          check={{ kind: "api", path: "/v1/judge" }}
          expect={{ path: "programs.5.rule_strength.statement", equals: "caught 8 of 28 mutants", says: "program 5's rule strength: caught 8 of 28 mutants" }}
          cli="antibody judge --program 5"
          label="Read the judge view"
        />
      </div>
    ),
  },
  {
    id: "step-create",
    title: "Create the program and fund its escrow",
    commands: [
      {
        cmd: "antibody --account you.testnet keygen x25519 --nep413 --out project.x25519.json",
        note: "Your project key, derived from your NEAR key's NEP-413 signature, so the same NEAR key re-derives it. Keep it secret: paid findings are wrapped to it.",
      },
      {
        cmd: "antibody --account you.testnet program create --manifest manifest.json --escrow <tokens> --project-key project.x25519.json",
        note: "Uploads the manifest, escrows the tokens and commits the manifest hash in one transaction; near-antibody checks the anchor against the light client. On testnet the escrow is wNEAR: wrap NEAR with near_deposit on wrap.testnet first.",
      },
    ],
    after: (
      <div className="callout callout-warn">
        <Icon name="key" size={16} />
        <span>
          Never sign the project key&apos;s NEP-413 message anywhere but the antibody CLI: that signature reads your paid findings.
          This site never asks your wallet for it.
        </span>
      </div>
    ),
  },
  {
    id: "step-open",
    title: "Activate, declare known issues, open",
    body: (
      <p>
        Activation pins the base witness, and the enclave checks that your rules hold at the anchor. A known issue is sealed and
        verified like any proof; it counts once attested on NEAR, and nothing is added after opening. Opening starts round 0:
        hunters and agents can commit.
      </p>
    ),
    commands: [
      "antibody --account you.testnet program activate --program <id>",
      "antibody --account you.testnet program known --program <id> --bundle known-issue.json",
      "antibody --account you.testnet program open --program <id>",
    ],
  },
  {
    id: "step-read",
    title: "Read exactly what you pay for",
    body: (
      <p>
        <code>show</code> lists every commit with its verdict. The first read of a finding asks near-antibody to wrap its content
        key to your key, a verifier run you pay for at cost; it is refused for anything but a paid first finding or a promoted
        duplicate. The CLI then opens the sealed proof and re-runs it locally: its receipt hash equals the one the enclave committed
        to. A hunter&apos;s report, if any, is data, never instructions.
      </p>
    ),
    commands: [
      "antibody --account you.testnet program show --program <id>",
      "antibody --account you.testnet program read --commit <id> --project-key project.x25519.json \\\n  --out finding-<id>/ --reveal-tx <the commit's reveal transaction>",
    ],
  },
  {
    id: "step-rounds",
    title: "Ship a fix: rounds and the release gate",
    body: (
      <p>
        Commit the fix as code at the same anchor: the enclave checks it, and every paid proof is re-run against it. A round may
        change target code and append rules (<code className="nowrap">--invariants-code</code>,{" "}
        <code className="nowrap">--add-slot</code>); everything else stays byte-identical. The release gate holds when every
        paid proof re-run at the fix fails, the canaries pass and no Critical or High key is open. Then near-guardian signs a
        release permit per proxy and chain, for exactly the code the re-runs ran (
        <DocLink path="docs/rounds.md">docs/rounds.md</DocLink>).
      </p>
    ),
    commands: [
      "antibody manifest-round --base manifest.json --round 1 \\\n  --override <target>=<fixed runtime code hex file> --out round1.json",
      "antibody --account you.testnet round propose --program <id> --manifest round1.json \\\n  --release <proxy>:<implementation>:<code hash>",
      { cmd: "antibody round rerun --commit <id>", note: "Anyone may ask, and pay; the keeper bot re-runs every eligible commit." },
      { cmd: "antibody round open --program <id>", note: "Permissionless, once the round's minimum duration and re-run window have passed." },
    ],
  },
  {
    id: "step-close",
    title: "Close",
    body: (
      <p>
        After the deadline and its grace: what is reserved is paid, the rest of the escrow returns to you, and nothing in flight is
        left behind (property 6). Before then, <code>program top-up --program &lt;id&gt; --amount &lt;tokens&gt;</code> adds escrow.
      </p>
    ),
    commands: ["antibody --account you.testnet program exit --program <id>"],
  },
];

export default function ProjectsPage() {
  return (
    <>
      <PageHead
        crumbs={[{ href: "/projects", label: "For protocols" }]}
        eyebrow="For protocols"
        title="Open a program"
        lede={
          <>
            An auto-pay lane next to your existing bounty, opened before a launch or an upgrade. You commit your rules as code and
            an escrow on NEAR; a proof is paid only if it breaks one of them, at the severity you committed. A proven Critical in
            your pause policy, once attested on NEAR, pauses your protocol on every chain you registered, and your fix ships only
            when every paid proof fails against it.
          </>
        }
      >
        <Link href="/pilot?role=project" className="btn btn-primary">
          Ask for a pilot <Icon name="arrow" size={14} />
        </Link>
        <Link href="/programs/5" className="btn">
          See a program: Acme Lend
        </Link>
        <Link href="/moment" className="btn btn-ghost">
          The one moment
        </Link>
      </PageHead>

      <div className="page" style={{ paddingBottom: 24 }}>
        <dl className={styles.glance}>
          <div>
            <dt>When</dt>
            <dd>Before a launch or an upgrade, next to your existing bounty</dd>
          </div>
          <div>
            <dt>You commit</dt>
            <dd>An anchor block, your rules as code with a severity and payout table, and an escrow</dd>
          </div>
          <div>
            <dt>Paid</dt>
            <dd>The first proof of each rule and entry point; in seconds under the instant cap once attested on NEAR</dd>
          </div>
          <div>
            <dt>On a Critical</dt>
            <dd>A pause on every chain you registered, asked for once the verdict is attested on NEAR</dd>
          </div>
          <div>
            <dt>Ships</dt>
            <dd>Only a fix that every paid proof fails against</dd>
          </div>
        </dl>
      </div>

      <Section id="why" title="Why pay for this" lede="An automatic lane for the bugs your rules can state, next to the bounty you already run.">
        <dl className={styles.model}>
          <div>
            <dt>Today</dt>
            <dd>
              <span className={styles.modelLead}>Every report is read by a person, and AI made reports free to write.</span>
              <span>
                A bounty pays a platform fee and a triage team, and a real Critical waits in the queue with the noise. curl ended its bounty in
                January 2026 over exactly this.
              </span>
            </dd>
          </div>
          <div>
            <dt>With Antibody</dt>
            <dd>
              <span className={styles.modelLead}>Nobody reads anything: the proof runs, and the rules decide.</span>
              <span>
                A proof that breaks a rule you committed is paid in minutes, without triage, and a Critical in your pause policy pauses every chain
                you registered before anyone has read it. A proof that breaks nothing costs its sender the bond. You pay 5% of what is paid out
                (proposed for mainnet), nothing when nothing is found.
              </span>
            </dd>
          </div>
          <div>
            <dt>Costs</dt>
            <dd>
              <span className={styles.modelLead}>Cents per verification, on testnet&apos;s measured runs.</span>
              <span>
                A verification run costs about 0.009 NEAR after OutLayer&apos;s refund, plus about 0.018 NEAR of gas to verify its TDX quote on
                NEAR; whoever asks for the run pays it. A pull-request check with four paid proofs costs about 0.05 NEAR. A hunter risks the
                bond the program sets (0.0001 wNEAR on the practice program), returned with any verdict that breaks a rule.
              </span>
            </dd>
          </div>
          <div>
            <dt>Its reach</dt>
            <dd>
              <span className={styles.modelLead}>Bugs your invariants can state; the rest stays with your bounty.</span>
              <span>
                The automatic lane pays only for broken rules, so its reach is your rules&apos; reach: <code>antibody invariants strength</code>{" "}
                measures it against mutants of your own code before you open (Acme Lend&apos;s fixture rules catch 8 of 28). Harm no rule catches
                goes to the gap lane: refunded, recorded, and paid once a later round adds the rule.
              </span>
            </dd>
          </div>
          <div>
            <dt>A wrong pause?</dt>
            <dd>
              <span className={styles.modelLead}>You choose which rules may pause, and you lift a pause with your fix.</span>
              <span>
                Only the rules you put in your pause policy can pause, and only once the verdict is attested on NEAR. A pause stops the targets
                you registered, nothing else; the release gate lifts it when your fix passes every paid proof, through the same signed permits.
                The worst case is a pause until that fix ships: the funds stay where they are.
              </span>
            </dd>
          </div>
          <div>
            <dt>Design partners</dt>
            <dd>
              <span className={styles.modelLead}>We are onboarding the first protocols for mainnet.</span>
              <span>
                A pilot runs your own invariants as a program on testnet first. <a href="#pilot">Ask for one</a>.
              </span>
            </dd>
          </div>
        </dl>
      </Section>

      <OnThisPage
        items={[
          { href: "#why", label: "Why pay for this" },
          { href: "#what-you-get", label: "What you get" },
          { href: "#business-model", label: "Business model" },
          { href: "#quickstart", label: "Quickstart" },
          { href: "#roadmap", label: "Roadmap" },
          { href: "#pilot", label: "Ask for a pilot" },
        ]}
      />

      <Section
        id="what-you-get"
        title="What a project gets"
        lede="Shown on testnet with Acme Lend, our deliberately vulnerable fixture, except what is marked not yet. Each check runs in your browser: an API record, a NEAR view call or an EVM read."
      >
        <div className={styles.claims}>
          <Claim
            icon="pause"
            title="Pause on proof"
            evidence={
              <>
                <Suspense fallback={<Loading label="Reading the recorded pause" />}>
                  <PauseEvidence />
                </Suspense>
                <Check
                  check={{ kind: "api", path: "/v1/programs/5/reflex" }}
                  expect={{ path: "chains.0.permits", contains: "Pause", says: "each chain lists the pause permit for commit 13, then the release permits" }}
                  cli="antibody judge --program 5"
                  label="Read the permits"
                />
              </>
            }
          >
            <p>
              An attested Critical in your pause policy makes near-guardian ask the MPC network to sign a pause permit for
              every registered chain. Anyone may relay it, and your adapter checks it on chain before it pauses. The pause is asked
              for before the payment.
            </p>
          </Claim>

          <Claim
            icon="shield"
            title="The release gate"
            evidence={
              <Check
                check={{ kind: "evm", chainId: SEPOLIA, method: "eth_getStorageAt", params: [ACME_LEND_PROXY, EIP1967_SLOT, "latest"] }}
                expect={{ endsWith: ACME_LEND_FINAL, says: `Acme Lend's proxy on Sepolia points at the final fix (…${ACME_LEND_FINAL.slice(-8)})` }}
                cli={curlRpc(SEPOLIA, "eth_getStorageAt", [ACME_LEND_PROXY, EIP1967_SLOT, "latest"])}
                label="Read the proxy's implementation"
              />
            }
          >
            <p>
              A fix ships only when every paid proof fails against it. In the recorded run, round 1&apos;s partial fix left two
              paid proofs still breaking a rule: the gate stayed shut and no release permit was signed. Round 2&apos;s final fix:
              every paid proof failed, release permits upgraded both proxies on both chains, and B&apos;s exploit now reverts.
            </p>
            <p className="small">
              <Link href="/programs/5">Program 5&apos;s rounds and permits</Link>
            </p>
          </Claim>

          <Claim icon="refresh" title="Immune memory">
            <ul className={styles.rmParts}>
              <li>
                <Pill tone="ok" size="sm">
                  every round
                </Pill>
                <span>
                  Inside a program: every paid proof, from any round, is re-run against each proposed fix, so a fix that brings an
                  old bug back is caught before it ships.
                </span>
              </li>
              <li>
                <Pill tone="ok" size="sm">
                  testnet
                </Pill>
                <span>
                  On every pull request, in the enclave: the pull request&apos;s build is proposed as a round of a program kept for these
                  checks, and every paid proof, sealed or not, is re-run inside the enclave. Only which keys still break is published.
                  On the{" "}
                  <a href={DEMO_REPO} target="_blank" rel="noreferrer noopener">
                    demo repository
                  </a>
                  , a harmless change reads green and a pull request that brings back a paid defect reads red, naming the rule. How it
                  works:{" "}
                  <DocLink path="docs/immune.md" hash="re-runs-in-the-enclave">
                    docs/immune.md
                  </DocLink>
                </span>
              </li>
              <li>
                <Pill tone="info" size="sm">
                  on the runner
                </Pill>
                <span>
                  <code>antibody immune</code> and a GitHub Action re-run every paid proof the check can read (disclosed, published, or
                  opened with your project key) against the pull request&apos;s build, with the local verifier. Pull requests from forks
                  get this check only.
                </span>
              </li>
              <li>
                <NotYet>next</NotYet>
                <span>A bounty window per pull request, built and not yet run on testnet, and a check-only proposal mode.</span>
              </li>
            </ul>
          </Claim>

          <Claim
            icon="chain"
            title="NEAR apps that act on verdicts"
            evidence={
              <Suspense fallback={<Loading label="Preparing the check" />}>
                <GuardCheck />
              </Suspense>
            }
          >
            <p>
              Verdicts are NEAR state, so any NEAR contract can read them and act. <code>guard.antibody.testnet</code>, a DAO
              treasury the Acme project owns, froze on A&apos;s attested Critical (commit 13), not on D&apos;s duplicate (commit
              16), and refused the next transfer. The same reads fit a lending market, a DAO vote or an insurance pool (
              <DocLink path="docs/verdict-guard.md">docs/verdict-guard.md</DocLink>).
            </p>
          </Claim>
        </div>
      </Section>

      <Section id="business-model" title="Business model" lede="Verification at cost, fees pinned per program, and forfeited bonds that are never ours.">
        <dl className={styles.model}>
          <div>
            <dt>For</dt>
            <dd>
              <span className={styles.modelLead}>Protocols before a launch or an upgrade.</span>
              <span>
                No team reads reports: the verifier pays or refuses a proof in minutes, and a Critical pauses every chain you registered before
                anyone has read it. A fix ships only once every paid proof fails against it.
              </span>
            </dd>
          </div>
          <div>
            <dt>Price</dt>
            <dd>
              <span className={styles.modelLead}>5% of each payout. Nothing when nothing is found.</span>
              <span>
                The proposed mainnet price (0 on this testnet). A program that pays two Criticals of 50,000 and one High of 10,000 pays 5,500 in
                fees, taken from its escrow after the hunters are paid. No triage team, no retainer: each verification run is paid at cost by
                whoever asks for it.
              </span>
            </dd>
          </div>
          <div>
            <dt>Verification</dt>
            <dd>
              <span className={styles.modelLead}>At cost.</span>
              <span>
                The requester of each verification run pays for it: a hunter for a proof, a project to read a paid finding, anyone
                for a re-run at a fix.
              </span>
            </dd>
          </div>
          <div>
            <dt>Revenue</dt>
            <dd>
              <span className={styles.modelLead}>A fee on payouts and a fee per release permit.</span>
              <span>Both are pinned per program: a live program&apos;s pins never change (property 12).</span>
              <Suspense fallback={<Loading label="Preparing the check" />}>
                <FeeCheck />
              </Suspense>
            </dd>
          </div>
          <div>
            <dt>Forfeited bonds</dt>
            <dd>
              <span className={styles.modelLead}>Never revenue.</span>
              <span>A bond is refunded, or forfeited to the program&apos;s own escrow (property 7): never to us.</span>
              <Check
                check={{ kind: "api", path: "/v1/programs" }}
                expect={{
                  path: "programs.5.ledger.forfeits",
                  equals: "1000000000000000000000",
                  says: "program 5's ledger counts commit 15's forfeited bond, 0.001 wNEAR, in its own escrow",
                }}
                cli={`curl -s ${config.apiShown}/v1/programs`}
                label="Read the ledgers"
              />
            </dd>
          </div>
          <div>
            <dt>Later</dt>
            <dd>
              <span className={styles.modelLead}>A paid tier for the drafter, rule strength and analytics.</span>
              <span>
                <NotYet />
              </span>
            </dd>
          </div>
        </dl>
      </Section>

      <Section
        id="quickstart"
        title="The quickstart"
        lede={
          <>
            Nine steps with their exact commands, against the testnet deployment the CLI has built in. The full text:{" "}
            <DocLink path="docs/project-quickstart.md">docs/project-quickstart.md</DocLink>.
          </>
        }
      >
        <div className="callout" style={{ marginBottom: 8 }}>
          <Icon name="info" size={16} />
          <span>
            Rehearse it offline first, on Linux or macOS: <code>make serve</code> runs the whole story in a sandbox and keeps it up.
            Add <code>--network sandbox --rpc http://127.0.0.1:3030 --deployments target/demo/deployments.json</code> to any command.
          </span>
        </div>
        <Steps steps={steps} label="Open a program, step by step" />
      </Section>

      <Section
        id="roadmap"
        title="Roadmap"
        lede={
          <>
            From docs/roadmap.md, in its order. Nothing below is built yet, except the parts marked done.{" "}
            <DocLink path="docs/roadmap.md">docs/roadmap.md</DocLink>
          </>
        }
      >
        <Roadmap />
      </Section>

      <PilotCta id="pilot" role="project" title="Ask for a pilot" cta="Ask for a pilot" secondary={{ href: "/pilot", label: "I hunt or run an agent" }}>
        Tell us about your contracts, the chains they run on and your timeline. We reply about a pilot; your details stay on our
        server and are never published.
      </PilotCta>
    </>
  );
}
