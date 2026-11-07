import Link from "next/link";
import { Suspense } from "react";
import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { Empty, ErrorState, Loading } from "@/components/ui/States";
import { Time } from "@/components/ui/Time";
import { TxLink } from "@/components/ui/TxLink";
import type { FeedRow, ProgramDetail, Round } from "@/lib/api";
import { chainName, explorers, sourceUrl } from "@/lib/config";
import { Cell } from "./Cell";
import { PermitStatus, PermitTx, targetNames } from "./ChainsSection";
import { getReflex } from "./data";
import { findEvent, normHex, proposalOf, rowsWithTag, slotName, whoLabel, type SlotView } from "./helpers";
import { LocationView } from "./Location";
import { RELEASE } from "./recorded";
import styles from "./programs.module.css";

function GatePill({ holds }: { holds: boolean | undefined }) {
  if (typeof holds !== "boolean")
    return (
      <Pill tone="pending" size="sm">
        release gate not read
      </Pill>
    );
  return holds ? (
    <Pill tone="ok" icon="check">
      release gate holds
    </Pill>
  ) : (
    <Pill tone="warn" icon="lock">
      release gate shut
    </Pill>
  );
}

/** A key (a tag) in words: its rule, and the commits that proved it, first one first. */
function TagItem({ tag, rows, slots }: { tag: string; rows: FeedRow[]; slots: SlotView[] }) {
  const holders = rowsWithTag(rows, tag);
  const slot = holders[0]?.verdict?.keys?.find((k) => k.tag === tag)?.slot;
  return (
    <li>
      <span className={styles.keyRow}>
        <span className="mono xsmall">{slot !== undefined ? slotName(slots, slot) : "a key"}</span>
        <Hash value={tag} head={8} label="tag" />
      </span>
      {holders.length ? (
        <span className="small">
          {" "}
          {holders.map((h, i) => (
            <span key={h.commit_id}>
              {i === 0 ? "proven by " : i === 1 ? "; duplicate " : ", "}
              <Link href={`/commits/${h.commit_id}`}>commit {h.commit_id}</Link>
              {whoLabel(h.hunter) ? ` (${whoLabel(h.hunter)})` : ""}
            </span>
          ))}
        </span>
      ) : null}
    </li>
  );
}

