"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import Link from "next/link";
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

const SERVICES: { key: string; what: string }[] = [
  { key: "indexer", what: "Indexes every event of near-antibody, the token and near-guardian, in order." },
  { key: "keeper", what: "Every 30 s: allocates in commit order, pays funded claims, asks for pauses, re-runs at a fix." },
  { key: "relayer", what: "Submits near-guardian's signed pause and release permits to each chain's adapter." },
  { key: "notifier", what: "Posts new events, and each new alert once, to the webhooks." },
  { key: "t1", what: "Relays each answered run's TDX quote to near-antibody, which verifies it on chain." },
];

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
    const id = window.setInterval(() => {
      if (document.visibilityState === "visible") void refresh();
    }, POLL_MS);
    const onVisible = () => {
      if (document.visibilityState === "visible" && Date.now() - lastFetch.current > POLL_MS) void refresh();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
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

  const t1 = has(c, "t1") ? obj(c.t1) : null;
  const until = t1 ? str(t1.outlayer_collateral_valid_until) : null;
  const untilUnix = unixOf(until);
  const expired = untilUnix !== null && num(current.at) !== null ? untilUnix <= (num(current.at) as number) : null;
  const collateral = t1 ? str(t1.collateral) : null;
  const windows = t1 ? arr(t1.outlayer_collateral).map(obj) : [];
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
  const pending = arr(c.permits_pending).map(obj);
  const applied = arr(c.permits_applied).map(obj);
  const programs = arr(c.programs).map(obj);
  const prover = has(c, "t2_prover") ? obj(c.t2_prover) : null;

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
              <span className="small muted">
                {warnings} warning{warnings === 1 ? "" : "s"}
                {notices ? `, ${notices} notice${notices === 1 ? "" : "s"}` : ""}
              </span>
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
                  <span>{a.message}</span>
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
                <div>
                  <dt>OutLayer&apos;s copies valid until</dt>
                  <dd className="stack" style={{ ["--gap" as string]: "4px" }}>
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
                {windows.length ? (
                  <div>
                    <dt>Per platform</dt>
                    <dd>
                      <ul className={styles.items}>
                        {windows.map((w, i) => (
                          <li key={`${str(w.fmspc)}-${i}`}>
                            <Icon name="clock" size={13} />
                            <span>
                              FMSPC <code>{str(w.fmspc) ?? "—"}</code>: valid until {str(w.valid_until)?.replace("T", " ").replace("Z", " UTC") ?? "—"}
                            </span>
                          </li>
                        ))}
                      </ul>
                    </dd>
                  </div>
                ) : null}
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
                      <AccountLink account={str(images.registry)} />
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
              <div className="table-wrap">
                <table className="table">
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
                          <td className={styles.cell}>
                            <strong className="small">{str(f.role) ?? "—"}</strong>
                            <div className="mono xsmall">{str(f.account) ?? "—"}</div>
                          </td>
                          <td className="num">{nearAmt(liquid)}</td>
                          <td className="num">{nearAmt(min, 2)}</td>
                          <td>
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
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            ) : (
              <Empty>No operating account reports a float here.</Empty>
            )}
            <p className="xsmall muted">The keeper, the T1 relayer and the T2 prover keep NEAR liquid for gas: under the float is a warning, under a quarter of it is critical.</p>
          </div>
        </div>
      </Section>

      <Section
        id="permits"
        title="Permits"
        lede="Pause and release permits near-guardian signed, by their state on each chain. One counts as applied when a recorded run relayed it or its adapter reports it used; the others are pending."
      >
        <div className={styles.grid2}>
          <div className="stack" style={{ ["--gap" as string]: "10px" }}>
            <h3 className={styles.subhead}>
              Pending <span className="muted num">({pending.length})</span>
            </h3>
            {pending.length ? (
              <div className="table-wrap">
                <table className="table">
                  <thead>
                    <tr>
                      <th scope="col">Permit</th>
                      <th scope="col">Chain</th>
                      <th scope="col">Relayer&apos;s state</th>
                    </tr>
                  </thead>
                  <tbody>
                    {pending.map((p, i) => (
                      <tr key={`${str(p.permit) ?? i}-${num(p.chain_id)}`}>
                        <td className="num">{str(p.permit) ?? String(num(p.permit) ?? "—")}</td>
                        <td>{num(p.chain_id) !== null ? chainName(num(p.chain_id) as number) : "—"}</td>
                        <td>
                          <code>{str(p.status) ?? "—"}</code>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <Empty>No permit is waiting for its chain.</Empty>
            )}
          </div>
          <div className="stack" style={{ ["--gap" as string]: "10px" }}>
            <h3 className={styles.subhead}>
              Applied <span className="muted num">({applied.length})</span>
            </h3>
            {applied.length ? (
              <div className="table-wrap">
                <table className="table">
                  <thead>
                    <tr>
                      <th scope="col">Permit</th>
                      <th scope="col">Chain</th>
                      <th scope="col">Evidence</th>
                    </tr>
                  </thead>
                  <tbody>
                    {applied.map((p, i) => {
                      const chain = num(p.chain_id);
                      return (
                        <tr key={`${str(p.permit) ?? i}-${chain}`}>
                          <td className="num">{str(p.permit) ?? String(num(p.permit) ?? "—")}</td>
                          <td>{chain !== null ? chainName(chain) : "—"}</td>
                          <td className={styles.cell}>
                            <code>{str(p.status) ?? "applied"}</code>{" "}
                            {str(p.tx) && chain !== null ? <TxLink hash={str(p.tx)} chainId={chain} /> : null}
                            {str(p.source) ? <div className="xsmall muted mono">{str(p.source)}</div> : null}
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            ) : (
              <Empty>None listed as applied.</Empty>
            )}
          </div>
        </div>
      </Section>

      <Section id="escrow" title="Escrow" lede="Each program's available escrow against its largest payout: an open program below it raises a warning.">
        {programs.length ? (
          <div className="table-wrap">
            <table className="table">
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
                {programs.map((p, i) => {
                  const id = num(p.program_id);
                  const available = str(p.available_raw);
                  const largest = str(p.largest_payout_raw);
                  const covered = available !== null && largest !== null ? Number(available) >= Number(largest) : null;
                  const role = id !== null ? runRoles[String(id)] : undefined;
                  return (
                    <tr key={id ?? i}>
                      <td className={styles.cell}>
                        {id !== null ? <Link href={`/programs/${id}`}>Program {id}</Link> : "—"}
                        <div className="xsmall muted">{role ? `this run: ${role}` : "an earlier run"}</div>
                      </td>
                      <td>
                        <code>{str(p.phase) ?? "—"}</code>, round {num(p.round) ?? "—"}
                      </td>
                      <td className="num">{wnear(available)}</td>
                      <td className="num">{wnear(largest)}</td>
                      <td>
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
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
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
