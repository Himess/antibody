import Link from "next/link";
import type { ReactNode } from "react";
import { KindBadge, TierBadge } from "@/components/ui/Badges";
import { CopyButton } from "@/components/ui/CopyButton";
import { Hash } from "@/components/ui/Hash";
import { Pill } from "@/components/ui/Pill";
import { Time } from "@/components/ui/Time";
import { AccountLink, TxLink } from "@/components/ui/TxLink";
import type { IndexedEvent, ProgramDetail } from "@/lib/api";
import { seconds } from "@/lib/format";
import { isObj, MODE_WORDS, slotInfo, slotName, strOf, usd, who, type Answered } from "./model";
import styles from "./commit.module.css";

function Account({ account, project }: { account: string | null; project: string | null }) {
  if (!account) return <span className="muted">—</span>;
  const w = project && account === project ? "the program's project" : who(account);
  return (
    <span className={styles.inline}>
      <AccountLink account={account} />
      {w ? <span className="small muted">{w}</span> : null}
    </span>
  );
}

/** What the facts' mode-specific `extra` carries: a disclosure wrap, a sealing key, a round check, … */
function Extra({ extra }: { extra: unknown }): ReactNode {
  if (!isObj(extra)) return <span className="small muted">none</span>;
  const parts: ReactNode[] = [];
  for (const [k, v] of Object.entries(extra)) {
    if (k === "WrapD")
      parts.push(
        <span key={k} className="small">
          a disclosure wrap (<code>wrap_d</code>): one key per tag and round opens the proof at disclosure
        </span>,
      );
    else if (k === "SealingKey" && isObj(v))
      parts.push(
        <span key={k} className={`small ${styles.inline}`}>
          the program&apos;s sealing key <Hash value={strOf(v.pk)} head={12} label="sealing public key" /> epoch <code>{strOf(v.epoch) ?? "—"}</code>
        </span>,
      );
    else
      parts.push(
        <span key={k} className="small">
          <code>{k}</code>
          {isObj(v) && Object.keys(v).length ? ` (${Object.keys(v).join(", ")})` : ""}
        </span>,
      );
  }
  return parts.length ? <span className="stack" style={{ ["--gap" as string]: "4px" }}>{parts}</span> : <span className="small muted">none</span>;
}

