import Link from "next/link";
import { Suspense } from "react";
import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { Empty, ErrorState, Loading } from "@/components/ui/States";
import { TxLink } from "@/components/ui/TxLink";
import type { ProgramDetail, Reflex, ReflexPermit } from "@/lib/api";
import { chainName, explorers, sourceUrl } from "@/lib/config";
import { Cell } from "./Cell";
import { getReflex } from "./data";
import { normHex, type ProgramKind } from "./helpers";
import { RELEASE, type RecordedPermit } from "./recorded";
import styles from "./programs.module.css";

type Chain = Reflex["chains"][number];

/** Address (lowercase, no 0x) → the target's name in the manifest. */
export function targetNames(d: ProgramDetail): Map<string, string> {
  const m = new Map<string, string>();
  for (const t of Array.isArray(d.meta?.targets) ? d.meta.targets : []) {
    if (Array.isArray(t) && typeof t[0] === "string" && typeof t[1] === "string") m.set(normHex(t[1]), t[0]);
  }
  return m;
}

export function ChainState({ state }: { state: string }) {
  if (state === "paused")
    return (
      <Pill tone="warn" icon="pause" size="sm">
        paused
      </Pill>
    );
  if (state === "released")
    return (
      <Pill tone="ok" icon="check" size="sm">
        released: running the fix
      </Pill>
    );
  return (
    <Pill tone="neutral" icon="play" size="sm">
      live: not paused
    </Pill>
  );
}

export function PermitStatus({ status }: { status: string | null | undefined }) {
  if (status === "confirmed" || status === "applied")
    return (
      <Pill tone="ok" size="sm">
        {status} on chain
      </Pill>
    );
  if (status === "signed")
    return (
      <Pill tone="info" size="sm">
        signed, not yet seen on chain
      </Pill>
    );
  if (status === "failed" || status === "skipped")
    return (
      <Pill tone="bad" size="sm">
        {status}
      </Pill>
    );
  return (
    <Pill tone="pending" size="sm">
      {status ?? "queued"}
    </Pill>
  );
}

/** What a permit does, in words: a pause for a ticket, or a release of one proxy to one code hash. */
export function PermitWhat({ p, names }: { p: ReflexPermit; names: Map<string, string> }) {
  if (p.body?.Pause)
    return (
      <span className="small">
        Pause, for the ticket of <Link href={`/commits/${p.body.Pause.ticket}`}>commit {p.body.Pause.ticket}</Link>
      </span>
    );
  if (p.body?.Release) {
    const r = p.body.Release;
    return (
      <span className="small stack" style={{ ["--gap" as string]: "2px" }}>
        <span>
          Release <span className="mono">{names.get(normHex(r.proxy)) ?? "proxy"}</span> to code hash
        </span>
        <Hash value={r.code_hash} head={12} label="code hash" />
      </span>
    );
  }
  return <span className="small muted">—</span>;
}

export function PermitTx({ p, chainId, recorded }: { p: ReflexPermit; chainId: number; recorded?: RecordedPermit }) {
  if (!p.tx && recorded?.tx_hash)
    return (
      <span className="stack" style={{ ["--gap" as string]: "2px" }}>
        <TxLink hash={recorded.tx_hash} chainId={chainId} href={recorded.explorer} head={10} />
        <a href={sourceUrl(RELEASE.results_file)} target="_blank" rel="noreferrer noopener" className="xsmall muted tap">
          recorded run <Icon name="external" size={10} className={styles.inlineIcon} />
        </a>
      </span>
    );
  // Applied on chain (the adapter says so) with no recorded relay transaction: say exactly that.
  if (!p.tx && (p.status === "applied" || p.status === "confirmed"))
    return (
      <span className="stack" style={{ ["--gap" as string]: "2px" }}>
        <span className="small">relay transaction not recorded</span>
        <span className="xsmall muted">applied, per {p.source || "the adapter"}</span>
      </span>
    );
  if (!p.tx) return <span className="small muted">not on chain yet</span>;
  return (
    <span className="stack" style={{ ["--gap" as string]: "2px" }}>
      <TxLink hash={p.tx} chainId={chainId} href={p.explorer} head={10} />
      {p.source ? (
        <a href={sourceUrl(p.source)} target="_blank" rel="noreferrer noopener" className="xsmall muted tap">
          recorded run <Icon name="external" size={10} className={styles.inlineIcon} />
        </a>
      ) : null}
    </span>
  );
}