/** The release permits a round's gate produced, from the reflex panel; the recorded run fills a transaction the API lacks. */
async function RoundPermits({ d, id, round }: { d: ProgramDetail; id: number; round: number }) {
  const r = await getReflex(id);
  if (!r.ok) return <ErrorState what="the release permits" error={r.error} />;
  const names = targetNames(d);
  const items = (r.data?.chains ?? []).flatMap((c) => (c.permits ?? []).filter((p) => p.body?.Release && p.round === round).map((p) => ({ c, p })));
  if (!items.length) return <p className="small muted">The reflex panel lists no release permit for this round yet.</p>;
  const recorded = id === RELEASE.program_id ? RELEASE.release.permits : [];
  return (
    <div className={`table-wrap ${styles.cardsWrap}`}>
      <table className={`table ${styles.rtable}`}>
        <caption className="visually-hidden">Release permits of round {round}</caption>
        <thead>
          <tr>
            <th scope="col">Permit</th>
            <th scope="col">Chain</th>
            <th scope="col">Upgrades</th>
            <th scope="col">Status</th>
            <th scope="col">Transaction</th>
          </tr>
        </thead>
        <tbody>
          {items.map(({ c, p }) => {
            const rec = !p.tx ? recorded.find((x) => x.permit_id === p.permit_id && x.chain_id === c.chain_id && x.tx_hash) : undefined;
            const rel = p.body?.Release;
            if (!rel) return null;
            return (
              <tr key={`${c.chain_id}-${p.permit_id}`}>
                <Cell label="Permit" lead>
                  <span className="num">#{p.permit_id}</span>
                </Cell>
                <Cell label="Chain">
                  <span className="small">{chainName(c.chain_id)}</span>
                </Cell>
                <Cell label="Upgrades">
                  <span className="stack" style={{ ["--gap" as string]: "2px" }}>
                    <span className="mono small">{names.get(normHex(rel.proxy)) ?? "proxy"}</span>
                    <span className="xsmall muted">
                      to code hash <Hash value={rel.code_hash} head={10} label="code hash" />
                    </span>
                  </span>
                </Cell>
                <Cell label="Status">
                  <PermitStatus status={rec ? rec.status : p.status} />
                </Cell>
                <Cell label="Transaction">
                  <PermitTx p={p} chainId={c.chain_id} recorded={rec} />
                </Cell>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function RoundCard({ r, index, d, id, rows, slots, antibody }: { r: Round; index: number; d: ProgramDetail; id: number; rows: FeedRow[]; slots: SlotView[]; antibody: string }) {
  const proposed = findEvent(d, "round_proposed", (x) => x.round === r.round);
  const opened = findEvent(d, "round_opened", (x) => x.round === r.round);
  const checkRun = typeof proposed?.data?.run_id === "number" ? (proposed.data.run_id as number) : null;
  const manifest = (d.meta?.round_manifests ?? []).find((m) => m.sha256 === r.manifest_sha256) ?? d.meta?.round_manifests?.[r.round - 1];
  const releases = d.meta?.round_releases?.[r.round - 1] ?? [];
  const names = targetNames(d);
  const chainId = d.meta?.chain_id ?? d.rules?.anchor?.chain_id;
  const released = r.released ?? [];
  const carried = r.carried ?? [];
  const promoted = r.promoted ?? [];

  return (
    <article className="card stack" aria-labelledby={`round-${r.round}-title`}>
      <div className={styles.roundHead}>
        <h3 id={`round-${r.round}-title`} className={styles.cardTitle}>
          Round {r.round}
        </h3>
        <GatePill holds={r.release_gate} />
      </div>

      <div className={styles.split}>
        <div className="stack" style={{ ["--gap" as string]: "12px" }}>
          <h4 className={styles.subhead}>In plain words</h4>
          <ul className={styles.plain}>
            <li>
              <Icon name="check" size={14} />
              <div className="stack" style={{ ["--gap" as string]: "4px" }}>
                <span className="small">
                  <strong>Fixed by this round&apos;s code</strong> ({released.length} {released.length === 1 ? "key" : "keys"} released)
                </span>
                {released.length ? (
                  <ul className={styles.tagList}>
                    {released.map((t) => (
                      <TagItem key={t} tag={t} rows={rows} slots={slots} />
                    ))}
                  </ul>
                ) : (
                  <span className="small muted">none</span>
                )}
              </div>
            </li>
            <li>
              <Icon name="alert" size={14} />
              <div className="stack" style={{ ["--gap" as string]: "4px" }}>
                <span className="small">
                  <strong>Still breaks</strong>
                  {carried.length ? ` (${carried.length} ${carried.length === 1 ? "key" : "keys"} carried to the same holder)` : ""}
                </span>
                {carried.length ? (
                  <ul className={styles.tagList}>
                    {carried.map((t) => (
                      <TagItem key={t} tag={t} rows={rows} slots={slots} />
                    ))}
                  </ul>
                ) : (
                  <span className="small muted">nothing: every paid proof fails at this code</span>
                )}
              </div>
            </li>
            <li>
              <Icon name="arrow" size={14} />
              <span className="small">
                <strong>Promoted</strong>:{" "}
                {promoted.length ? (
                  promoted.map((c, i) => {
                    const row = rows.find((x) => x.commit_id === c);
                    return (
                      <span key={c}>
                        {i ? ", " : ""}
                        <Link href={`/commits/${c}`}>commit {c}</Link>
                        {row && whoLabel(row.hunter) ? ` (${whoLabel(row.hunter)})` : ""}
                      </span>
                    );
                  })
                ) : (
                  <span className="muted">none</span>
                )}
                {promoted.length ? <span className="muted">: a duplicate or gap claim whose re-run broke a rule at this code, paid at its original priority</span> : null}
              </span>
            </li>
            <li>
              <Icon name={r.release_gate === true ? "check" : r.release_gate === false ? "lock" : "info"} size={14} />
              <span className="small">
                {r.release_gate === true
                  ? "Every paid proof was re-run at this fix and fails: near-antibody asked for a release permit per proxy and chain."
                  : r.release_gate !== false
                    ? "The API did not say whether the gate held; the check below reads it on NEAR."
                    : carried.length
                      ? "A paid proof still breaks a rule at this fix, so no release permit was asked for: this code does not ship."
                      : "A condition of the gate failed, so no release permit was asked for: this code does not ship."}
              </span>
            </li>
          </ul>
        </div>

        <dl className={`kv ${styles.kvStack}`}>
          <dt>Proposed</dt>
          <dd className="small">
            {proposed ? <TxLink hash={proposed.tx} href={proposed.explorer} /> : <span className="muted">—</span>}
            {checkRun !== null ? (
              <>
                {" "}
                · the TEE checked the fix in <Link href={`/runs/${checkRun}`}>run {checkRun}</Link>
              </>
            ) : null}
          </dd>
          <dt>Opened</dt>
          <dd className="stack" style={{ ["--gap" as string]: "2px" }}>
            <Time t={r.opened_at} />
            {opened ? <TxLink hash={opened.tx} href={opened.explorer} /> : null}
          </dd>
          <dt>Round manifest</dt>
          <dd className="stack" style={{ ["--gap" as string]: "4px" }}>
            <Hash value={r.manifest_sha256} head={16} label={`round ${r.round} manifest sha256`} />
            {manifest ? <LocationView location={manifest.location} what="The round manifest" /> : null}
          </dd>
          <dt>The fix proposed</dt>
          <dd className="stack" style={{ ["--gap" as string]: "6px" }}>
            {releases.length ? (
              releases.map((x) => (
                <span key={`${x.proxy}-${x.implementation}`} className="stack" style={{ ["--gap" as string]: "2px" }}>
                  <span className="small">
                    <span className="mono">{names.get(normHex(x.proxy)) ?? "proxy"}</span> to implementation
                  </span>
                  <Hash
                    value={x.implementation}
                    head={10}
                    tail={4}
                    href={chainId ? explorers.evmAddress(chainId, x.implementation) : undefined}
                    label={`round ${r.round} implementation`}
                  />
                  <span className="xsmall muted">
                    code hash <Hash value={x.code_hash} head={10} label="code hash" />
                  </span>
                </span>
              ))
            ) : (
              <span className="small muted">No release named.</span>
            )}
          </dd>
        </dl>
      </div>

      <div className="stack" style={{ ["--gap" as string]: "10px" }}>
        <h4 className={styles.subhead}>Release permits</h4>
        {r.release_gate === false ? (
          <p className="small muted">None: the gate stayed shut.</p>
        ) : (
          <Suspense fallback={<Loading label="Reading the release permits" />}>
            <RoundPermits d={d} id={id} round={r.round} />
          </Suspense>
        )}
      </div>

      <LiveCheck
        check={{ kind: "near-view", contract: antibody, method: "get_program", args: { program_id: id } }}
        expect={
          typeof r.release_gate === "boolean"
            ? { path: `rounds.${index}.release_gate`, equals: r.release_gate, says: `round ${r.round}'s release gate is ${r.release_gate}` }
            : undefined
        }
        cli={`antibody status --program ${id}`}
        label="Read this round on NEAR"
      />
    </article>
  );
}

/** Each proxy's EIP-1967 implementation slot names the final fix on every chain, and B's exploit reverts. */
function ReleaseChecks() {
  const checks = RELEASE.release.checks;
  const slots = checks.filter((c) => c.address && c.expected);
  const reverts = checks.filter((c) => typeof c.b_exploit_reverts === "boolean");
  return (
    <div className="card stack">
      <div className={styles.roundHead}>
        <h3 className={styles.cardTitle}>The released code, on each chain</h3>
        <Pill icon="clock" size="sm">
          recorded {RELEASE.results_file.match(/\d{4}-\d{2}-\d{2}/)?.[0] ?? ""}, checkable now
        </Pill>
      </div>
      <p className="small muted">
        After round 2 the relayer submitted the release permits. Each proxy&apos;s implementation slot (EIP-1967) now names the
        final fix, and B&apos;s fee-sweep exploit (<Link href="/commits/14">commit 14</Link>) reverts against it. Recorded in{" "}
        <a href={sourceUrl(RELEASE.results_file)} target="_blank" rel="noreferrer noopener">
          the results file <Icon name="external" size={11} className={styles.inlineIcon} />
        </a>{" "}
        under <code>release.checks</code>.
      </p>
      <div className={`table-wrap ${styles.cardsWrap}`}>
        <table className={`table ${styles.rtable}`}>
          <caption className="visually-hidden">Recorded post-release checks</caption>
          <thead>
            <tr>
              <th scope="col">Chain</th>
              <th scope="col">Check</th>
              <th scope="col">Recorded</th>
            </tr>
          </thead>
          <tbody>
            {checks.map((c, i) => (
              <tr key={i}>
                <Cell label="Chain" lead>
                  <span className="small">{chainName(c.chain_id)}</span>
                </Cell>
                <Cell label="Check">
                  {c.address ? (
                    <span className="stack" style={{ ["--gap" as string]: "2px" }}>
                      <span className="small">
                        <span className="mono">{c.proxy ?? "proxy"}</span>&apos;s implementation is the final fix
                      </span>
                      <Hash value={c.expected} head={10} tail={4} href={explorers.evmAddress(c.chain_id, c.expected ?? "")} label="expected implementation" />
                    </span>
                  ) : (
                    <span className="small">B&apos;s exploit reverts against the released code</span>
                  )}
                </Cell>
                <Cell label="Recorded">
                  {(c.address ? c.ok : c.b_exploit_reverts) ? (
                    <Pill tone="ok" size="sm">
                      true
                    </Pill>
                  ) : (
                    <Pill tone="bad" size="sm">
                      false
                    </Pill>
                  )}
                </Cell>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <h4 className={styles.subhead}>Read each slot now, from your browser</h4>
      <div className="grid grid-2">
        {slots.map((c) => {
          const want = normHex(c.expected);
          const body = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "eth_getStorageAt", params: [c.address, RELEASE.eip1967_implementation_slot, "latest"] });
          return (
            <div key={`${c.chain_id}-${c.address}`} className="card-flat stack" style={{ ["--gap" as string]: "8px" }}>
              <p className="small">
                <strong>
                  {chainName(c.chain_id)}, {c.proxy}
                </strong>
              </p>
              <LiveCheck
                check={{ kind: "evm", chainId: c.chain_id, method: "eth_getStorageAt", params: [c.address, RELEASE.eip1967_implementation_slot, "latest"] }}
                expect={{ endsWith: want, says: `the slot ends with ${want}` }}
                cli={c.rpc ? `curl -s ${c.rpc} -H 'content-type: application/json' -d '${body}'` : undefined}
                label="Read the slot"
              />
            </div>
          );
        })}
      </div>
      {reverts.length ? (
        <Command label="Recorded value">{`python3 scripts/docs/recorded.py 'release.checks.*.b_exploit_reverts'`}</Command>
      ) : null}
    </div>
  );
}

/** 8. Rounds: how a fix ships. The TEE checks it, the keeper re-runs every paid proof at it, the gate decides. */
export function RoundsSection({ d, id, slots, antibody }: { d: ProgramDetail; id: number; slots: SlotView[]; antibody: string }) {
  const all = d.program?.rounds ?? [];
  const rounds = [...all].sort((a, b) => a.round - b.round);
  const rows = d.commits ?? [];
  const proposal = proposalOf(d.program?.proposal);

  return (
    <Section
      id="rounds"
      title="Rounds and the release gate"
      lede="A round ships a fix as code overrides at the same anchor. The TEE checks it, every paid proof is re-run against it, and a release permit is signed only for code that every paid proof fails against."
    >
      <div className="stack">
        {proposal ? (
          <div className="callout" role="status">
            <Icon name="clock" size={16} />
            <span>
              A fix is proposed{proposal.round !== null ? ` for round ${proposal.round}` : ""}
              {proposal.check_run !== null ? (
                <>
                  ; the TEE checks it in <Link href={`/runs/${proposal.check_run}`}>run {proposal.check_run}</Link>
                </>
              ) : null}
              {proposal.proposed_at ? (
                <>
                  , proposed <Time t={proposal.proposed_at} />
                </>
              ) : null}
              . Re-runs at it are in flight; the round opens once its window has passed.
            </span>
          </div>
        ) : null}
        {rounds.length ? (
          rounds.map((r) => <RoundCard key={r.round} r={r} index={all.indexOf(r)} d={d} id={id} rows={rows} slots={slots} antibody={antibody} />)
        ) : (
          <Empty>
            No round yet: this program still runs its round-0 code. A round is how a fix ships, and only a fix every paid proof fails
            against is released.
          </Empty>
        )}
        {id === RELEASE.program_id && rounds.some((r) => r.release_gate) ? <ReleaseChecks /> : null}
        {rounds.length ? (
          <p className="small muted">
            The re-runs that decided each gate are on each commit&apos;s page. How rounds work:{" "}
            <a href={sourceUrl("docs/rounds.md")} target="_blank" rel="noreferrer noopener">
              docs/rounds.md <Icon name="external" size={11} className={styles.inlineIcon} />
            </a>
            .
          </p>
        ) : null}
      </div>
    </Section>
  );
}