export function RunFacts({
  mode,
  a,
  failed,
  tier,
  t2Note,
  requestSha,
  startedAt,
  commitId,
  programId,
  round,
  payer,
  signer,
  program,
  requested,
  answered,
}: {
  mode: string | null;
  a: Answered | null;
  failed: string | null;
  tier: string | null;
  t2Note: ReactNode;
  requestSha: string | null;
  startedAt: number | null;
  commitId: number | null;
  programId: number | null;
  round: number | null;
  payer: string | null;
  signer: string | null;
  program: ProgramDetail | null;
  requested?: IndexedEvent;
  answered?: IndexedEvent;
}) {
  const took = a?.at && startedAt ? a.at - startedAt : null;
  return (
    <div className="card">
      <dl className={`kv ${styles.facts}`}>
        <dt>Mode</dt>
        <dd>
          <code>{mode ?? "—"}</code>
          {mode && MODE_WORDS[mode] ? <div className="small muted">{MODE_WORDS[mode]}</div> : null}
        </dd>
        <dt>Kind</dt>
        <dd>
          {failed ? (
            <span className={styles.inline}>
              <Pill tone="warn">failed</Pill>
              <code className="small">{failed}</code>
            </span>
          ) : a ? (
            <span className={styles.inline}>
              <KindBadge kind={a.kind} />
              {a.reason ? <code className="small">{a.reason}</code> : null}
            </span>
          ) : (
            <Pill tone="pending">in flight</Pill>
          )}
        </dd>
        <dt>Keys</dt>
        <dd>
          {a?.keys.length ? (
            <ul className={styles.keyList}>
              {a.keys.map((k) => {
                const s = slotInfo(program, k.slot);
                return (
                  <li key={`${k.slot}-${k.tag}`}>
                    <span className="small">
                      {slotName(program, k.slot)}
                      {s.severity ? ` · ${s.severity}` : ""}
                      {s.added ? " (added by a round)" : ""}
                    </span>
                    <Hash value={k.tag} head={12} label={`tag of slot ${k.slot}`} />
                  </li>
                );
              })}
            </ul>
          ) : (
            <span className="small muted">{a ? "none: no rule broke" : "—"}</span>
          )}
        </dd>
        <dt>Impact</dt>
        <dd>
          <span className="num">{usd(a?.impact)}</span> <span className={styles.unit}>fixture USD</span>
          <div className="xsmall muted">min(protocol loss, attacker gain), at the manifest&apos;s committed prices, in the fixture&apos;s own dollars.</div>
        </dd>
        <dt>Protected harm</dt>
        <dd>
          <span className="num">{usd(a?.harm)}</span> <span className={styles.unit}>fixture USD</span>
          <div className="xsmall muted">Loss on the accounts the program declared protected: above its threshold, a proof that breaks no rule is a gap claim.</div>
        </dd>
        <dt>Tier</dt>
        <dd>
          <span className={styles.inline}>
            <TierBadge tier={tier} />
            {t2Note}
          </span>
        </dd>
        <dt>Receipt sha256</dt>
        <dd>
          <Hash value={a?.receipt} head={16} label="receipt sha256" />
        </dd>
        <dt>Request sha256</dt>
        <dd>
          <Hash value={requestSha} head={16} label="request sha256" />
          <div className="xsmall muted">The SHA-256 of the input near-antibody sent the TEE: T0 and T1 bind to the same bytes.</div>
        </dd>
        <dt>Output sha256</dt>
        <dd>
          <Hash value={a?.output} head={16} label="output sha256" />
          <div className="xsmall muted">The SHA-256 of the facts near-antibody received: the TDX quote binds it.</div>
        </dd>
        {a ? (
          <>
            <dt>Mode extras</dt>
            <dd>
              <Extra extra={a.extra} />
            </dd>
          </>
        ) : null}
        <dt>Started</dt>
        <dd>
          <span className={styles.inline}>
            <Time t={startedAt} />
            {requested ? <TxLink hash={requested.tx} href={requested.explorer} /> : null}
          </span>
        </dd>
        <dt>Answered</dt>
        <dd>
          <span className={styles.inline}>
            {a?.at ? <Time t={a.at} /> : <span className="muted">not yet</span>}
            {took !== null ? <span className="small muted">{seconds(took, 0)} after it was requested</span> : null}
            {answered ? <TxLink hash={answered.tx} href={answered.explorer} /> : null}
          </span>
        </dd>
        <dt>Commit</dt>
        <dd>{commitId !== null ? <Link href={`/commits/${commitId}`}>Commit {commitId}</Link> : <span className="small muted">none: this run is the program&apos;s own</span>}</dd>
        <dt>Program</dt>
        <dd>
          {programId !== null ? (
            <span className={styles.inline}>
              <Link href={`/programs/${programId}`}>Program {programId}</Link>
              <span className="small muted">round {round ?? "—"}</span>
            </span>
          ) : (
            "—"
          )}
        </dd>
        <dt>Paid for by</dt>
        <dd>
          <Account account={payer} project={strOf(program?.program?.project)} />
        </dd>
        {signer && signer !== payer ? (
          <>
            <dt>Signed by</dt>
            <dd>
              <Account account={signer} project={strOf(program?.program?.project)} />
            </dd>
          </>
        ) : null}
      </dl>
    </div>
  );
}

/** The run as near-antibody stores it, collapsed by default, with a copy button. */
export function RawRun({ id, raw }: { id: number; raw: unknown }) {
  const json = JSON.stringify(raw ?? null, null, 2);
  return (
    <details className={styles.raw}>
      <summary>The run as near-antibody stores it (JSON)</summary>
      <div className={styles.rawBody}>
        <div className={styles.rawCopy}>
          <span className="xsmall muted mono">get_run {"{"}&quot;run_id&quot;: {id}{"}"}</span>
          <CopyButton value={json} label="Copy the raw run JSON" />
        </div>
        <pre className={styles.rawPre} tabIndex={0} aria-label={`Run ${id}, raw JSON`}>
          {json}
        </pre>
      </div>
    </details>
  );
}
