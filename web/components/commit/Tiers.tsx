import type { ReactNode } from "react";
import { TierBadge } from "@/components/ui/Badges";
import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { Pill } from "@/components/ui/Pill";
import { Empty, ErrorState } from "@/components/ui/States";
import { TxLink } from "@/components/ui/TxLink";
import type { IndexedEvent } from "@/lib/api";
import { chainName, sourceUrl } from "@/lib/config";
import { RequestIdFinder } from "./RequestIdFinder";
import { evData, isObj, kindWords, numOf, strOf, type T2Find } from "./model";
import styles from "./commit.module.css";

/** What a T2 proof's public values bind (docs/zk.md, "near-t2"), as near-antibody records them. */
export interface Bound {
  requestSha: string | null;
  runId: number | null;
  round: number | null;
  commitId: number | null;
  blobSha: string | null;
  baseSha: string | null;
  manifestSha: string | null;
  anchor: { chainId: number | null; block: number | null; hash: string | null } | null;
  programKey: string | null;
}

export interface TierInput {
  runId: number | null;
  /** The TEE tier near-antibody records (T0 or T1). */
  tier: string | null;
  kind: string | null;
  receipt: string | null;
  /** The run failed instead of answering (its reason). */
  failed?: string | null;
  answered?: IndexedEvent;
  t1?: IndexedEvent;
  requested?: IndexedEvent;
  /** Who signed the requesting transaction (the RPC needs it to find the transaction). */
  signer: string | null;
  contract: string;
  wasmSha: string | null;
  t2: T2Find;
  bound: Bound;
}

function Row({
  badge,
  title,
  reached,
  state,
  children,
  checks,
}: {
  badge: ReactNode;
  title: string;
  reached: boolean;
  state: ReactNode;
  children: ReactNode;
  checks: ReactNode;
}) {
  return (
    <article className={`card ${styles.tierRow} ${reached ? "" : styles.notReached}`}>
      <div className={styles.tierAbout}>
        <div className={`row ${styles.tierState}`}>
          {badge}
          {state}
        </div>
        <h3>{title}</h3>
        {children}
      </div>
      <div className={styles.checks}>{checks}</div>
    </article>
  );
}

export function Tiers(x: TierInput) {
  if (x.runId === null) {
    return <Empty>No verification run yet: the tiers apply once the proof is revealed and the TEE answers.</Empty>;
  }
  const t0 = Boolean(x.answered) || Boolean(x.tier);
  const t1 = x.tier === "T1" || x.tier === "T2" || Boolean(x.t1);
  const requestSha = strOf(evData(x.requested).request_sha256) ?? x.bound.requestSha;
  return (
    <div className={styles.tiers}>
      <Row
        badge={<TierBadge tier="T0" />}
        title="The TEE's answer"
        reached={t0}
        state={
          x.failed ? (
            <Pill tone="warn" size="sm">
              run failed
            </Pill>
          ) : t0 ? (
            <Pill tone="ok" size="sm">
              reached
            </Pill>
          ) : (
            <Pill tone="pending" size="sm">
              waiting for the TEE
            </Pill>
          )
        }
        checks={
          <>
            {x.answered ? (
              <p className="small">
                Delivered in <TxLink hash={x.answered.tx} href={x.answered.explorer} />
              </p>
            ) : null}
            <Command label="Read the run">{`antibody status --run ${x.runId}`}</Command>
          </>
        }
      >
        {x.failed ? (
          <p className="small muted">
            The run failed: <code>{x.failed}</code>. A submission&apos;s run that fails for a reason that is not the hunter&apos;s is retried, with its
            priority kept.
          </p>
        ) : (
          <p className="small muted">
            {t0 ? "OutLayer's operator delivered the TEE worker's output to near-antibody" : "OutLayer's operator delivers the TEE worker's output to near-antibody"}
            {x.kind ? `: ${kindWords(x.kind)}` : ""}
            {x.receipt ? (
              <>
                , receipt <Hash value={x.receipt} head={12} label="receipt sha256" />
              </>
            ) : null}
            . On its own, a T0 verdict settles a submission only after its challenge window.
          </p>
        )}
      </Row>

      <Row
        badge={<TierBadge tier="T1" />}
        title="The TDX quote, verified on NEAR"
        reached={t1}
        state={
          t1 ? (
            <Pill tone="ok" size="sm">
              reached
            </Pill>
          ) : (
            <Pill tone="pending" size="sm">
              not confirmed
            </Pill>
          )
        }
        checks={
          <>
            {x.t1 ? (
              <p className="small">
                Confirmed in <TxLink hash={x.t1.tx} href={x.t1.explorer} />
              </p>
            ) : null}
            {x.requested && x.signer ? (
              <RequestIdFinder tx={x.requested.tx} signer={x.signer} requestSha={requestSha} blobSha={x.bound.blobSha} wasmSha={x.wasmSha} />
            ) : (
              <Command label="T1 check">{"antibody tee check --request-id <the run's OutLayer request>"}</Command>
            )}
            {x.requested ? <Command label="OutLayer's own check from the transaction">{`outlayer-verify tx ${x.requested.tx}`}</Command> : null}
            <LiveCheck
              check={{ kind: "near-view", contract: x.contract, method: "get_run", args: { run_id: x.runId } }}
              expect={
                t1 || x.tier
                  ? {
                      path: "outcome.Answered.tier",
                      equals: t1 ? "T1" : (x.tier ?? "T0"),
                      says: t1 ? "the run's answer is T1: its quote was verified on NEAR" : `the run's answer is ${x.tier ?? "T0"}`,
                    }
                  : undefined
              }
              label="Read the run from NEAR"
            />
          </>
        }
      >
        <p className="small muted">
          {t1
            ? "OutLayer's attestation names the pinned verifier wasm and the sha256 of the request near-antibody sent and of the output it received; NEAR's DCAP verifier accepted the run's Intel TDX quote over it. Instant payouts need T1."
            : x.answered
              ? "Not T1-confirmed: the verdict stands on its T0 answer and its challenge window. The T1 relayer sends each answered run's quote, and anyone may."
              : "Once the TEE answers, the T1 relayer (or anyone) relays the run's quote to near-antibody, which verifies it on chain."}
        </p>
      </Row>

      <Row
        badge={<TierBadge tier="T2" />}
        title="A validity proof of the disclosed verdict"
        reached={Boolean(x.t2.record && x.t2.record.outcome === "confirmed")}
        state={<T2State t2={x.t2} />}
        checks={<T2Checks x={x} />}
      >
        <T2About t2={x.t2} />
      </Row>
    </div>
  );
}

