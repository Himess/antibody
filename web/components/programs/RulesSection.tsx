import { Suspense } from "react";
import { SeverityBadge } from "@/components/ui/Badges";
import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { Section } from "@/components/ui/PageHead";
import { Skeleton } from "@/components/ui/States";
import type { ProgramDetail } from "@/lib/api";
import { chainName, explorers, sourceUrl } from "@/lib/config";
import { wnear } from "@/lib/format";
import { Cell } from "./Cell";
import { getJudge } from "./data";
import { isZero, ruleSource, type ProgramKind, type SlotView } from "./helpers";
import styles from "./programs.module.css";

interface Strength {
  caught?: number;
  total?: number;
  label?: string;
  report_sha256?: string;
  by_class?: Record<string, { caught?: number; total?: number }>;
}

/** Rule strength, from `/v1/judge` (`antibody judge`): how many deliberately broken versions the rules catch. */
async function RuleStrength({ id }: { id: number }) {
  const j = await getJudge();
  if (!j.ok) return null;
  const programs = Array.isArray(j.data?.programs) ? (j.data.programs as Record<string, unknown>[]) : [];
  const rs = programs.find((p) => p?.program_id === id)?.rule_strength as Strength | undefined;
  if (!rs || typeof rs.caught !== "number" || typeof rs.total !== "number") return null;
  const classes = Object.entries(rs.by_class ?? {});
  return (
    <div className="card stack">
      <h3 className={styles.cardTitle}>Rule strength</h3>
      <p className="small">
        The rules caught{" "}
        <strong className="num">
          {rs.caught} of {rs.total}
        </strong>{" "}
        deliberately broken versions (mutants) of the final fix. It measures the rules, not the code&apos;s security, and a mutant
        that survives is not necessarily a bug.
      </p>
      {classes.length ? (
        <ul className={`small ${styles.list}`}>
          {classes.map(([k, v]) => (
            <li key={k}>
              <span className="mono">{k}</span>: {v.caught ?? "—"} of {v.total ?? "—"}
            </li>
          ))}
        </ul>
      ) : null}
      <div className="row xsmall muted" style={{ ["--gap" as string]: "8px" }}>
        <span>Report sha256</span>
        <Hash value={rs.report_sha256} head={12} label="rule strength report sha256" />
        <a href={sourceUrl("docs/rule-strength.md")} target="_blank" rel="noreferrer noopener">
          How it is measured <Icon name="external" size={11} className={styles.inlineIcon} />
        </a>
      </div>
      <Command label="CLI command">{`antibody judge --program ${id}`}</Command>
    </div>
  );
}

function Targets({ d, kind }: { d: ProgramDetail; kind: ProgramKind }) {
  const targets = Array.isArray(d.meta?.targets) ? d.meta.targets : [];
  if (!targets.length) return null;
  const chainId = d.meta?.chain_id ?? d.rules?.anchor?.chain_id;
  const forkOnly = kind === "fork";
  return (
    <details className={styles.details}>
      <summary>
        The targets ({targets.length}){chainId ? ` on ${chainName(chainId)}` : ""}
        {forkOnly ? ": deployed only inside the verifier" : ""}
      </summary>
      <ul className={styles.targets}>
        {targets.map(([name, address]) => (
          <li key={`${name}-${address}`}>
            <span className="mono small">{name}</span>
            <Hash
              value={address}
              head={10}
              tail={4}
              href={!forkOnly && chainId ? explorers.evmAddress(chainId, address) : undefined}
              label={`${name} address`}
            />
          </li>
        ))}
      </ul>
    </details>
  );
}

/** 2. Each slot of the invariants contract: its severity, its payout, and the line that writes it. */
export function RulesSection({ d, kind, slots, id }: { d: ProgramDetail; kind: ProgramKind; slots: SlotView[]; id: number }) {
  return (
    <Section
      id="rules"
      title="The rules as code"
      lede="Each rule is one bit of the program's invariants contract. After a proof's calls, at the anchor, the verifier runs its check(): a set bit is a broken rule, paid by its severity. The verifier never reads a report."
    >
      <div className="stack">
        {slots.length ? (
          <div className={`table-wrap ${styles.cardsWrap}`}>
            <table className={`table ${styles.rtable}`}>
              <caption className="visually-hidden">The program&apos;s rules</caption>
              <thead>
                <tr>
                  <th scope="col">Rule</th>
                  <th scope="col">Severity</th>
                  <th scope="col">Payout</th>
                  <th scope="col">Budget per round</th>
                  <th scope="col">Source</th>
                </tr>
              </thead>
              <tbody>
                {slots.map((s) => {
                  const src = s.name ? ruleSource(kind, s.name) : null;
                  return (
                    <tr key={s.bit}>
                      <Cell label="Rule" lead>
                        <div className={styles.keyRow}>
                          <span className="mono">{s.name ?? `slot ${s.bit}`}</span>
                          <span className="tag">bit {s.bit}</span>
                          {s.added ? <span className="tag">added by a round</span> : null}
                          {s.pauses ? <span className="tag">an attested proof pauses</span> : null}
                        </div>
                        {src ? <p className={styles.says}>{src.says}</p> : null}
                      </Cell>
                      <Cell label="Severity">
                        <SeverityBadge severity={s.severity} size="sm" />
                      </Cell>
                      <Cell label="Payout">
                        <span className="num">{s.payout_raw && !isZero(s.payout_raw) ? wnear(s.payout_raw) : "not paid"}</span>
                      </Cell>
                      <Cell label="Budget per round">
                        <span className="num">{s.budget_raw ? wnear(s.budget_raw) : "the escrow"}</span>
                      </Cell>
                      <Cell label="Source">
                        {src ? (
                          <a href={sourceUrl(src.path, src.line)} target="_blank" rel="noreferrer noopener" className="small">
                            {src.contract}, line {src.line} <Icon name="external" size={11} className={styles.inlineIcon} />
                            <span className="visually-hidden"> (the source of {s.name}, opens the repository)</span>
                          </a>
                        ) : (
                          <span className="muted small">in the committed manifest</span>
                        )}
                      </Cell>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        ) : (
          <p className="small muted">The API sent no rules for this program.</p>
        )}
        <p className="small muted">
          The budget caps what one rule pays per round, so many entry points into one root cause cannot drain the escrow. A
          finding&apos;s key is the rule and the entry point that broke it (by default, the target and function): the first proof
          of each key is paid.
        </p>
        <Targets d={d} kind={kind} />
        <Suspense fallback={<Skeleton height={120} />}>
          <RuleStrength id={id} />
        </Suspense>
      </div>
    </Section>
  );
}
