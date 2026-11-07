import Link from "next/link";
import { Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { AccountLink } from "@/components/ui/TxLink";
import { rawOf } from "@/lib/wallet/amounts";
import type { AgentRecord, AgentStatement } from "@/lib/wallet/types";
import { Amount } from "./shared";

/** Programs an agent is bound to: null is any program. */
export function ProgramList({ programs }: { programs: number[] | null | undefined }) {
  if (!programs) return <span>any program</span>;
  if (!programs.length) return <span className="muted">—</span>;
  return (
    <span>
      {programs.map((p, i) => (
        <span key={p}>
          <Link href={`/programs/${p}`}>program {p}</Link>
          {p <= 4 ? <span className="muted xsmall"> (earlier run)</span> : null}
          {i < programs.length - 1 ? ", " : ""}
        </span>
      ))}
    </span>
  );
}

/**
 * The agent statement (spec §13, near-antibody's `agent_statement`): what the agent may cost its owner and
 * what it has cost and earned. Its bonds come from the owner's token credit; its payouts go to the owner.
 */
export function AgentStatementView({ statement, record }: { statement: AgentStatement | null; record: AgentRecord | null }) {
  const s = statement ?? { registered: Boolean(record) };
  const budget = rawOf(s.bond_budget ?? record?.bond_budget);
  const locked = rawOf(s.bonds_locked ?? record?.bonds_locked) ?? BigInt(0);
  const lost = rawOf(s.bonds_lost ?? record?.bonds_lost) ?? BigInt(0);
  const headroom = budget !== null ? budget - locked - lost : null;
  const containment = s.containment_events ?? record?.containment_events ?? 0;
  return (
    <Section
      id="agent"
      eyebrow="This account is an agent"
      title="Agent statement"
      lede="An agent commits with its owner's money: bonds come from the owner's token credit, within a per-commit cap and a budget that counts bonds locked and lost; payouts go to the owner; its key cannot withdraw."
    >
      <div className="grid grid-2">
        <dl className="kv card">
          <dt>Owner</dt>
          <dd>
            <AccountLink account={s.owner ?? record?.owner} />
          </dd>
          <dt>Bond budget</dt>
          <dd>
            <Amount raw={budget} />
          </dd>
          <dt>Headroom</dt>
          <dd>
            {headroom !== null ? <Amount raw={headroom} /> : <span className="muted">—</span>}
            <span className="xsmall muted"> · budget less bonds locked and lost</span>
          </dd>
          <dt>Per-commit cap</dt>
          <dd>
            <Amount raw={s.per_commit_cap ?? record?.per_commit_cap} />
          </dd>
          <dt>Bound to</dt>
          <dd>
            <ProgramList programs={record?.programs} />
          </dd>
          <dt>Key</dt>
          <dd className="small">{s.key_permissions ?? "commit, reveal and upload; no withdrawal, transfer or deposit"}</dd>
        </dl>
        <dl className="kv card">
          <dt>Bonds locked</dt>
          <dd>
            <Amount raw={locked} />
          </dd>
          <dt>Bonds lost</dt>
          <dd>
            <Amount raw={lost} />
          </dd>
          <dt>Bonds returned</dt>
          <dd>
            <Amount raw={s.bonds_returned ?? record?.bonds_returned} />
          </dd>
          <dt>Paid (to the owner)</dt>
          <dd>
            <Amount raw={s.paid ?? record?.paid} />
          </dd>
          <dt>Net</dt>
          <dd>
            <Amount raw={s.net_tokens} />
            <span className="xsmall muted"> · paid less bonds lost</span>
          </dd>
          <dt>Verification spend</dt>
          <dd>
            <Amount raw={s.verification_spent_yocto ?? record?.verification_spent} unit="NEAR" />
          </dd>
          <dt>Commits</dt>
          <dd className="num">{s.commits ?? record?.commits ?? "—"}</dd>
          <dt>Containment events</dt>
          <dd>
            {containment > 0 ? (
              <Pill tone="warn" size="sm">
                {containment} refused
              </Pill>
            ) : (
              <span className="num">0</span>
            )}
            <span className="xsmall muted"> · commits refused for leaving the cap, the budget or the programs</span>
          </dd>
        </dl>
      </div>
    </Section>
  );
}
