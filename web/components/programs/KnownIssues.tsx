import Link from "next/link";
import { SeverityBadge } from "@/components/ui/Badges";
import { Hash } from "@/components/ui/Hash";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { Empty } from "@/components/ui/States";
import { TxLink } from "@/components/ui/TxLink";
import { Sealed } from "@/components/ui/Verdict";
import type { ProgramDetail } from "@/lib/api";
import { findEvent, rowsWithTag, slotName, usdMicro, whoLabel, type SlotView } from "./helpers";
import styles from "./programs.module.css";

/** 5. Known issues: declared sealed before open, so a later proof of the same key is not paid again. */
export function KnownIssues({ d, slots, id, antibody }: { d: ProgramDetail; slots: SlotView[]; id: number; antibody: string }) {
  const known = Array.isArray(d.program?.known) ? d.program.known : [];
  const count = typeof d.rules?.known_issues_sealed === "number" ? d.rules.known_issues_sealed : known.length;
  const opened = findEvent(d, "program_opened");
  const rows = d.commits ?? [];
  const first = known[0]?.tags?.[0];

  return (
    <Section
      id="known"
      title="Known issues"
      lede="Declared sealed before open; a later proof with the same tag is not paid again. Their count, rules and severities are public; their content stays sealed for good."
      actions={
        <Pill tone={count ? "neutral" : "ok"} icon="lock" size="sm">
          {count} sealed
        </Pill>
      }
    >
      {known.length ? (
        <div className="stack">
          <div className="grid grid-2">
            {known.map((k, i) => {
              const declared = typeof k.run === "number" ? findEvent(d, "known_issue", (x) => x.run_id === k.run) : undefined;
              const matched = (k.tags ?? []).flatMap((t) => rowsWithTag(rows, t));
              return (
                <div key={`${k.run}-${i}`} className="card stack" style={{ ["--gap" as string]: "10px" }}>
                  <div className={styles.roundHead}>
                    <h3 className={styles.cardTitle}>Known issue {i + 1}</h3>
                    <div className="row" style={{ ["--gap" as string]: "6px" }}>
                      <SeverityBadge severity={k.severity} size="sm" />
                      {k.fixed ? (
                        <Pill tone="ok" size="sm">
                          fixed
                        </Pill>
                      ) : (
                        <Pill tone="warn" size="sm">
                          not fixed: still breaks
                        </Pill>
                      )}
                    </div>
                  </div>
                  <Sealed seed={(k.tags ?? []).join("") || `known-${i}`} label="Content sealed for good" lines={2} />
                  <dl className={`kv ${styles.kvStack}`}>
                    <dt>Rules</dt>
                    <dd className="small">{(k.slots ?? []).map((b) => `${slotName(slots, b)} (slot ${b})`).join(", ") || "—"}</dd>
                    <dt>Tags</dt>
                    <dd className="stack" style={{ ["--gap" as string]: "4px" }}>
                      {(k.tags ?? []).map((t) => (
                        <Hash key={t} value={t} head={16} label="known issue tag" />
                      ))}
                    </dd>
                    <dt>Verified by</dt>
                    <dd className="small">
                      {typeof k.run === "number" ? <Link href={`/runs/${k.run}`}>run {k.run}</Link> : <span className="muted">—</span>}
                      {declared ? (
                        <>
                          {" "}
                          · <TxLink hash={declared.tx} href={declared.explorer} />
                        </>
                      ) : null}
                    </dd>
                    {k.impact_usd_micro && k.impact_usd_micro !== "0" ? (
                      <>
                        <dt>Impact bound</dt>
                        <dd className="num">{usdMicro(k.impact_usd_micro)}</dd>
                      </>
                    ) : null}
                    <dt>Later proofs</dt>
                    <dd className="small">
                      {matched.length ? (
                        matched.map((r, j) => (
                          <span key={r.commit_id}>
                            {j ? ", " : ""}
                            <Link href={`/commits/${r.commit_id}`}>commit {r.commit_id}</Link>
                            {whoLabel(r.hunter, r.program_id) ? ` (${whoLabel(r.hunter, r.program_id)})` : ""}
                          </span>
                        ))
                      ) : (
                        <span className="muted">none yet</span>
                      )}
                      {matched.length ? <span className="muted">: matched the known issue, not paid</span> : null}
                    </dd>
                  </dl>
                </div>
              );
            })}
          </div>
          <p className="small muted">
            Declared before open{opened ? <> (opened with {String(opened.data?.known_issues ?? count)} known: <TxLink hash={opened.tx} href={opened.explorer} />)</> : null}. A round may never add one, and a known issue blocks no release.
          </p>
          {first ? (
            <LiveCheck
              check={{ kind: "near-view", contract: antibody, method: "get_program", args: { program_id: id } }}
              expect={{ path: "known.0.tags.0", equals: first, says: "the first known issue's tag, as committed before open" }}
              cli={`antibody status --program ${id}`}
              label="Read the known issues on NEAR"
            />
          ) : null}
        </div>
      ) : (
        <Empty>No known issue declared: every key this program prices was open to the first proof.</Empty>
      )}
    </Section>
  );
}
