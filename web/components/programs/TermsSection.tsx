import { SeverityBadge } from "@/components/ui/Badges";
import { Section } from "@/components/ui/PageHead";
import { Time } from "@/components/ui/Time";
import type { ProgramDetail, Severity } from "@/lib/api";
import { near, wnear } from "@/lib/format";
import { Cell } from "./Cell";
import { CoverageBar } from "./CoverageBar";
import { bps, coverageOf, duration, isZero, largestPayout, num, usdMicro, WINDOWS, type SlotView } from "./helpers";
import styles from "./programs.module.css";

const SEVERITIES: Severity[] = ["critical", "high", "medium", "low"];

const money = (raw: unknown) => (typeof raw === "string" && raw ? (isZero(raw) ? "none" : wnear(raw)) : "—");

/** 3. What the contract enforces: payouts, the bond, instant payments, fees, the windows and the escrow. */
export function TermsSection({ d, slots }: { d: ProgramDetail; slots: SlotView[] }) {
  const params: Record<string, unknown> = d.program?.params ?? {};
  const rules = d.rules;
  const windows = (params.windows ?? {}) as Record<string, number>;
  const largest = largestPayout(rules?.payouts_raw);
  const available = d.program?.ledger?.available ?? d.audit?.available;
  const coverage = coverageOf(rules?.escrow_coverage, available, largest);
  const gap = params.gap && typeof params.gap === "object" ? (params.gap as { claim_amount?: string; threshold_usd_micro?: string }) : null;
  const pauseSlots = Array.isArray(params.pause_slots) ? (params.pause_slots as number[]) : [];
  const knownMax = num(params.known_max_count);
  const knownShare = num(params.known_max_share_bps);
  const deadline = num(windows.deadline);
  const shownWindows = WINDOWS.filter((w) => typeof windows[w.key] === "number");
  const minImpact = params.pause_min_impact_usd_micro;
  const noMinImpact = (typeof minImpact === "string" || typeof minImpact === "number") && isZero(String(minImpact));

  return (
    <Section
      id="terms"
      title="The terms"
      lede="Committed in the manifest with the rules, their text fixed by the terms sha256, and enforced by near-antibody: no person decides a payout."
    >
      <div className="stack">
        <div className="grid grid-2">
          <div className="card stack">
            <h3 className={styles.cardTitle}>Payouts</h3>
            <div className={`table-wrap ${styles.inCard}`}>
              <table className="table">
                <caption className="visually-hidden">The payout table</caption>
                <thead>
                  <tr>
                    <th scope="col">Severity</th>
                    <th scope="col">Payout</th>
                    <th scope="col">Rules</th>
                  </tr>
                </thead>
                <tbody>
                  {SEVERITIES.map((sev) => {
                    const of = slots.filter((s) => s.severity === sev);
                    return (
                      <tr key={sev}>
                        <td>
                          <SeverityBadge severity={sev} size="sm" />
                        </td>
                        <td className="num">{money(rules?.payouts_raw?.[sev])}</td>
                        <td className="small">{of.length ? of.map((s) => s.name ?? `slot ${s.bit}`).join(", ") : <span className="muted">none</span>}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
            <dl className={`kv ${styles.kvStack}`}>
              <dt>Bond per commit</dt>
              <dd>
                <span className="num">{money(rules?.bond_raw)}</span>
                <span className="xsmall muted"> · refunded, or forfeited to this program&apos;s escrow when the proof breaks nothing</span>
              </dd>
              <dt>Instant payments</dt>
              <dd>
                <span className="num">up to {money(rules?.instant?.cap_raw)}</span>
                <details className={styles.termMore}>
                  <summary>When a payout is instant</summary>
                  <p className="xsmall muted">
                    Paid in seconds when the verdict is attested on NEAR, within {money(rules?.instant?.budget_raw)} per {duration(rules?.instant?.window_secs)}.
                    Otherwise a payout waits the challenge window; above the cap it also needs an agreeing second run.
                  </p>
                </details>
              </dd>
              <dt>Challenge bond</dt>
              <dd className="num">{money(params.challenge_bond)}</dd>
              <dt>Protocol fee</dt>
              <dd>
                {params.protocol_fee_bps === 0 ? (
                  <>
                    <span className="num">none</span>
                    <span className="xsmall muted"> · pinned at 0 for this program</span>
                  </>
                ) : (
                  <>
                    <span className="num">{bps(params.protocol_fee_bps)}</span>
                    <span className="xsmall muted"> · taken from each payout</span>
                  </>
                )}
              </dd>
              <dt>Release fee</dt>
              <dd>
                {typeof params.release_fee === "string" && params.release_fee && isZero(params.release_fee) ? (
                  <>
                    <span className="num">none</span>
                    <span className="xsmall muted"> · pinned at 0 for this program</span>
                  </>
                ) : (
                  <>
                    <span className="num">{money(params.release_fee)}</span>
                    <span className="xsmall muted"> · charged per release permit</span>
                  </>
                )}
              </dd>
              {typeof params.run_deposit === "string" ? (
                <>
                  <dt>Verification deposit</dt>
                  <dd>
                    <span className="num">{near(params.run_deposit)}</span>
                    <span className="xsmall muted"> per run, paid by whoever asks for it; OutLayer refunds the unused part</span>
                  </dd>
                </>
              ) : null}
            </dl>
          </div>

          <div className="card stack">
            <h3 className={styles.cardTitle}>The escrow against the largest payout</h3>
            <CoverageBar coverage={coverage} available={available} largest={largest} scale />
            <p className="small muted">
              The available escrow over the largest payout. Below 1× the status page raises an alert; a payout the escrow cannot
              cover is queued as debt, in commit order.
            </p>
            <dl className={`kv ${styles.kvStack}`}>
              <dt>Pauses</dt>
              <dd className="small">
                {pauseSlots.length ? (
                  <>
                    An attested proof of {pauseSlots.map((b) => slots.find((s) => s.bit === b)?.name ?? `slot ${b}`).join(", ")}, on a
                    light-client anchor, pauses the protocol on every chain the project registered (one pause ticket per round),
                    {noMinImpact ? " with no minimum impact." : ` from a minimum impact of ${usdMicro(minImpact)}.`}
                  </>
                ) : (
                  "No rule pauses anything."
                )}
              </dd>
              <dt>Known issues</dt>
              <dd className="small">
                {knownMax === 0
                  ? "None allowed."
                  : `At most ${knownMax ?? "—"}, sealed before open, their base payouts at most ${bps(knownShare)} of the escrow.`}
              </dd>
              <dt>Gap lane</dt>
              <dd className="small">
                {gap ? (
                  <>
                    A proof that breaks no rule but harms a declared protected account by more than {usdMicro(gap.threshold_usd_micro)}{" "}
                    is refunded and recorded as a gap claim, with {money(gap.claim_amount)} reserved. It is paid if a later round adds a
                    rule its proof breaks.
                  </>
                ) : (
                  "Off."
                )}
              </dd>
              <dt>Unevaluable</dt>
              <dd className="small">
                {params.unevaluable === "refund"
                  ? "A proof the rules cannot evaluate (check() reverts or runs out of gas) is refunded."
                  : params.unevaluable
                    ? "Counted as a violation of the committed severity."
                    : "—"}
              </dd>
            </dl>
          </div>
        </div>

        <div className="card stack">
          <h3 className={styles.cardTitle}>The windows</h3>
          {shownWindows.length || deadline ? (
            <div className={`table-wrap ${styles.cardsWrap} ${styles.inCard}`}>
              <table className={`table ${styles.rtable} ${styles.windows}`}>
                <caption className="visually-hidden">The program&apos;s windows</caption>
                <thead>
                  <tr>
                    <th scope="col">Window</th>
                    <th scope="col">Length</th>
                    <th scope="col">What it bounds</th>
                  </tr>
                </thead>
                <tbody>
                  {shownWindows.map((w) => (
                    <tr key={w.key}>
                      <Cell label="Window" lead>
                        <strong className="small">{w.label}</strong>
                      </Cell>
                      <Cell label="Length">
                        <span className="num">{duration(windows[w.key])}</span>
                      </Cell>
                      <Cell label="Bounds">
                        <span className="small muted">{w.says}</span>
                      </Cell>
                    </tr>
                  ))}
                  {deadline ? (
                    <tr>
                      <Cell label="Window" lead>
                        <strong className="small">Deadline</strong>
                      </Cell>
                      <Cell label="At">
                        <Time t={deadline} />
                      </Cell>
                      <Cell label="Bounds">
                        <span className="small muted">No commit is accepted after it.</span>
                      </Cell>
                    </tr>
                  ) : null}
                </tbody>
              </table>
            </div>
          ) : (
            <p className="small muted">The API sent no windows for this program.</p>
          )}
        </div>
      </div>
    </Section>
  );
}