function T2State({ t2 }: { t2: T2Find }) {
  if (t2.record?.outcome === "confirmed")
    return (
      <Pill tone="t2" size="sm">
        confirmed on NEAR
      </Pill>
    );
  if (t2.record)
    return (
      <Pill tone="bad" size="sm">
        contradicts
      </Pill>
    );
  if (t2.notYet)
    return (
      <Pill tone="pending" size="sm">
        not yet on testnet
      </Pill>
    );
  if (t2.error)
    return (
      <Pill tone="warn" size="sm">
        status unavailable
      </Pill>
    );
  return (
    <Pill tone="pending" size="sm">
      no record yet
    </Pill>
  );
}

function T2About({ t2 }: { t2: T2Find }) {
  const r = t2.record;
  if (r?.outcome === "confirmed")
    return (
      <p className="small muted">
        near-t2 verified an SP1 proof that the verifier, re-run on the TEE&apos;s exact input, gives the TEE&apos;s receipt, kind and slots, and compared every
        bound value with near-antibody&apos;s records. It claims no privacy (the proof it re-runs is disclosed), and no payout depends on it.
      </p>
    );
  if (r) {
    const differ = Array.isArray(r.differ) ? r.differ.join(", ") : "";
    return (
      <p className="small muted">
        near-t2 recorded a contradiction{differ ? ` (${differ})` : ""}: the same input re-ran to another result. In the locked deployment it reverses
        nothing; consumers can act on it.
      </p>
    );
  }
  if (t2.notYet)
    return (
      <p className="small muted">
        T2 is not yet on testnet. Once near-t2 is deployed, an SP1 proof that the verifier, re-run on this run&apos;s exact input, gives the same receipt is
        checked on NEAR against near-antibody&apos;s records, after the proof is disclosed. No privacy claim; no payout depends on it.
      </p>
    );
  if (t2.error) return <ErrorState what="near-t2's records" error={t2.error} />;
  return (
    <p className="small muted">
      near-t2 has no record of this run yet. A T2 proof re-runs the disclosed proof, so it follows disclosure; practice tries are proven by the T2 prover
      service, since their content key is public.
    </p>
  );
}