function ChainCard({ c, names, recorded }: { c: Chain; names: Map<string, string>; recorded: RecordedPermit[] }) {
  const permits = c.permits ?? [];
  const recordedOf = (p: ReflexPermit) => (!p.tx ? recorded.find((x) => x.permit_id === p.permit_id && x.chain_id === c.chain_id) : undefined);
  return (
    <div className="card stack" style={{ ["--gap" as string]: "12px" }}>
      <div className={styles.roundHead}>
        <h3 className={styles.cardTitle}>
          {chainName(c.chain_id)} <span className="xsmall muted num">({c.chain_id})</span>
        </h3>
        <ChainState state={c.state} />
      </div>
      <dl className={`kv ${styles.kvStack}`}>
        <dt>Adapter</dt>
        <dd>
          <Hash value={c.adapter} head={10} tail={4} href={explorers.evmAddress(c.chain_id, c.adapter)} label={`${chainName(c.chain_id)} adapter`} />
        </dd>
        <dt>Proxies</dt>
        <dd className="stack" style={{ ["--gap" as string]: "4px" }}>
          {(c.proxies ?? []).length
            ? c.proxies.map((x) => (
                <span key={x} className={styles.keyRow}>
                  <span className="mono small">{names.get(normHex(x)) ?? "proxy"}</span>
                  <Hash value={x} head={10} tail={4} href={explorers.evmAddress(c.chain_id, x)} label={`${names.get(normHex(x)) ?? "proxy"} on ${chainName(c.chain_id)}`} />
                </span>
              ))
            : "—"}
        </dd>
      </dl>
      {permits.length ? (
        <div className={`table-wrap ${styles.cardsWrap} ${styles.inCard}`}>
          <table className={`table ${styles.rtable}`}>
            <caption className="visually-hidden">Permits on {chainName(c.chain_id)}</caption>
            <thead>
              <tr>
                <th scope="col">Permit</th>
                <th scope="col">What it does</th>
                <th scope="col">Status</th>
                <th scope="col">Transaction</th>
              </tr>
            </thead>
            <tbody>
              {permits.map((p) => (
                <tr key={p.permit_id}>
                  <Cell label="Permit" lead>
                    <span className="num">#{p.permit_id}</span>
                  </Cell>
                  <Cell label="What">
                    <PermitWhat p={p} names={names} />
                    <span className={`${styles.sub} num`}>round {p.round}</span>
                  </Cell>
                  <Cell label="Status">
                    <PermitStatus status={recordedOf(p)?.status ?? p.status} />
                  </Cell>
                  <Cell label="Transaction">
                    <PermitTx p={p} chainId={c.chain_id} recorded={recordedOf(p)} />
                  </Cell>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="small muted">No permit for this chain yet.</p>
      )}
    </div>
  );
}

async function ChainsBody({ d, id, kind, moment }: { d: ProgramDetail; id: number; kind: ProgramKind; moment: boolean }) {
  const r = await getReflex(id);
  if (!r.ok)
    return (
      <div className="stack">
        <ErrorState what="the chains (the reflex panel)" error={r.error}>
          <span className="small">
            The API reads each chain&apos;s adapter live, which can be slow; this page retries on its next load. The same permits,
            from a terminal:
          </span>
        </ErrorState>
        <Command label="CLI command">{`antibody judge --program ${id}`}</Command>
      </div>
    );
  const chains = r.data?.chains ?? [];
  if (!chains.length)
    return (
      <Empty>
        {kind === "practice" || kind === "fork"
          ? "No chains: a practice program pauses nothing and ships no fix. Its proofs are verified and paid; no permit is ever asked for."
          : kind === "window"
            ? "No chains: a pull request's window is a fork-mode program, so it pauses nothing and ships no fix. Its proofs are verified and paid; no permit is ever asked for."
            : "No chain registered: this program asks for no pause or release permit."}
      </Empty>
    );
  const names = targetNames(d);
  const recorded = id === RELEASE.program_id ? RELEASE.release.permits : [];
  return (
    <div className="stack">
      {r.data.guardian_address ? (
        <p className="small">
          Permits are EIP-712 messages the MPC network signs under this program&apos;s own path; each adapter trusts only its
          address, <Hash value={r.data.guardian_address} head={10} tail={4} label="the program's guardian address" />. The adapter can
          pause and upgrade; it can never unpause.
        </p>
      ) : null}
      {/* One chain per row: each permit table keeps its transaction column in view. */}
      <div className="stack">
        {chains.map((c) => (
          <ChainCard key={c.chain_id} c={c} names={names} recorded={recorded} />
        ))}
      </div>
      {moment ? (
        <p className="small muted">
          The pause permits are the one moment&apos;s: A&apos;s Critical paused both chains. <Link href="/moment">Every step, timed</Link>.
        </p>
      ) : null}
      <Command label="CLI command">{`antibody judge --program ${id}`}</Command>
    </div>
  );
}

/** 6. Where the program acts: each chain's adapter and proxies, live, paused or released, with every permit. */
export function ChainsSection({ d, id, kind, moment }: { d: ProgramDetail; id: number; kind: ProgramKind; moment: boolean }) {
  return (
    <Section
      id="chains"
      title="The chains"
      lede="Per chain: the program's adapter, the proxies it may pause or upgrade, and every permit near-guardian asked the MPC network to sign, with the transaction that carried it there."
    >
      <Suspense fallback={<Loading label="Reading the chains" />}>
        <ChainsBody d={d} id={id} kind={kind} moment={moment} />
      </Suspense>
    </Section>
  );
}
