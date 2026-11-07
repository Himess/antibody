import Link from "next/link";
import type { ReactNode } from "react";
import { KindBadge, TierBadge } from "@/components/ui/Badges";
import { CopyButton } from "@/components/ui/CopyButton";
import { Hash } from "@/components/ui/Hash";
import { Pill } from "@/components/ui/Pill";
import { Time } from "@/components/ui/Time";
import { AccountLink, TxLink } from "@/components/ui/TxLink";
import { VerdictStamp } from "@/components/ui/Verdict";
import type { IndexedEvent, ProgramDetail } from "@/lib/api";
import { seconds } from "@/lib/format";
import { isObj, isVerdictMode, MODE_WORDS, numOf, slotInfo, slotName, strOf, topSeverity, usd, who, type Answered } from "./model";
import styles from "./commit.module.css";

function Account({ account, project, program }: { account: string | null; project: string | null; program: number | null }) {
  if (!account) return <span className="muted">—</span>;
  const w = project && account === project ? "the program's project" : who(account, program);
  return (
    <span className={styles.inline}>
      <AccountLink account={account} />
      {w ? <span className="small muted">{w}</span> : null}
    </span>
  );
}

/** A verify run's mode-specific `extra`: a disclosure wrap, or none. */
function Extra({ extra }: { extra: unknown }): ReactNode {
  if (!isObj(extra)) return <span className="small muted">none</span>;
  const parts: ReactNode[] = [];
  for (const [k, v] of Object.entries(extra)) {
    if (k === "WrapD")
      parts.push(
        <span key={k} className="small">
          a disclosure wrap (<code>wrap_d</code>): if its tag and round are disclosed, their key opens the proof
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

/** What a run that judges no proof produced (spec §5.2: `sealing_pk`, `round_check`, `wrap`, `disclosed`, `distinct`). */
function Produced({ mode, extra, project }: { mode: string | null; extra: unknown; project: string | null }): ReactNode {
  const x = isObj(extra) ? extra : {};
  const wrap = isObj(x.Wrap) ? strOf(x.Wrap.wrap) : null;
  if (mode === "release_wrap" && wrap)
    return (
      <span className="stack" style={{ ["--gap" as string]: "4px" }}>
        <span className="small">
          The proof&apos;s content key, wrapped to the project&apos;s key{project ? <> ({project})</> : null}: {wrap.length / 2} bytes{" "}
          <Hash value={wrap} head={16} label="the release wrap" />
        </span>
        <span className="xsmall muted">
          Only the project&apos;s key opens it: <code>antibody program read</code> unwraps the content key, opens the proof and re-runs it locally.
        </span>
      </span>
    );
  const sk = isObj(x.SealingKey) ? x.SealingKey : null;
  if (mode === "activate" && sk)
    return (
      <span className={`small ${styles.inline}`}>
        The program&apos;s sealing public key <Hash value={strOf(sk.pk)} head={16} label="sealing public key" /> · key epoch <code>{strOf(sk.epoch) ?? "—"}</code>
      </span>
    );
  const rc = isObj(x.RoundCheck) ? x.RoundCheck : null;
  if (mode === "check_round" && rc)
    return (
      <span className="stack" style={{ ["--gap" as string]: "4px" }}>
        <span className={styles.inline}>
          <Pill tone={rc.ok === true ? "ok" : "bad"} size="sm">
            {rc.ok === true ? "valid round" : "round refused"}
          </Pill>
          <Pill tone={rc.canaries_ok === true ? "ok" : "bad"} size="sm">
            {rc.canaries_ok === true ? "canaries succeed" : "canaries fail"}
          </Pill>
        </span>
        <span className="xsmall muted">
          The TEE checked the proposed fix at the anchor: only target code changed, every rule kept, <code>check()</code> holds, the canaries still succeed,
          and each release replaces a proxy&apos;s implementation.
        </span>
      </span>
    );
  const disclosed = isObj(x.Disclosed) && Array.isArray(x.Disclosed.keys) ? x.Disclosed.keys : null;
  if (mode === "disclose" && disclosed)
    return (
      <ul className={styles.keyList}>
        {disclosed.map((d, i) =>
          Array.isArray(d) ? (
            <li key={i}>
              <span className="small">round {numOf(d[0]) ?? "—"}, tag</span>
              <Hash value={strOf(d[1])} head={12} label="disclosed tag" />
              <span className="small">key</span>
              <Hash value={strOf(d[2])} head={12} label="disclosure key" />
            </li>
          ) : null,
        )}
      </ul>
    );
  if (mode === "distinct" && isObj(x.Distinct))
    return <span className="small">{x.Distinct.distinct === true ? "Distinct: the patch stops the earlier proof but not this one." : "Not distinct."}</span>;
  return <Extra extra={extra} />;
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
  const verdict = isVerdictMode(mode);
  // A run that judges no proof answers with a fixed NoViolation: its kind is shown only when it says more.
  const kindShown = verdict || Boolean(failed) || (a ? a.kind !== "NoViolation" : true);
  const project = strOf(program?.program?.project);
  const severity = a ? topSeverity(program, a.keys) : null;
  return (
    <div className={`card ${styles.summary}`}>
      <dl className={`kv ${styles.facts}`}>
        <dt>Mode</dt>
        <dd>
          <code>{mode ?? "—"}</code>
          {mode && MODE_WORDS[mode] ? <div className="small muted">{MODE_WORDS[mode]}</div> : null}
        </dd>
        {kindShown ? (
          <>
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
          </>
        ) : null}
        {verdict ? (
          <>
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
                <span className="small muted">{a ? (mode === "rerun" ? "none: at this round's code it breaks no rule" : "none: no rule broke") : "—"}</span>
              )}
            </dd>
            {mode === "rerun" ? (
              <>
                <dt>Impact</dt>
                <dd>
                  <span className="small muted">not published for a re-run</span>
                  <div className="xsmall muted">
                    A re-run runs the project&apos;s fix against a proof it may not have paid for: its facts name only the keys that still break, never
                    the impact or the harm.
                  </div>
                </dd>
              </>
            ) : (
              <>
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
              </>
            )}
          </>
        ) : (
          <>
            <dt>What it produced</dt>
            <dd>
              {a ? <Produced mode={mode} extra={a.extra} project={project} /> : <span className="muted">{failed ? "nothing: the run failed" : "not yet"}</span>}
              <div className="xsmall muted">Not a verdict: this run does not judge a proof, so it has no kind, keys or impact of its own.</div>
            </dd>
          </>
        )}
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
        {a && verdict ? (
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
          <Account account={payer} project={project} program={programId} />
        </dd>
        {signer && signer !== payer ? (
          <>
            <dt>Signed by</dt>
            <dd>
              <Account account={signer} project={project} program={programId} />
            </dd>
          </>
        ) : null}
      </dl>
      {verdict && a ? (
        <div className={styles.stampCol}>
          <VerdictStamp kind={a.kind} slots={a.keys.map((k) => k.slot)} severity={severity ?? undefined} />
          {a.receipt ? (
            <span className={`xsmall muted ${styles.inline}`}>
              receipt <Hash value={a.receipt} head={12} label="receipt sha256" />
            </span>
          ) : null}
        </div>
      ) : null}
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
