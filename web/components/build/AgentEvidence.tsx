import Link from "next/link";
import { Empty } from "@/components/ui/States";
import { TxLink } from "@/components/ui/TxLink";
import { fixtureAgentLetter, provenance, wnear } from "@/lib/format";
import { getMoments } from "@/lib/moment";
import { deployment } from "./deployment";
import { OfflineCopy, Recorded } from "./Labels";
import { Check } from "./Check";
import styles from "./build.module.css";

/** A practice account from the recorded run (JUDGES.md, claim 12): an agent of the vending contract. */
export const PRACTICE_AGENT = "judge-1-p6.practice.antibody.testnet";

const seconds = (s: number | undefined) => (typeof s === "number" && Number.isFinite(s) ? `${s.toFixed(1)} s` : "—");

/**
 * The one agent submission over MCP that is recorded: our fixture agent A in the recorded run (a
 * results file, served by /v1/moments, or the bundled copy when the API is down).
 */
export async function RecordedAgentRun() {
  const { data, live } = await getMoments();
  const runs = Array.isArray(data?.moments) ? data.moments : [];
  const m = runs.find((x) => x.id === data.latest && /mcp/i.test(x.via ?? "")) ?? runs.find((x) => /mcp/i.test(x.via ?? ""));
  if (!m) return <Empty>No recorded submission over MCP yet.</Empty>;
  const steps = Array.isArray(m.steps) ? m.steps : [];
  const at = (k: string) => steps.find((s) => s.key === k);
  const paid = at("paid");
  const who =
    m.hunter && provenance(m.hunter) === "fixture-agent" ? `our fixture agent ${fixtureAgentLetter(m.hunter)}` : (m.hunter ?? "our fixture agent A");
  return (
    <div className="card stack">
      <div className={styles.labelRow}>
        <Recorded />
        {live ? null : <OfflineCopy />}
        <span className="xsmall muted">
          program {m.program_id} (Acme Lend, our fixture) · commit {m.commit_id}
        </span>
      </div>
      <h3>What is recorded: {who}, over MCP</h3>
      <p className="small muted">
        A script drove the MCP server the way an AI agent would, with a function-call key, and submitted a sealed proof of a
        Critical. It shows the MCP path working end to end on testnet; it says nothing about an agent finding a bug.
      </p>
      <dl className={styles.strip}>
        <div>
          <dt>Verified</dt>
          <dd className="num">{seconds(at("verified")?.seconds)}</dd>
        </div>
        <div>
          <dt>T1-confirmed</dt>
          <dd className="num">{seconds(at("t1")?.seconds)}</dd>
        </div>
        <div>
          <dt>Paid</dt>
          <dd className="num">{seconds(paid?.seconds)}</dd>
        </div>
      </dl>
      <p className="xsmall muted">
        Seconds from the commit&apos;s block on NEAR testnet. Paid {wnear(m.paid_raw)}
        {m.instant ? ", an instant payout (T1, under the cap)" : ""}.
      </p>
      <ul className={styles.txList}>
        <li>
          <span className="muted">Commit</span> <TxLink hash={m.commit?.tx} href={m.commit?.explorer} />
        </li>
        <li>
          <span className="muted">Payment</span> <TxLink hash={paid?.tx} href={paid?.explorer} />
        </li>
      </ul>
      <Check
        check={{ kind: "api", path: `/v1/commits/${m.commit_id}` }}
        expect={{ path: "row.state", equals: "Done:Paid", says: "state Done:Paid" }}
        label="Read the state"
      />
      <Check
        check={{ kind: "api", path: `/v1/commits/${m.commit_id}` }}
        expect={{ path: "row.verdict.tier", equals: "T1", says: "tier T1: the TDX quote verified on NEAR" }}
        cli={`antibody status --commit ${m.commit_id}`}
        label="Read the tier"
      />
      <p className="small">
        <Link href={`/commits/${m.commit_id}`}>Commit {m.commit_id}, step by step</Link> · <Link href="/moment">The one moment</Link>
      </p>
    </div>
  );
}

/** An agent's limits, read live: from the API's account record, and from near-antibody itself. */
export async function ContainmentChecks() {
  const d = await deployment();
  return (
    <div className="grid grid-2">
      <div className="card stack">
        <h3>From the API: an agent&apos;s limits</h3>
        <p className="small muted">
          <span className="mono">judge-1-p6</span>, a practice account from the recorded run, is an agent of the vending
          contract: its owner, a per-commit cap, a budget, and one program it may commit to.
        </p>
        <Check
          check={{ kind: "api", path: `/v1/accounts/${PRACTICE_AGENT}` }}
          expect={{ path: "credit.agent.programs.0", equals: 6, says: `bound to program 6, the practice program; its owner is ${d.vending}` }}
          cli={`antibody status --of ${PRACTICE_AGENT}`}
        />
      </div>
      <div className="card stack">
        <h3>From NEAR: what its key may do</h3>
        <p className="small muted">near-antibody&apos;s own statement for the same agent, read by your browser from a NEAR RPC.</p>
        <Check
          check={{ kind: "near-view", contract: d.antibody, method: "agent_statement", args: { agent: PRACTICE_AGENT } }}
          expect={{ path: "key_permissions", contains: "no withdrawal", says: "commit, reveal, upload and the permissionless keeper methods: no withdrawal, transfer or deposit" }}
          cli={`antibody view --contract ${d.antibody} --method agent_statement --args '{"agent":"${PRACTICE_AGENT}"}'`}
        />
      </div>
    </div>
  );
}
