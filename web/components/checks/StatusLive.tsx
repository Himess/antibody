"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import Link from "next/link";
import { Cell } from "@/components/programs/Cell";
import rt from "@/components/programs/programs.module.css";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { Empty, ErrorState, Loading } from "@/components/ui/States";
import { Time } from "@/components/ui/Time";
import { AccountLink, TxLink } from "@/components/ui/TxLink";
import { get, type Result, type Status } from "@/lib/api";
import { chainName } from "@/lib/config";
import { wnear } from "@/lib/format";
import { AlertMessage } from "./AlertMessage";
import styles from "./checks.module.css";

const POLL_MS = 30_000;
/** The API's thresholds (crates/services/src/status.rs): a service is stale after 5 minutes. */
const STALE_S = 300;

type Obj = Record<string, unknown>;
const obj = (v: unknown): Obj => (v && typeof v === "object" && !Array.isArray(v) ? (v as Obj) : {});
const arr = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);
const num = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);
const str = (v: unknown): string | null => (typeof v === "string" ? v : null);
const has = (o: Obj, k: string) => Object.prototype.hasOwnProperty.call(o, k);

const nearAmt = (v: number | null, digits = 3) => (v === null ? "—" : `${v.toLocaleString("en-US", { maximumFractionDigits: digits })} NEAR`);

/** "42 s ago", "6 min ago", "2.5 h ago". */
function ago(s: number): string {
  if (s < 90) return `${Math.round(s)} s ago`;
  if (s < 5400) return `${Math.round(s / 60)} min ago`;
  return `${(s / 3600).toFixed(1)} h ago`;
}

/** An RFC 3339 time as Unix seconds (null when it is not one). */
function unixOf(iso: string | null): number | null {
  if (!iso) return null;
  const ms = Date.parse(iso);
  return Number.isNaN(ms) ? null : Math.floor(ms / 1000);
}

/** A permit id as a number to sort by ("10" after "9"); the API sends them as strings. */
const permitNo = (v: unknown): number => {
  const n = typeof v === "number" ? v : typeof v === "string" ? Number(v) : NaN;
  return Number.isFinite(n) ? n : Number.MAX_SAFE_INTEGER;
};

/** The operating accounts by their role in floats.json. */
const ROLE_NAME: Record<string, string> = { keeper: "the keeper", "t1-relayer": "the T1 relayer", "t2-prover": "the T2 prover" };

