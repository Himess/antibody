import Link from "next/link";
import { Hash } from "@/components/ui/Hash";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { Section } from "@/components/ui/PageHead";
import { Time } from "@/components/ui/Time";
import { AccountLink, TxLink } from "@/components/ui/TxLink";
import type { ProgramDetail } from "@/lib/api";
import { amount, seconds, TOKEN } from "@/lib/format";
import { bps, findEvent, num, str, type ProgramKind } from "./helpers";
import { LocationView } from "./Location";
import { PhaseBadge } from "./ProgramBadges";
import styles from "./programs.module.css";

function projectLabel(project: string, kind: ProgramKind): string | null {
  if (project === "acme.antibody.testnet") return "our fixture project";
  if (project === "antibody.testnet") return "ours";
  if (kind === "fixture" && project.endsWith(".antibody.testnet")) return "our fixture project";
  return null;
}

/** 1. Who opened the program, when, and the hashes everything else is checked against. */
export function Overview({ d, kind, id, antibody }: { d: ProgramDetail; kind: ProgramKind; id: number; antibody: string }) {
  const p = d.program;
  const created = findEvent(d, "program_created");
  const opened = findEvent(d, "program_opened");
  const activated = findEvent(d, "program_activated");
  const activationRun = num(p.activation_run);
  const roundStarted = num(p.round_started_at);
  const sealing = str(p.sealing_pk);
  const rec = d.record;
  const label = p.project ? projectLabel(p.project, kind) : null;

  return (
    <Section
      id="overview"
      title="Overview"
      lede="The program as near-antibody stores it. Its manifest hash commits the anchor, the targets, the rules and the terms; once it is open, the deployment's owner can change nothing it depends on."
    >
      <div className="grid grid-2">
        <div className="card">
          <dl className={`kv ${styles.kvStack}`}>
            <dt>Project</dt>
            <dd>
              <AccountLink account={p.project} />
              {label ? <span className="xsmall muted"> · {label}</span> : null}
            </dd>
            <dt>Program key</dt>
            <dd>
              <Hash value={p.key} full label="program key" />
            </dd>
            <dt>Phase</dt>
            <dd className="stack" style={{ ["--gap" as string]: "4px" }}>
              <span>
                <PhaseBadge phase={p.phase} size="sm" /> <span className="small">round {typeof p.round === "number" ? p.round : "—"}</span>
              </span>
              {roundStarted ? (
                <span className="xsmall muted">
                  this round since <Time t={roundStarted} showRelative={false} />
                </span>
              ) : null}
            </dd>
            <dt>Created</dt>
            <dd className="stack" style={{ ["--gap" as string]: "2px" }}>
              <Time t={p.created_at} />
              {created ? <TxLink hash={created.tx} href={created.explorer} /> : null}
            </dd>
            <dt>Opened</dt>
            <dd className="stack" style={{ ["--gap" as string]: "2px" }}>
              <Time t={p.opened_at} />
              {opened ? <TxLink hash={opened.tx} href={opened.explorer} /> : null}
            </dd>
            <dt>Terms sha256</dt>
            <dd>
              <Hash value={d.meta?.terms_sha256} head={16} label="terms sha256" />
            </dd>
            <dt>Manifest sha256</dt>
            <dd className="stack" style={{ ["--gap" as string]: "4px" }}>
              <Hash value={p.manifest_sha256} head={16} label="manifest sha256" />
              <LocationView location={d.meta?.manifest_url} what="The manifest" />
            </dd>
            <dt>Sealing key</dt>
            <dd className="stack" style={{ ["--gap" as string]: "4px" }}>
              <Hash value={sealing} head={16} label="sealing public key" />
              <span className="xsmall muted">
                Proofs are sealed to it. It counts only once its activation run
                {activationRun !== null ? (
                  <>
                    {" "}
                    (<Link href={`/runs/${activationRun}`}>run {activationRun}</Link>)
                  </>
                ) : null}{" "}
                is T1-confirmed{activated ? <>: <TxLink hash={activated.tx} href={activated.explorer} /></> : "."}
              </span>
            </dd>
          </dl>
        </div>

        <div className="card stack">
          <h3 className={styles.cardTitle}>Its record</h3>
          {rec ? (
            <dl className={styles.stats}>
              <div>
                <dt>At stake</dt>
                <dd>
                  {amount(rec.at_stake)}
                  <small>{TOKEN.symbol} available now</small>
                </dd>
              </div>
              <div>
                <dt>Verified attempts</dt>
                <dd>{typeof rec.verified_attempts === "number" ? rec.verified_attempts : "—"}</dd>
              </div>
              <div>
                <dt>Paid findings</dt>
                <dd>{typeof rec.paid_findings === "number" ? rec.paid_findings : "—"}</dd>
              </div>
              <div>
                <dt>First claim</dt>
                <dd>
                  {seconds(rec.time_to_first_claim_secs, 0)}
                  <small>after it opened</small>
                </dd>
              </div>
              <div>
                <dt>Claimed in the first hour</dt>
                <dd>
                  {bps(rec.share_claimed_first_hour_bps)}
                  <small>of the escrow</small>
                </dd>
              </div>
              <div>
                <dt>Open for</dt>
                <dd>{seconds(rec.survived_secs)}</dd>
              </div>
            </dl>
          ) : (
            <p className="small muted">The API sent no record for this program.</p>
          )}
          <LiveCheck
            check={{ kind: "near-view", contract: antibody, method: "get_program", args: { program_id: id } }}
            expect={p.manifest_sha256 ? { path: "manifest_sha256", equals: p.manifest_sha256, says: "the manifest hash shown here, as near-antibody stores it" } : undefined}
            cli={`antibody judge --program ${id}`}
            label="Read the program on NEAR"
          />
        </div>
      </div>
    </Section>
  );
}