function T2Checks({ x }: { x: TierInput }) {
  const r = x.t2.record;
  const docs = (
    <p className="xsmall muted">
      <a href={sourceUrl("docs/zk.md")} target="_blank" rel="noreferrer noopener">
        How T2 works (docs/zk.md)
      </a>
    </p>
  );
  if (!r) {
    return (
      <>
        <Command label="T2 status">{`antibody zk status --run ${x.runId}`}</Command>
        {docs}
      </>
    );
  }
  const slotsOf = (v: unknown) =>
    Array.isArray(v) ? v.map((s) => (isObj(s) ? `${numOf(s.slot) ?? "?"}${strOf(s.severity) ? ` ${s.severity}` : ""}` : String(s))).join(", ") : "";
  const same = (a: unknown, b: unknown): boolean | null => {
    const sa = strOf(a);
    const sb = strOf(b);
    return sa && sb ? sa.toLowerCase() === sb.toLowerCase() : null;
  };
  const b = x.bound;
  const anchor = b.anchor;
  const rows: { field: string; value: ReactNode; check: ReactNode }[] = [
    {
      field: "Request sha256",
      value: <Hash value={strOf(r.request_sha256)} head={12} label="the proof's request sha256" />,
      check: <Eq v={same(r.request_sha256, b.requestSha)} what="the run's" />,
    },
    {
      field: "Run · round · commit",
      value: `${numOf(r.run_id) ?? "—"} · ${numOf(r.round) ?? "—"} · ${numOf(r.commit_id) ?? "—"}`,
      check: (
        <Eq
          v={numOf(r.run_id) === b.runId && (b.round === null || numOf(r.round) === b.round) && (b.commitId === null || numOf(r.commit_id) === b.commitId)}
          what="near-antibody's"
        />
      ),
    },
    { field: "Blob sha256", value: <Hash value={b.blobSha} head={12} label="blob sha256" />, check: <Pill size="sm">bound</Pill> },
    { field: "Base sha256", value: <Hash value={b.baseSha} head={12} label="base witness sha256" />, check: <Pill size="sm">bound</Pill> },
    { field: "Manifest sha256", value: <Hash value={b.manifestSha} head={12} label="manifest sha256" />, check: <Pill size="sm">bound</Pill> },
    {
      field: "Anchor",
      value: anchor ? `block ${anchor.block ?? "—"}${anchor.chainId ? ` on ${chainName(anchor.chainId)}` : ""}` : "—",
      check: <Pill size="sm">bound</Pill>,
    },
    {
      field: "Program key · verifier",
      value: (
        <>
          {b.programKey ?? "—"}
          {x.t2.verifier?.version ? ` · ${x.t2.verifier.version}` : ""}
          {x.t2.verifier?.wasm ? (
            <>
              {" "}
              <Hash value={x.t2.verifier.wasm} head={10} label="the verifier wasm sha256 near-t2 pins" />
            </>
          ) : null}
        </>
      ),
      check: <Pill size="sm">bound</Pill>,
    },
    {
      field: "Receipt sha256",
      value: <Hash value={strOf(r.receipt_sha256)} head={12} label="the proof's receipt sha256" />,
      check: <Eq v={same(r.receipt_sha256, r.tee_receipt_sha256)} what="the TEE's" />,
    },
    { field: "Kind", value: strOf(r.kind) ?? "—", check: <Eq v={same(r.kind, r.tee_kind)} what="the TEE's" /> },
    {
      field: "Slots (severity)",
      value: slotsOf(r.slots) || "none",
      check: (
        <Eq
          v={
            Array.isArray(r.slots) && Array.isArray(r.tee_slots)
              ? JSON.stringify(r.slots.map((s) => (isObj(s) ? numOf(s.slot) : null))) === JSON.stringify(r.tee_slots.map((s) => numOf(s)))
              : null
          }
          what="the TEE's"
        />
      ),
    },
  ];
  return (
    <>
      {x.t2.event ? (
        <p className="small">
          Recorded on NEAR in <TxLink hash={x.t2.event.tx} href={x.t2.event.explorer} />
        </p>
      ) : null}
      <p className={styles.checksTitle}>What its public values bind</p>
      <dl className={styles.bound}>
        {rows.map((row) => (
          <div key={row.field} className={styles.boundRow}>
            <dt>{row.field}</dt>
            <dd>{row.value}</dd>
            <dd>{row.check}</dd>
          </div>
        ))}
      </dl>
      <p className="xsmall muted">
        A different request, run, blob, base, manifest, anchor, program, verifier or severity is rejected outright; a different receipt, kind or slots is
        recorded as a contradiction.
      </p>
      <Command label="T2 status">{`antibody zk status --run ${x.runId}`}</Command>
      {x.t2.contract ? (
        <LiveCheck
          check={{ kind: "near-view", contract: x.t2.contract, method: "get_record", args: { run_id: String(x.runId) } }}
          expect={{ path: "outcome", equals: String(r.outcome), says: `near-t2's record says ${String(r.outcome)}` }}
          label="Read near-t2's record"
        />
      ) : null}
      {docs}
    </>
  );
}

function Eq({ v, what }: { v: boolean | null; what: string }) {
  if (v === null) return <Pill size="sm">bound</Pill>;
  return v ? (
    <Pill tone="ok" size="sm">
      equals {what}
    </Pill>
  ) : (
    <Pill tone="bad" size="sm">
      differs from {what}
    </Pill>
  );
}