function listed(names: string[]): string {
  if (names.length <= 1) return names.join("");
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

const SERVICES: { key: string; what: string }[] = [
  { key: "indexer", what: "Indexes every event of near-antibody, the token and near-guardian, in order." },
  { key: "keeper", what: "Every 30 s: allocates in commit order, pays funded claims, asks for pauses, re-runs at a fix." },
  { key: "relayer", what: "Submits near-guardian's signed pause and release permits to each chain's adapter." },
  { key: "notifier", what: "Posts new events, and each new alert once, to the webhooks." },
  { key: "t1", what: "Relays each answered run's TDX quote to near-antibody, which verifies it on chain." },
];

/** The escrow table's column widths, the same for each group of programs: their tables, stacked, line up. */
const ESCROW_COLS = ["20%", "21%", "20%", "24%", "15%"];

function Level({ level }: { level: string }) {
  if (level === "critical") return <Pill tone="bad">critical</Pill>;
  if (level === "warn") return <Pill tone="warn">warning</Pill>;
  return <Pill tone="info">info</Pill>;
}

function ServiceCard({ name, what, c }: { name: string; what: string; c: Obj | null }) {
  const s = c ? num(c.updated_s_ago) : null;
  let pill = (
    <Pill tone="neutral" size="sm">
      not reported
    </Pill>
  );
  if (c && s === null)
    pill = (
      <Pill tone="neutral" size="sm">
        not run here
      </Pill>
    );
  else if (c && s !== null && s > STALE_S && name !== "relayer")
    pill = (
      <Pill tone="warn" size="sm">
        stale
      </Pill>
    );
  else if (c && s !== null && s > STALE_S)
    pill = (
      <Pill tone="neutral" icon="clock" size="sm" title="The relayer's state changes only when a permit moves">
        quiet
      </Pill>
    );
  else if (c && s !== null)
    pill = (
      <Pill tone="ok" size="sm">
        advancing
      </Pill>
    );
  return (
    <li className={styles.service}>
      <div className="row" style={{ ["--gap" as string]: "8px", justifyContent: "space-between" }}>
        <h3>{name}</h3>
        {pill}
      </div>
      <span className={styles.ago}>{s === null ? "—" : ago(s)}</span>
      <span className="xsmall muted">
        last progress{c && str(c.state_file) ? (
          <>
            , in <code>{str(c.state_file)}</code>
          </>
        ) : null}
      </span>
      <p className="xsmall muted">{what}</p>
    </li>
  );
}

/**
 * `/v1/status` in plain words, refreshed by the browser every 30 s while the tab is visible: the
 * services' progress from their own state files, the T1 relays' collateral and the worker images, the
 * practice faucet, the operating accounts' floats, the permits, each program's escrow and every alert.
 */
export function StatusLive({ initial, runRoles }: { initial: Result<Status>; runRoles: Record<string, string> }) {
  const [current, setCurrent] = useState<Status | null>(initial.ok && initial.data ? initial.data : null);
  const [error, setError] = useState<string | null>(initial.ok ? null : initial.error);
  const [busy, setBusy] = useState(false);
  const lastFetch = useRef(initial.at);
  const inFlight = useRef(false);
  /** When the API read the status the server rendered (Unix seconds): a cached page can be minutes old. */
  const firstRead = useRef(initial.ok && initial.data ? num(initial.data.at) : null);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    try {
      const r = await get<Status>("/v1/status");
      lastFetch.current = r.at;
      if (r.ok && r.data && typeof r.data === "object") {
        setCurrent(r.data);
        setError(null);
      } else {
        setError(r.ok ? "the answer was empty" : r.error);
      }
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    // The server's copy can be minutes old (the page is cached): read again at once if it is older than a poll.
    const read = firstRead.current;
    const first = read === null || Date.now() - read * 1000 > POLL_MS ? window.setTimeout(() => void refresh(), 0) : null;
    const id = window.setInterval(() => {
      if (document.visibilityState === "visible") void refresh();
    }, POLL_MS);
    const onVisible = () => {
      if (document.visibilityState === "visible" && Date.now() - lastFetch.current > POLL_MS) void refresh();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      if (first !== null) window.clearTimeout(first);
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [refresh]);

  if (!current) {
    return (
      <div className="page stack" style={{ ["--gap" as string]: "16px", paddingBottom: 40 }}>
        {error ? (
          <ErrorState what="the status (/v1/status)" error={error}>
            <span className="small">This page asks again every 30 s while it is open.</span>
          </ErrorState>
        ) : (
          <Loading label="Reading the status" />
        )}
        <div>
          <button type="button" className="btn btn-sm" onClick={() => void refresh()} disabled={busy} aria-busy={busy}>
            <Icon name="refresh" size={13} /> {busy ? "Reading…" : "Try again now"}
          </button>
        </div>
      </div>
    );
  }

  const c = obj(current.components);
  const alerts = arr(current.alerts)
    .map(obj)
    .map((a) => ({ level: str(a.level) ?? "info", component: str(a.component) ?? "", key: str(a.key) ?? "", message: str(a.message) ?? "" }));
  const rank = (l: string) => (l === "critical" ? 0 : l === "warn" ? 1 : 2);
  alerts.sort((x, y) => rank(x.level) - rank(y.level));
  const critical = alerts.filter((a) => a.level === "critical").length;
  const warnings = alerts.filter((a) => a.level === "warn").length;
  const notices = alerts.length - critical - warnings;
  const ok = current.ok === true && critical === 0;

  const now = num(current.at);
  const t1 = has(c, "t1") ? obj(c.t1) : null;
  const until = t1 ? str(t1.outlayer_collateral_valid_until) : null;
  const untilUnix = unixOf(until);
  const expired = untilUnix !== null && now !== null ? untilUnix <= now : null;
  const collateral = t1 ? str(t1.collateral) : null;
  // Each platform's copy against the reading's own time: the valid ones first, latest expiry first.
  const windows = (t1 ? arr(t1.outlayer_collateral).map(obj) : [])
    .map((w) => {
      const at = unixOf(str(w.valid_until));
      return { fmspc: str(w.fmspc), until: at, expired: at !== null && now !== null ? at <= now : null };
    })
    .sort((a, b) => Number(a.expired === true) - Number(b.expired === true) || (b.until ?? 0) - (a.until ?? 0));
  const failing = t1 ? arr(t1.failing) : [];

  const images = has(c, "images") ? obj(c.images) : null;
  const pendingImages = images
    ? arr(images.pending)
        .map(obj)
        .sort((a, b) => (num(a.counts_from) ?? 0) - (num(b.counts_from) ?? 0))
    : [];
  const unlisted = images ? num(images.unlisted) : null;
  const delay = images ? num(images.delay_secs) : null;

  const practice = has(c, "practice") ? obj(c.practice) : null;
  const balance = practice ? num(practice.balance_near) : null;
  const allowance = practice ? num(practice.public_vend_key_allowance_near) : null;
  const credit = practice ? num(practice.credit_near) : null;

  const floats = arr(c.floats).map(obj);
  const floatRoles = floats.map((f) => str(f.role) ?? "");
  const floatNames = floatRoles.map((r) => ROLE_NAME[r] ?? r).filter(Boolean);
  // Permits in number order (the API sends their ids as strings): pending first, then applied.
  const byPermit = (a: Obj, b: Obj) => permitNo(a.permit) - permitNo(b.permit) || (num(a.chain_id) ?? 0) - (num(b.chain_id) ?? 0);
  const pending = arr(c.permits_pending).map(obj).sort(byPermit);
  const applied = arr(c.permits_applied).map(obj).sort(byPermit);
  // This run's programs first (Acme Lend, practice, fork mode), then any opened after it, then earlier runs', each by id.
  const programs = arr(c.programs)
    .map(obj)
    .sort((a, b) => (num(a.program_id) ?? 0) - (num(b.program_id) ?? 0));
  const roleOf = (p: Obj) => {
    const id = num(p.program_id);
    return id !== null ? runRoles[String(id)] : undefined;
  };
  const runIds = Object.keys(runRoles).map(Number).filter(Number.isFinite);
  const firstRunId = runIds.length ? Math.min(...runIds) : Number.POSITIVE_INFINITY;
  const later = (p: Obj) => !roleOf(p) && (num(p.program_id) ?? -1) > firstRunId;
  const programGroups = [
    { title: "This run's programs", note: "", rows: programs.filter((p) => roleOf(p)) },
    { title: "Opened after the recorded run", note: "opened after the recorded run", rows: programs.filter(later) },
    { title: "Earlier runs on the same deployment", note: "an earlier run", rows: programs.filter((p) => !roleOf(p) && !later(p)) },
  ].filter((g) => g.rows.length);
  const prover = has(c, "t2_prover") ? obj(c.t2_prover) : null;

  // The daily drill (docs/drill.md) and the EVM accounts: generic components the API added, each shown here.
  const drill = has(c, "drill") ? obj(c.drill) : null;
  const drillId = str(drill?.latest)?.replace(/\.json$/, "") ?? null;
  const drillSteps = drill ? Object.entries(obj(drill.steps)).map(([k, v]) => ({ step: k, ok: obj(v).ok === true })) : [];
  const drillCost = drill ? obj(drill.cost) : null;
  const drillNear = drillCost ? arr(drillCost.near).map(obj) : [];
  const evmBalances = arr(c.evm_balances)
    .map(obj)
    .sort((a, b) => (str(a.role) ?? "").localeCompare(str(b.role) ?? "") || (num(a.chain_id) ?? 0) - (num(b.chain_id) ?? 0));

  return (
    <>
      <div className="page" style={{ paddingBottom: 8 }}>
        <div className={styles.overall}>
          <h2 className="visually-hidden">Overall</h2>
          <div className={styles.overallTop}>
            <div className="row" style={{ ["--gap" as string]: "12px" }} aria-live="polite">
              <Pill tone={ok ? "ok" : "bad"}>{ok ? "ok" : "critical"}</Pill>
              <span className={styles.overallWord}>
                {ok ? "No critical alert" : critical ? `${critical} critical alert${critical === 1 ? "" : "s"}` : "Not ok"}
              </span>
              {warnings ? (
                <a href="#alerts" className={styles.pillLink}>
                  <Pill tone="warn">
                    {warnings} warning{warnings === 1 ? "" : "s"}
                  </Pill>
                </a>
              ) : (
                <span className="small muted">no warnings</span>
              )}
              {notices ? (
                <a href="#alerts" className={styles.pillLink}>
                  <Pill tone="info">
                    {notices} notice{notices === 1 ? "" : "s"}
                  </Pill>
                </a>
              ) : null}
            </div>
            <div className={styles.refresh}>
              <span className="small muted">
                Read <Time t={num(current.at)} />
              </span>
              <button type="button" className="btn btn-sm" onClick={() => void refresh()} disabled={busy} aria-busy={busy}>
                <Icon name="refresh" size={13} /> {busy ? "Refreshing…" : "Refresh now"}
              </button>
            </div>
          </div>
          <p className="small muted">
            NEAR {current.network ?? "testnet"}. Refreshed by your browser every 30 s while this tab is open; the same JSON is at{" "}
            <code>/v1/status</code>.
          </p>
          {error ? (
            <p className="small" role="status">
              <Pill tone="warn" size="sm">
                could not refresh
              </Pill>{" "}
              <span className="mono xsmall">{error}</span> · showing the last reading.
            </p>
          ) : null}
        </div>
      </div>

      <Section id="alerts" title="Alerts" lede="Every alert the services raise now, most serious first. Each also goes to the notifier's webhooks once.">
        {alerts.length ? (
          <ul className={styles.alerts}>
            {alerts.map((a) => (
              <li
                key={`${a.component}-${a.key}-${a.message}`}
                className={`${styles.alert} ${a.level === "critical" ? styles.alertCritical : a.level === "warn" ? styles.alertWarn : ""}`}
              >
                <div>
                  <Level level={a.level} />
                </div>
                <div className={styles.alertText}>
                  <span>
                    <AlertMessage text={a.message} />
                  </span>
                  <span className="xsmall muted mono">
                    {a.component} · {a.key}
                  </span>
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <Empty>No alerts.</Empty>
        )}
      </Section>

      <Section id="services" title="Services" lede="Each service's last progress, from its own state file. None of them is trusted: every value they show is re-derivable from NEAR.">
        <ul className={styles.services} style={{ listStyle: "none", margin: 0, padding: 0 }}>
          {SERVICES.map((s) => (
            <ServiceCard key={s.key} name={s.key} what={s.what} c={has(c, s.key) ? obj(c[s.key]) : null} />
          ))}
        </ul>
      </Section>

      <Section id="t1" title="T1 relays and worker images" lede="What a run needs to become T1: DCAP collateral that verifies its quote, and a worker image its program counts.">
        <div className={styles.grid2}>
          <div className={styles.cardBox}>
            <h3>The collateral the T1 relayer uses</h3>
            {t1 ? (
              <dl className={styles.mini}>
                <div>
                  <dt>Source</dt>
                  <dd>
                    <code>{collateral ?? "—"}</code>{" "}
                    <span className="xsmall muted">
                      {collateral === "auto"
                        ? "OutLayer's on-chain copies while they verify, then fresh Intel PCS collateral"
                        : collateral === "outlayer"
                          ? "OutLayer's on-chain copies only"
                          : collateral === "pcs"
                            ? "fresh Intel PCS collateral"
                            : ""}
                    </span>
                  </dd>
                </div>
                {windows.length ? (
                  <div>
                    <dt>OutLayer&apos;s copies, per platform</dt>
                    <dd>
                      <ul className={styles.items}>
                        {windows.map((w, i) => (
                          <li key={`${w.fmspc}-${i}`}>
                            <Icon name={w.expired ? "alert" : "clock"} size={13} />
                            <span className="stack" style={{ ["--gap" as string]: "4px", alignItems: "flex-start" }}>
                              <span>
                                FMSPC <code>{w.fmspc ?? "—"}</code>
                              </span>
                              <span className="row" style={{ ["--gap" as string]: "6px" }}>
                                {w.expired === true ? (
                                  <Pill tone={collateral === "outlayer" ? "bad" : "info"} size="sm">
                                    expired
                                  </Pill>
                                ) : w.expired === false ? (
                                  <Pill tone="ok" size="sm">
                                    valid
                                  </Pill>
                                ) : null}
                                <span className="xsmall">
                                  {w.expired ? "since" : "until"} <Time t={w.until} />
                                </span>
                              </span>
                              {w.expired ? (
                                <span className="xsmall muted">
                                  {collateral === "outlayer"
                                    ? "The T1 relayer uses only OutLayer's copies."
                                    : "T1 relays for this platform use fresh Intel PCS collateral."}
                                </span>
                              ) : null}
                            </span>
                          </li>
                        ))}
                      </ul>
                    </dd>
                  </div>
                ) : (
                  <div>
                    <dt>OutLayer&apos;s copies valid until</dt>
                    <dd className="stack" style={{ ["--gap" as string]: "4px", alignItems: "flex-start" }}>
                      {untilUnix !== null ? <Time t={untilUnix} /> : <span className="muted">—</span>}
                      {expired === true ? (
                        <span className="small">
                          <Pill tone={collateral === "outlayer" ? "bad" : "info"} size="sm">
                            expired
                          </Pill>{" "}
                          {collateral === "outlayer" ? "and the relayer uses only those copies" : "T1 relays use fresh Intel PCS collateral"}
                        </span>
                      ) : expired === false ? (
                        <Pill tone="ok" size="sm">
                          valid
                        </Pill>
                      ) : null}
                    </dd>
                  </div>
                )}
                <div>
                  <dt>Runs it could not confirm</dt>
                  <dd>
                    {failing.length ? (
                      <ul className={styles.items}>
                        {failing.map((f, i) => (
                          <li key={i}>
                            <Icon name="alert" size={13} />
                            <span className="mono xsmall">{typeof f === "number" || typeof f === "string" ? `run ${f}` : JSON.stringify(f)}</span>
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <span className="small">none</span>
                    )}
                  </dd>
                </div>
              </dl>
            ) : (
              <Empty>The T1 relayer does not run with this API.</Empty>
            )}
          </div>
          <div className={styles.cardBox}>
            <h3>OutLayer&apos;s worker images</h3>
            {images ? (
              <>
                <dl className={styles.mini}>
                  <div>
                    <dt>Registry</dt>
                    <dd>
                      <AccountLink account={str(images.registry)} site={false} />
                    </dd>
                  </div>
                  <div>
                    <dt>Approved, listed</dt>
                    <dd className="num">
                      {num(images.approved) ?? "—"} approved by OutLayer · {num(images.listed) ?? "—"} listed by near-antibody
                    </dd>
                  </div>
                  <div>
                    <dt>Unlisted</dt>
                    <dd>
                      {unlisted === null ? (
                        "—"
                      ) : unlisted > 0 ? (
                        <Pill tone="bad" size="sm">
                          {unlisted} unlisted
                        </Pill>
                      ) : (
                        <Pill tone="ok" size="sm">
                          none
                        </Pill>
                      )}
                    </dd>
                  </div>
                  <div>
                    <dt>Image delay</dt>
                    <dd className="small">
                      {delay === null ? "—" : `${Math.round(delay / 3600)} h`}: a newly listed image counts for existing programs only after it
                    </dd>
                  </div>
                </dl>
                {pendingImages.length ? (
                  <div className="stack" style={{ ["--gap" as string]: "8px" }}>
                    <p className="small">
                      <Pill tone="warn" size="sm">
                        {pendingImages.length} pending
                      </Pill>{" "}
                      A run on one of these gets no T1 before its time; its payout waits the challenge window.
                    </p>
                    <ul className={styles.items}>
                      {pendingImages.map((p, i) => (
                        <li key={`${str(p.rtmr3)}-${i}`}>
                          <Icon name="clock" size={13} />
                          <span className="stack" style={{ ["--gap" as string]: "2px" }}>
                            <span>
                              counts from <Time t={num(p.counts_from)} />
                            </span>
                            <span className="xsmall muted">
                              RTMR3 <Hash value={str(p.rtmr3)} head={12} label="RTMR3" /> · listed <Time t={num(p.listed_at)} showRelative={false} />
                            </span>
                          </span>
                        </li>
                      ))}
                    </ul>
                  </div>
                ) : (
                  <p className="small">No listed image is waiting out the delay.</p>
                )}
              </>
            ) : (
              <Empty>The API did not report the worker images.</Empty>
            )}
          </div>
        </div>
      </Section>

      <Section
        id="drill"
        title="The daily drill"
        lede="The automated drill runs the one moment again on a fresh Acme Lend instance each day. The recorded run stays the hero; a drill shows the system still works today."
      >
        {drill ? (
          drillId ? (
            <div className={styles.cardBox}>
              <div className="row" style={{ ["--gap" as string]: "10px", justifyContent: "space-between" }}>
                <h3>
                  <Link href={`/moment?run=${encodeURIComponent(drillId)}`}>{drillId}</Link>
                </h3>
                {drill.skipped === true ? (
                  <Pill tone="warn" size="sm">
                    skipped
                  </Pill>
                ) : drill.ok === true ? (
                  <Pill tone="ok" size="sm">
                    all checks passed
                  </Pill>
                ) : (
                  <Pill tone="bad" size="sm">
                    did not pass
                  </Pill>
                )}
              </div>
              <dl className={styles.mini}>
                <div>
                  <dt>Ran</dt>
                  <dd>
                    {num(drill.finished_unix) ? <Time t={num(drill.finished_unix)} /> : "—"}
                    {num(drill.age_s) !== null ? <span className="xsmall muted"> · {ago(num(drill.age_s) as number)}</span> : null}
                  </dd>
                </div>
                {drill.skipped === true ? (
                  <div>
                    <dt>Why skipped</dt>
                    <dd className="small">{str(drill.skipped_reason) ?? "—"}</dd>
                  </div>
                ) : null}
                {drillSteps.length ? (
                  <div>
                    <dt>Steps</dt>
                    <dd className="row" style={{ ["--gap" as string]: "6px", flexWrap: "wrap" }}>
                      {drillSteps.map((s) => (
                        <Pill key={s.step} tone={s.ok ? "ok" : "bad"} size="sm">
                          {s.step}
                        </Pill>
                      ))}
                    </dd>
                  </div>
                ) : null}
                {obj(drill.failed_step).step ? (
                  <div>
                    <dt>Stopped at</dt>
                    <dd className="small">
                      <code>{str(obj(drill.failed_step).step)}</code>
                      {str(obj(drill.failed_step).error) ? <span className="xsmall muted"> · {str(obj(drill.failed_step).error)}</span> : null}
                    </dd>
                  </div>
                ) : null}
                {drillNear.length ? (
                  <div>
                    <dt>Cost (NEAR, this drill)</dt>
                    <dd className="small">
                      {drillNear
                        .filter((r) => str(r.spent_near) && str(r.spent_near) !== "0.000000000000000000000000")
                        .map((r) => `${str(r.role)} ${str(r.spent_near)}`)
                        .join(" · ") || "—"}
                    </dd>
                  </div>
                ) : null}
              </dl>
            </div>
          ) : (
            <Empty>No drill has run yet. The daily drill writes drill-YYYY-MM-DD.json into the results.</Empty>
          )
        ) : (
          <Empty>The API did not report a drill.</Empty>
        )}
      </Section>

      <Section id="balances" title="Balances that keep it running" lede="The practice faucet, watched against draining, and the NEAR each operating account keeps for its gas.">
        <div className={styles.grid2}>
          <div className={styles.cardBox}>
            <h3>The practice faucet</h3>
            {practice ? (
              <dl className={styles.mini}>
                <div>
                  <dt>Vending contract</dt>
                  <dd>
                    <AccountLink account={str(practice.vending)} />
                    {num(practice.program) !== null ? (
                      <div className="xsmall">
                        vends for <Link href={`/programs/${num(practice.program)}`}>program {num(practice.program)}</Link>
                      </div>
                    ) : null}
                  </dd>
                </div>
                <div>
                  <dt>Balance</dt>
                  <dd>
                    <span className="num">{nearAmt(balance)}</span>{" "}
                    {balance === null ? null : balance < 0.5 ? (
                      <Pill tone="bad" size="sm">
                        below 0.5 NEAR
                      </Pill>
                    ) : (
                      <Pill tone="ok" size="sm">
                        above 0.5 NEAR
                      </Pill>
                    )}
                  </dd>
                </div>
                <div>
                  <dt>Public vend key allowance</dt>
                  <dd>
                    {allowance === null ? (
                      <Pill tone="bad" size="sm">
                        the key is gone
                      </Pill>
                    ) : (
                      <>
                        <span className="num">{nearAmt(allowance, 4)}</span>{" "}
                        {allowance < 0.05 ? (
                          <Pill tone="warn" size="sm">
                            below 0.05 NEAR
                          </Pill>
                        ) : (
                          <Pill tone="ok" size="sm">
                            enough
                          </Pill>
                        )}
                      </>
                    )}
                    <div className="xsmall muted">the allowance caps what calls to vend can burn</div>
                  </dd>
                </div>
                <div>
                  <dt>Its credit at near-antibody</dt>
                  <dd>
                    <span className="num">{nearAmt(credit)}</span>
                    {str(practice.credit_tokens_raw) ? <span className="xsmall muted"> · {wnear(str(practice.credit_tokens_raw))}</span> : null}{" "}
                    {credit !== null && credit < 0.1 ? (
                      <Pill tone="warn" size="sm">
                        below 0.1 NEAR
                      </Pill>
                    ) : null}
                  </dd>
                </div>
              </dl>
            ) : (
              <Empty>The API did not report the practice faucet.</Empty>
            )}
          </div>
          <div className={styles.cardBox}>
            <h3>Operating accounts&apos; floats</h3>
            {floats.length ? (
              <div className={`table-wrap ${rt.cardsWrap} ${rt.inCard}`}>
                <table className={`table ${rt.rtable}`}>
                  <caption className="visually-hidden">Each operating account&apos;s liquid NEAR against its float</caption>
                  <thead>
                    <tr>
                      <th scope="col">Account</th>
                      <th scope="col">Liquid</th>
                      <th scope="col">Float</th>
                      <th scope="col">State</th>
                    </tr>
                  </thead>
                  <tbody>
                    {floats.map((f, i) => {
                      const liquid = num(f.liquid_near);
                      const min = num(f.float_near);
                      return (
                        <tr key={`${str(f.account)}-${i}`}>
                          <Cell label="Account" lead className={styles.cell}>
                            <strong className="small">{str(f.role) ?? "—"}</strong>
                            <div className="mono xsmall">{str(f.account) ?? "—"}</div>
                          </Cell>
                          <Cell label="Liquid" className="num">
                            {nearAmt(liquid)}
                          </Cell>
                          <Cell label="Float" className="num">
                            {nearAmt(min, 2)}
                          </Cell>
                          <Cell label="State">
                            {liquid === null || min === null ? (
                              "—"
                            ) : liquid >= min ? (
                              <Pill tone="ok" size="sm">
                                above
                              </Pill>
                            ) : liquid >= min / 4 ? (
                              <Pill tone="warn" size="sm">
                                under
                              </Pill>
                            ) : (
                              <Pill tone="bad" size="sm">
                                under a quarter
                              </Pill>
                            )}
                          </Cell>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            ) : (
              <Empty>No operating account reports a float here.</Empty>
            )}
            <p className="xsmall muted">
              {floatNames.length ? `${listed(floatNames).replace(/^t/, "T")} keep${floatNames.length === 1 ? "s" : ""} NEAR liquid for gas` : "Each operating account keeps NEAR liquid for gas"}
              {floatRoles.includes("t2-prover") ? "" : " (the T2 prover too, once near-t2 is on testnet)"}: under the float is a warning, under a quarter
              of it is critical.
            </p>
          </div>
        </div>
      </Section>

      <Section
        id="permits"
        title="Permits"
        lede="Pause and release permits near-guardian signed, by their state on each chain. One counts as applied when a recorded run relayed it or its adapter reports it used; the others are pending."
      >
        {pending.length || applied.length ? (
          <div className="stack" style={{ ["--gap" as string]: "12px" }}>
            <p className="row" style={{ ["--gap" as string]: "8px" }}>
              <Pill tone={pending.length ? "pending" : "neutral"} size="sm">
                {pending.length} pending
              </Pill>
              <Pill tone="ok" size="sm">
                {applied.length} applied
              </Pill>
            </p>
            <div className={`table-wrap ${rt.cardsWrap}`}>
              <table className={`table ${rt.rtable}`}>
                <caption className="visually-hidden">Each permit near-guardian signed, pending first, then applied, in number order</caption>
                <thead>
                  <tr>
                    <th scope="col">Permit</th>
                    <th scope="col">Chain</th>
                    <th scope="col">State</th>
                    <th scope="col">Evidence</th>
                  </tr>
                </thead>
                <tbody>
                  {pending.map((p, i) => {
                    const chain = num(p.chain_id);
                    return (
                      <tr key={`pending-${str(p.permit) ?? i}-${chain}`}>
                        <Cell label="Permit" lead className="num">
                          <strong>Permit {str(p.permit) ?? String(num(p.permit) ?? "—")}</strong>
                        </Cell>
                        <Cell label="Chain">{chain !== null ? chainName(chain) : "—"}</Cell>
                        <Cell label="State">
                          <Pill tone="pending" size="sm">
                            pending
                          </Pill>
                        </Cell>
                        <Cell label="Evidence" className={styles.cell}>
                          <span className="xsmall muted">relayer&apos;s state</span> <code>{str(p.status) ?? "—"}</code>
                        </Cell>
                      </tr>
                    );
                  })}
                  {applied.map((p, i) => {
                    const chain = num(p.chain_id);
                    return (
                      <tr key={`applied-${str(p.permit) ?? i}-${chain}`}>
                        <Cell label="Permit" lead className="num">
                          <strong>Permit {str(p.permit) ?? String(num(p.permit) ?? "—")}</strong>
                        </Cell>
                        <Cell label="Chain">{chain !== null ? chainName(chain) : "—"}</Cell>
                        <Cell label="State">
                          <Pill tone="ok" size="sm">
                            applied
                          </Pill>
                        </Cell>
                        <Cell label="Evidence" className={styles.cell}>
                          <code>{str(p.status) ?? "applied"}</code>{" "}
                          {str(p.tx) && chain !== null ? <TxLink hash={str(p.tx)} chainId={chain} /> : null}
                          {str(p.source) ? <div className="xsmall muted mono">{str(p.source)}</div> : null}
                        </Cell>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </div>
        ) : (
          <Empty>The API lists no permit.</Empty>
        )}
      </Section>

      {evmBalances.length ? (
        <Section
          id="evm"
          title="EVM balances"
          lede="The permit relayer and the drill deployer on each chain, against their floors. The relayer submits the pause permits; the deployer deploys each drill's fresh fixture."
        >
          <div className={`table-wrap ${rt.cardsWrap}`}>
            <table className={`table ${rt.rtable}`}>
              <caption className="visually-hidden">Each EVM account&apos;s balance on each chain against its floor</caption>
              <thead>
                <tr>
                  <th scope="col">Account</th>
                  <th scope="col">Chain</th>
                  <th scope="col">Balance</th>
                  <th scope="col">Floor</th>
                  <th scope="col">State</th>
                </tr>
              </thead>
              <tbody>
                {evmBalances.map((e, i) => {
                  const chain = num(e.chain_id);
                  const eth = str(e.balance_eth);
                  const floor = str(e.floor_eth);
                  const ok = e.ok === true;
                  const unread = has(e, "error") && e.balance_wei === null;
                  return (
                    <tr key={`${str(e.role)}-${chain}-${i}`}>
                      <Cell label="Account" lead className={styles.cell}>
                        <strong className="small">{str(e.role) ?? "—"}</strong>
                        <div className="mono xsmall">{str(e.address) ?? "—"}</div>
                      </Cell>
                      <Cell label="Chain">{chain !== null ? chainName(chain) : "—"}</Cell>
                      <Cell label="Balance" className="num">
                        {unread ? "—" : eth ? `${Number(eth).toLocaleString("en-US", { maximumFractionDigits: 6 })} ETH` : "—"}
                      </Cell>
                      <Cell label="Floor" className="num">
                        {floor ? `${Number(floor).toLocaleString("en-US", { maximumFractionDigits: 6 })} ETH` : "—"}
                      </Cell>
                      <Cell label="State">
                        {unread ? (
                          <Pill tone="neutral" size="sm">
                            not read
                          </Pill>
                        ) : ok ? (
                          <Pill tone="ok" size="sm">
                            above
                          </Pill>
                        ) : (
                          <Pill tone="bad" size="sm">
                            below floor
                          </Pill>
                        )}
                      </Cell>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
          <p className="xsmall muted" style={{ marginTop: 10 }}>
            Amounts on Sepolia and Base Sepolia, testnets with no value.
          </p>
        </Section>
      ) : null}

      <Section id="escrow" title="Escrow" lede="Each program's available escrow against its largest payout: an open program below it raises a warning.">
        {programs.length ? (
          <div className="stack" style={{ ["--gap" as string]: "18px" }}>
            {programGroups.map((g) => (
                <div key={g.title} className="stack" style={{ ["--gap" as string]: "10px" }}>
                  <h3 className={styles.subhead}>{g.title}</h3>
                  <div className={`table-wrap ${rt.cardsWrap}`}>
                    <table className={`table ${rt.rtable} ${rt.fixedCols}`}>
                      <caption className="visually-hidden">{g.title}: available escrow against the largest payout</caption>
                      <colgroup>
                        {ESCROW_COLS.map((w, i) => (
                          <col key={i} style={{ width: w }} />
                        ))}
                      </colgroup>
                      <thead>
                        <tr>
                          <th scope="col">Program</th>
                          <th scope="col">Phase, round</th>
                          <th scope="col">Available</th>
                          <th scope="col">Largest payout</th>
                          <th scope="col">Covered</th>
                        </tr>
                      </thead>
                      <tbody>
                        {g.rows.map((p, i) => {
                          const id = num(p.program_id);
                          const available = str(p.available_raw);
                          const largest = str(p.largest_payout_raw);
                          const covered = available !== null && largest !== null ? Number(available) >= Number(largest) : null;
                          const role = roleOf(p);
                          return (
                            <tr key={id ?? i}>
                              <Cell label="Program" lead className={styles.cell}>
                                {id !== null ? <Link href={`/programs/${id}`}>Program {id}</Link> : "—"}
                                <div className="xsmall muted">{role ? `this run: ${role}` : g.note}</div>
                              </Cell>
                              <Cell label="Phase, round">
                                <code>{str(p.phase) ?? "—"}</code>, round {num(p.round) ?? "—"}
                              </Cell>
                              <Cell label="Available" className="num">
                                {wnear(available)}
                              </Cell>
                              <Cell label="Largest payout" className="num">
                                {wnear(largest)}
                              </Cell>
                              <Cell label="Covered">
                                {covered === null ? (
                                  "—"
                                ) : covered ? (
                                  <Pill tone="ok" size="sm">
                                    yes
                                  </Pill>
                                ) : (
                                  <Pill tone="warn" size="sm">
                                    below
                                  </Pill>
                                )}
                              </Cell>
                            </tr>
                          );
                        })}
                      </tbody>
                    </table>
                  </div>
                </div>
              ))}
          </div>
        ) : (
          <Empty>The API listed no programs.</Empty>
        )}
        <p className="xsmall muted" style={{ marginTop: 10 }}>
          Amounts in wNEAR on testnet, which has no value.
        </p>
      </Section>

      {prover ? (
        <Section id="prover" title="The T2 prover" lede="The service that proves practice tries on SP1's prover network and submits them to near-t2.">
          <dl className={styles.mini} style={{ maxWidth: 720 }}>
            <div>
              <dt>Runs proven and submitted</dt>
              <dd className="num">{num(prover.proven) ?? "—"}</dd>
            </div>
            <div>
              <dt>PROVE balance</dt>
              <dd>
                <span className="num">{String(obj(prover.balance).prove ?? "—")}</span>{" "}
                {obj(prover.balance).low === true ? (
                  <Pill tone="warn" size="sm">
                    low: proofs pause
                  </Pill>
                ) : null}
              </dd>
            </div>
            <div>
              <dt>Budget</dt>
              <dd className="small">
                {String(obj(prover.budget).last_hour ?? "—")} in the last hour, {String(obj(prover.budget).last_day ?? "—")} today
                {obj(prover.budget).capped === true ? " (reached)" : ""}
              </dd>
            </div>
          </dl>
        </Section>
      ) : null}
    </>
  );
}
