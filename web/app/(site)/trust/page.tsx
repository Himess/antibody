import type { Metadata } from "next";
import type { ReactNode } from "react";
import Link from "next/link";
import { AnchorCheck } from "@/components/checks/AnchorCheck";
import { FullHash } from "@/components/checks/FullHash";
import { Md } from "@/components/checks/Md";
import { T2Records } from "@/components/checks/T2Records";
import { judges, safeUrl, type JudgesData } from "@/components/checks/judges";
import { RECEIPTS, REPRODUCE, RUN_BINDING, VERIFIER_BUILD, ZK } from "@/components/checks/trust-data";
import styles from "@/components/checks/checks.module.css";
import { Cell } from "@/components/programs/Cell";
import { OnThisPage } from "@/components/programs/OnThisPage";
import rt from "@/components/programs/programs.module.css";
import { TierBadge } from "@/components/ui/Badges";
import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { PageHead, Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { Empty, ErrorState } from "@/components/ui/States";
import { Time } from "@/components/ui/Time";
import { AccountLink } from "@/components/ui/TxLink";
import { api, t2NotYet } from "@/lib/api";
import { config, sourceUrl } from "@/lib/config";

export async function generateMetadata(): Promise<Metadata> {
  return {
    title: "Trust",
    description:
      "What you trust in Antibody and how to check each part: the anchor against the light client, a run's TDX quote and its T1 confirmation, T2 on NEAR, and the reproducible builds.",
  };
}

function Ext({ href, children, className }: { href: string; children: ReactNode; className?: string }) {
  return (
    <a href={href} target="_blank" rel="noreferrer noopener" className={className}>
      {children} <Icon name="external" size={11} style={{ display: "inline", verticalAlign: "-1px" }} />
    </a>
  );
}

/** The panel's actors start lower case ("the light client"); a contract's name stays as it is ("near-antibody's owner"). */
const actorTitle = (s: string) => (/^near-/.test(s) ? s : s.charAt(0).toUpperCase() + s.slice(1));

/** README's deployment table names the MPC network's contract; /v1/guardian reads it from near-guardian's config. */
const MPC_CONTRACT = "v1.signer-prod.testnet";

export default async function TrustPage() {
  const [trust, health, t2, guardian] = await Promise.all([api.trust(), api.health(), api.t2(), api.guardian()]);
  const guardianConfig = guardian.ok && guardian.data?.config && typeof guardian.data.config === "object" ? (guardian.data.config as Record<string, unknown>) : {};
  const mpcContract = typeof guardianConfig.mpc === "string" && guardianConfig.mpc ? guardianConfig.mpc : MPC_CONTRACT;
  const contracts = health.ok ? health.data.contracts : undefined;
  const release = contracts?.release;
  const antibody = contracts?.antibody ?? judges.deployment?.near_antibody ?? "bounty.antibody.testnet";
  const acme = judges.programs?.acme ?? 5;
  const liveVerifier = release?.verifier?.wasm_hash ? release.verifier : null;
  const verifier = liveVerifier ?? judges.deployment?.verifier ?? null;
  const anchor: JudgesData["anchor"] = judges.anchor ?? {};
  const lightClient = contracts?.light_client ?? anchor.light_client ?? judges.deployment?.light_client ?? "client-eth2.sepolia.testnet";
  // A near-t2 that holds no record yet reads as not yet on testnet: never a zero count.
  const t2Count = t2.ok && Array.isArray(t2.data?.records) ? t2.data.records.length : 0;
  const t2Status = t2Count ? (
    <Pill tone="t2" size="sm">
      {t2Count} record{t2Count === 1 ? "" : "s"} on testnet
    </Pill>
  ) : t2.ok || t2NotYet(t2) ? (
    <Pill tone="pending" size="sm">
      not yet on testnet
    </Pill>
  ) : (
    <Pill tone="warn" size="sm">
      status unavailable
    </Pill>
  );
  const deployed: { account: string; code_hash?: string; locked?: boolean; role: string }[] = release?.contracts?.length
    ? release.contracts
    : (judges.deployment?.contracts ?? []);
  const deployedLive = Boolean(release?.contracts?.length);

  return (
    <>
      <PageHead
        crumbs={[{ href: "/trust", label: "Trust" }]}
        eyebrow="Trust"
        title="What you trust, and how to check it"
        lede={
          <p>
            Each part of Antibody, what it is trusted for, and a check anyone can run. The checks on this page run in your browser against NEAR
            testnet and Sepolia, and each comes with its terminal command.
          </p>
        }
      />

      <OnThisPage
        items={[
          { id: "tiers", label: "Tiers" },
          { id: "panel", label: "Trust panel" },
          { id: "checks", label: "Browser checks" },
          { id: "builds", label: "Builds" },
          { id: "three", label: "Three environments" },
          { id: "deployment", label: "Deployment" },
          { id: "mock", label: "What is mock" },
        ]}
      />

      <Section id="tiers" title="Who vouches for a verdict" lede="Each tier is a different reason to believe the pinned verifier ran on the committed input.">
        <div className={styles.cards}>
          <div className={styles.cardBox}>
            <TierBadge tier="T0" />
            <p className="small">
              OutLayer&apos;s operator delivers the TEE worker&apos;s output. A payout on T0 waits its challenge window; above the instant cap it also
              needs an independent second run that agrees.
            </p>
          </div>
          <div className={styles.cardBox}>
            <TierBadge tier="T1" />
            <p className="small">
              The run&apos;s Intel TDX quote, verified on NEAR: it binds the pinned wasm, the input and the output. Instant payouts need T1.{" "}
              <a href="#t1">Check a run</a>
            </p>
          </div>
          <div className={styles.cardBox}>
            <div className="row" style={{ ["--gap" as string]: "8px" }}>
              <TierBadge tier="T2" />
              {t2Status}
            </div>
            <p className="small">
              A validity proof (SP1) of a disclosed verdict, verified on NEAR in about 34 Tgas. It claims no privacy, and no payout in the locked
              deployment depends on it. <a href="#t2">Its records</a>
            </p>
          </div>
          <div className={styles.cardBox}>
            <p className="eyebrow">The common ground</p>
            <p className="small">
              Native, the TEE&apos;s wasm and the zkVM give the same receipt, byte for byte. <a href="#three">Three environments, one answer</a>
            </p>
          </div>
        </div>
      </Section>

      <Section id="panel" title="The trust panel" lede={<>From <code>/v1/trust</code>: each actor, what it is trusted for, and how anyone checks it.</>}>
        {!trust.ok ? (
          <ErrorState what="the trust panel (/v1/trust)" error={trust.error} />
        ) : !trust.data.actors?.length ? (
          <Empty>The API lists no actors.</Empty>
        ) : (
          <ul className={styles.cards} style={{ listStyle: "none", margin: 0, padding: 0 }}>
            {trust.data.actors.map((a, i) => (
              <li key={a.actor ?? i} className={styles.cardBox}>
                <h3>{a.actor ? actorTitle(a.actor) : "An actor"}</h3>
                <dl className={styles.mini}>
                  <div>
                    <dt>Trusted for</dt>
                    <dd>
                      <Md text={a.trusted_for} />
                    </dd>
                  </div>
                  <div>
                    <dt>How anyone checks it</dt>
                    <dd>
                      <Md text={a.check} />
                    </dd>
                  </div>
                  {/MPC network/i.test(a.actor ?? "") ? (
                    <>
                      <div>
                        <dt>MPC contract</dt>
                        <dd>
                          <AccountLink account={mpcContract} site={false} />
                        </dd>
                      </div>
                      {a.account && a.account !== mpcContract ? (
                        <div>
                          <dt>Permits built by</dt>
                          <dd>
                            <AccountLink account={a.account} site={false} />
                          </dd>
                        </div>
                      ) : null}
                    </>
                  ) : (
                    <div>
                      <dt>Account</dt>
                      <dd>{a.account ? <AccountLink account={a.account} site={false} /> : <span className="muted">none: nothing on chain to trust</span>}</dd>
                    </div>
                  )}
                </dl>
              </li>
            ))}
          </ul>
        )}
        <p className="small muted" style={{ marginTop: 14 }}>
          The full table, part by part:{" "}
          <Ext href={sourceUrl("docs/trust.md")}>docs/trust.md</Ext>
        </p>
      </Section>

      <Section id="checks" title="Checks your browser runs" lede="Each check reads the chain itself through public RPCs and shows the raw answer.">
        <div id="anchor" className={styles.checkBlock}>
          <h3>The anchor against the light client</h3>
          <div className={styles.split}>
            <div className="stack" style={{ ["--gap" as string]: "10px" }}>
              <p className="small">
                A program&apos;s proofs run on Ethereum state at one anchor block. When the program is created, near-antibody asks NEAR&apos;s
                Ethereum light client for that block&apos;s hash and compares it with the one the project committed; a mismatch rejects the program
                and returns its escrow. An anchor the light client cannot confirm is labelled project-attested.
              </p>
              <p className="small">
                The light client takes the block number as 8 little-endian bytes and answers a Borsh <code>Option&lt;[u8; 32]&gt;</code>. On testnet
                it has one trusted relayer, and it keeps a window of recent blocks.
              </p>
            </div>
            {anchor.block && anchor.hash ? (
              <AnchorCheck block={anchor.block} expected={anchor.hash} lightClient={lightClient} chainId={anchor.chain_id ?? 11155111} program={acme} />
            ) : (
              <Empty>The recorded run names no anchor.</Empty>
            )}
          </div>
        </div>

        <div id="t1" className={styles.checkBlock}>
          <h3>A run&apos;s quote binding and its T1 confirmation</h3>
          <div className={styles.split}>
            <div className="stack" style={{ ["--gap" as string]: "12px" }}>
              <p className="small">
                OutLayer&apos;s worker signs one Intel TDX quote per execution. Its <code>report_data</code> is SHA-256 over the task: the wasm hash,
                the input hash, the output hash, the caller, the secrets and the time. near-antibody accepts the quote, and the run becomes T1, only
                if:
              </p>
              <ol className={styles.steps}>
                <li>
                  the attestation describes exactly the run it requested: the program&apos;s pinned wasm hash, the SHA-256 of the input it sent and
                  of the bytes it received, the caller and the program&apos;s secrets;
                </li>
                <li>NEAR&apos;s DCAP verifier returns Verified with an accepted TCB status;</li>
                <li>
                  the quote&apos;s <code>report_data</code> equals the recomputed hash;
                </li>
                <li>
                  the worker image (MRTD, RTMR0–3) is in the program&apos;s pinned allowlist; an image approved later counts only after a delay
                  longer than any challenge window.
                </li>
              </ol>
              <p className="small">
                Run {RUN_BINDING.run} (commit {RUN_BINDING.commit}, by our fixture agent A), as <code>antibody tee check</code> read it on{" "}
                {RUN_BINDING.checkedAt.replace("T", " ").replace("Z", " UTC")}:
              </p>
              <dl className={styles.mini}>
                <div>
                  <dt>OutLayer request</dt>
                  <dd>
                    <span className="num">{RUN_BINDING.requestId}</span> · <Ext href={RUN_BINDING.attestation}>its public attestation</Ext>
                  </dd>
                </div>
                <div>
                  <dt>Wasm hash</dt>
                  <dd>
                    <Hash value={RUN_BINDING.wasmHash} head={16} label="wasm hash" /> <span className="xsmall muted">the pinned verifier</span>
                  </dd>
                </div>
                <div>
                  <dt>Input hash</dt>
                  <dd>
                    <Hash value={RUN_BINDING.inputHash} head={16} label="input hash" /> <span className="xsmall muted">the run&apos;s request_sha256</span>
                  </dd>
                </div>
                <div>
                  <dt>Output hash</dt>
                  <dd>
                    <Hash value={RUN_BINDING.outputHash} head={16} label="output hash" /> <span className="xsmall muted">its output_sha256</span>
                  </dd>
                </div>
                <div>
                  <dt>Caller</dt>
                  <dd>
                    <span className="mono xsmall">{RUN_BINDING.caller}</span> <span className="xsmall muted">our fixture agent A</span>
                  </dd>
                </div>
                <div>
                  <dt>DCAP</dt>
                  <dd>
                    <Pill tone="ok" size="sm">
                      Verified · {RUN_BINDING.dcapStatus}
                    </Pill>{" "}
                    <span className="xsmall muted">
                      by <span className="mono">{RUN_BINDING.dcapVerifier}</span>, against OutLayer&apos;s collateral and against fresh Intel PCS
                      collateral
                    </span>
                  </dd>
                </div>
                <div>
                  <dt>Worker image</dt>
                  <dd className="small">OutLayer&apos;s approved image {RUN_BINDING.imageIndex}</dd>
                </div>
                <div>
                  <dt>Recorded in</dt>
                  <dd>
                    <Ext href={sourceUrl(RUN_BINDING.source)} className="mono xsmall">
                      {RUN_BINDING.source}
                    </Ext>
                  </dd>
                </div>
              </dl>
            </div>
            <div className="stack" style={{ ["--gap" as string]: "12px" }}>
              <LiveCheck
                check={{ kind: "near-view", contract: antibody, method: "get_run", args: { run_id: RUN_BINDING.run } }}
                expect={{ path: "outcome.Answered.tier", equals: "T1", says: `run ${RUN_BINDING.run} is T1: its quote was verified on NEAR` }}
                cli={`antibody view --contract ${antibody} --method get_run --args '{"run_id":${RUN_BINDING.run}}'`}
                label={`Read run ${RUN_BINDING.run} on NEAR`}
              />
              <p className="xsmall muted">
                The answer also holds <code>request_sha256</code> and <code>outcome.Answered.output_sha256</code>: the input and output hashes the
                quote binds, left.
              </p>
              <p className="small">The quote itself, checked end to end (report_data, DCAP and the worker image):</p>
              <Command label="Check the quote">{`antibody tee check --request-id ${RUN_BINDING.requestId}`}</Command>
              <p className="small">OutLayer&apos;s own open-source verifier, independent of Antibody&apos;s code, with Intel&apos;s root built in:</p>
              <Command label="OutLayer's verifier">{`outlayer-verify tx ${RUN_BINDING.revealTx}`}</Command>
            </div>
          </div>
        </div>

        <div id="t2" className={styles.checkBlock}>
          <h3>A T2 record on NEAR</h3>
          <p className="small" style={{ maxWidth: "72ch" }}>
            Once a verdict&apos;s proof is disclosed, its re-run can be a Groth16 proof (SP1 {ZK.sp1}) that near-t2 checks on NEAR, then compares
            with near-antibody&apos;s records: the request hash (the TEE&apos;s exact input), the blob, base, manifest and anchor, and the receipt,
            kind and slots. Equal means the verifier, re-run on exactly the TEE&apos;s input, gave the TEE&apos;s receipt.
          </p>
          <T2Records r={t2} />
        </div>
      </Section>

      <Section id="builds" title="Reproducible builds" lede="Rebuild each pinned artifact from the source and compare its hash.">
        <div className={styles.cards}>
          <div className={styles.cardBox}>
            <h3>The verifier wasm</h3>
            <p className="small">
              Each program pins the sha256 of the wasm OutLayer&apos;s TEE runs. The build script pins the toolchain, the locked dependencies and every
              build path, so the same source gives the same bytes on any Linux x86_64 machine.
            </p>
            {verifier ? (
              <dl className={styles.mini}>
                <div>
                  <dt>sha256</dt>
                  <dd>
                    <FullHash value={verifier.wasm_hash} label="verifier wasm sha256" />
                  </dd>
                </div>
                {safeUrl(verifier.wasm_url) ? (
                  <div>
                    <dt>Release</dt>
                    <dd>
                      <Ext href={safeUrl(verifier.wasm_url) as string}>antibody-verify.wasm</Ext>
                    </dd>
                  </div>
                ) : null}
                <div>
                  <dt>Read from</dt>
                  <dd className="small">
                    {liveVerifier ? (
                      <>
                        <code>/health</code> (<code>contracts.release.verifier</code>)
                      </>
                    ) : (
                      <>the recorded run (the API did not answer)</>
                    )}
                  </dd>
                </div>
              </dl>
            ) : (
              <ErrorState what="the verifier's pinned hash" error={health.ok ? "no verifier in /health" : health.error} />
            )}
            {verifier ? (
              <LiveCheck
                check={{ kind: "near-view", contract: antibody, method: "get_pins", args: { program_id: acme } }}
                expect={{ path: "verifier.wasm_hash", equals: verifier.wasm_hash, says: `program ${acme} pins this sha256` }}
                cli={`antibody view --contract ${antibody} --method get_pins --args '{"program_id":${acme}}'`}
                label={`Read program ${acme}'s pin on NEAR`}
              />
            ) : null}
            <Command label="Rebuild">{VERIFIER_BUILD.make}</Command>
            <Command label="Rebuild in a fixed container">{VERIFIER_BUILD.docker}</Command>
            {safeUrl(verifier?.wasm_url) ? <Command label="Hash the released file">{`curl -sL ${verifier?.wasm_url} | sha256sum`}</Command> : null}
            <p className="xsmall muted">
              <Ext href={sourceUrl(VERIFIER_BUILD.script)} className="mono">
                {VERIFIER_BUILD.script}
              </Ext>{" "}
              ·{" "}
              <Ext href={sourceUrl(VERIFIER_BUILD.dockerfile)} className="mono">
                {VERIFIER_BUILD.dockerfile}
              </Ext>
            </p>
          </div>
          <div className={styles.cardBox}>
            <h3>The ZK program (T2)</h3>
            <p className="small">
              SP1&apos;s docker build of the guest. near-t2 pins its program vkey, and the prover refuses an ELF whose vkey is not the pinned one.
              Three docker builds on fresh machines gave the same bytes. A local build is not reproducible across machines: its ELF embeds the
              build&apos;s paths.
            </p>
            <dl className={styles.mini}>
              <div>
                <dt>Program vkey</dt>
                <dd>
                  <FullHash value={ZK.programVkey} label="program vkey" />
                </dd>
              </div>
              <div>
                <dt>Guest ELF sha256</dt>
                <dd>
                  <FullHash value={ZK.elfSha256} label="guest ELF sha256" />
                </dd>
              </div>
              <div>
                <dt>The ELF</dt>
                <dd>
                  <Ext href={sourceUrl(ZK.elfPath)} className="mono xsmall">
                    {ZK.elfPath}
                  </Ext>
                </dd>
              </div>
              <div>
                <dt>Pinned by</dt>
                <dd className="small">
                  near-t2 {t2.ok ? <AccountLink account={t2.data.contract} site={false} /> : <>{t2Status}</>}
                </dd>
              </div>
            </dl>
            <Command label="Rebuild the guest">{ZK.build}</Command>
            <Command label="Hash it">{ZK.hash}</Command>
            <p className="xsmall muted">
              <Ext href={sourceUrl(ZK.doc)}>docs/zk.md</Ext> · <Ext href={sourceUrl(ZK.constants)} className="mono">{ZK.constants}</Ext>
            </p>
          </div>
        </div>
      </Section>

      <Section
        id="three"
        title="Three environments, one answer"
        lede="The verifier gives the same receipt, byte for byte, natively, in the TEE's wasm and in the zkVM. Every tier rests on it."
      >
        <div className="stack" style={{ ["--gap" as string]: "16px" }}>
          <ul className={styles.items}>
            <li>
              <Icon name="cpu" size={14} />
              <span>
                <strong>The TEE&apos;s wasm and native.</strong> CI re-runs the recorded fixtures in both and requires the same verdicts, keys and
                receipts (<Ext href={sourceUrl(".github/workflows/ci.yml")} className="mono xsmall">the verifier job</Ext>).
              </span>
            </li>
            <li>
              <Icon name="sigma" size={14} />
              <span>
                <strong>The zkVM.</strong> SP1&apos;s executor re-runs four testnet runs on the TEE&apos;s exact inputs and gives the TEE&apos;s
                receipts, with the same public values that <code>crates/zk/core</code> computes natively. Their content keys are public (our
                fixture agents and the recorded run&apos;s practice try published theirs in <code>fixtures/zk</code>; a practice try made now seals with
                a key anyone can derive), though on chain their proofs stay sealed until disclosure.
              </span>
            </li>
            <li>
              <Icon name="check" size={14} />
              <span>
                <strong>On NEAR.</strong> Each run&apos;s answer holds the receipt the TEE gave. Read it below and compare.
              </span>
            </li>
          </ul>
          <ul className={styles.cards} style={{ listStyle: "none", margin: 0, padding: 0 }}>
            {RECEIPTS.map((c) => (
              <li key={c.run} className={styles.cardBox}>
                <h3>
                  Run {c.run} · commit {c.commit}
                </h3>
                <p className="small muted">
                  {c.who}, program {c.program}: <code>{c.kind}</code>
                  {c.slots.length ? `, slot ${c.slots.join(", ")}` : ""}
                </p>
                <dl className={styles.mini}>
                  <div>
                    <dt>Receipt sha256</dt>
                    <dd>
                      <Hash value={c.receipt} head={20} label="receipt sha256" />
                    </dd>
                  </div>
                  <div>
                    <dt>Native and zkVM</dt>
                    <dd className="small">
                      the same receipt ·{" "}
                      <Ext href={sourceUrl(`${c.fixture}/expected.json`)} className="mono xsmall">
                        {c.fixture}
                      </Ext>
                    </dd>
                  </div>
                </dl>
                <LiveCheck
                  check={{ kind: "near-view", contract: antibody, method: "get_run", args: { run_id: c.run } }}
                  expect={{ path: "outcome.Answered.facts.receipt_sha256", equals: c.receipt, says: `the TEE's receipt for run ${c.run} is ${c.receipt.slice(0, 12)}…` }}
                  cli={`antibody view --contract ${antibody} --method get_run --args '{"run_id":${c.run}}'`}
                  label="Read the TEE's receipt on NEAR"
                />
              </li>
            ))}
          </ul>
          <div className={styles.grid2}>
            <div className="stack" style={{ ["--gap" as string]: "8px" }}>
              <p className="small">Re-run run 33 natively, from the chain, with the content key our fixture agent A published (no proof):</p>
              <Command label="Native re-run">{REPRODUCE.native}</Command>
            </div>
            <div className="stack" style={{ ["--gap" as string]: "8px" }}>
              <p className="small">Run the same case in SP1&apos;s executor with the docker-built guest:</p>
              <Command label="zkVM re-run">{REPRODUCE.zkvm}</Command>
            </div>
          </div>
        </div>
      </Section>

      <Section
        id="deployment"
        title="The deployment"
        lede="Each contract's code hash, and whether its account is locked: a locked account holds no full-access key, so its code cannot change."
      >
        {!health.ok ? <ErrorState what="the deployment record (/health)" error={health.error} /> : null}
        {deployed.length ? (
          <div className="stack" style={{ ["--gap" as string]: "10px" }}>
            {!deployedLive ? <p className="small muted">As the recorded run read it:</p> : null}
            <div className={`table-wrap ${rt.cardsWrap}`}>
              <table className={`table ${rt.rtable}`}>
                <caption className="visually-hidden">Each contract&apos;s code hash and whether its account is locked</caption>
                <thead>
                  <tr>
                    <th scope="col">Role</th>
                    <th scope="col">Account</th>
                    <th scope="col">Code hash</th>
                    <th scope="col">Locked</th>
                  </tr>
                </thead>
                <tbody>
                  {deployed.map((c) => (
                    <tr key={c.account}>
                      <Cell label="Role" lead>
                        <strong className="small">{c.role}</strong>
                      </Cell>
                      <Cell label="Account" className={styles.cell}>
                        <AccountLink account={c.account} site={false} />
                      </Cell>
                      <Cell label="Code hash">
                        <Hash value={c.code_hash ?? null} head={10} label="code hash" />
                      </Cell>
                      <Cell label="Locked">
                        {c.locked ? (
                          <Pill tone="ok" icon="lock" size="sm">
                            locked
                          </Pill>
                        ) : (
                          <Pill tone="neutral" size="sm">
                            {c.role === "payout token" ? "not ours; not locked" : "not locked"}
                          </Pill>
                        )}
                      </Cell>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p className="small muted">
              {release?.git_commit ? (
                <>
                  Built from commit{" "}
                  <Ext href={`${config.repo}/commit/${release.git_commit}`} className="mono xsmall">
                    {release.git_commit.slice(0, 12)}
                  </Ext>
                  .{" "}
                </>
              ) : null}
              {release?.recorded_at ? (
                <>
                  Recorded <Time t={release.recorded_at} />.{" "}
                </>
              ) : null}
              <code>antibody judge</code> reads the same from NEAR: code hashes, locked status and each program&apos;s pins.
            </p>
            <Command label="CLI command">antibody judge</Command>
          </div>
        ) : health.ok ? (
          <Empty>The deployment record lists no contracts.</Empty>
        ) : null}
      </Section>

      <Section id="mock" title="What is mock" lede="Only the offline sandbox demo. Nothing on testnet is.">
        <ul className={styles.items}>
          <li>
            <Icon name="info" size={14} />
            <span>
              <strong>The sandbox demo</strong> (<code>make demo</code>): MOCK OutLayer, DCAP and MPC contracts, resolved by the real verifier with a
              TEST seed and a TEST secp256k1 key. The T1 checks on its MOCK quotes are the real code.
            </span>
          </li>
          <li>
            <Icon name="info" size={14} />
            <span>
              <strong>The sandbox&apos;s near-t2</strong> takes proofs under a TEST Groth16 key that anyone can make proofs for (
              <code>fixtures/zk/t2-test-key.json</code>), from a MOCK prover. The MOCK worker lies once on purpose, and near-t2 records the
              contradiction.
            </span>
          </li>
          <li>
            <Icon name="info" size={14} />
            <span>
              <strong>Test keys in fixtures:</strong> the fingerprint tags in <code>fixtures/acme-local/expected.json</code> use a TEST key.
            </span>
          </li>
          <li>
            <Icon name="check" size={14} />
            <span>
              <strong>Testnet uses the real services:</strong> OutLayer&apos;s TEE workers, their TDX quotes verified on NEAR, the MPC network&apos;s
              signatures and the Ethereum light client. Acme Lend and the practice vaults are our own deliberately vulnerable fixtures, and wNEAR on
              testnet has no value. <Link href="/judges#limits">The limits</Link>
            </span>
          </li>
        </ul>
      </Section>
    </>
  );
}
