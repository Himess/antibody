import type { Metadata } from "next";
import Link from "next/link";
import { ClaimCard } from "@/components/checks/ClaimCard";
import { ClaimChecksProvider, ClaimIndex, type IndexRow } from "@/components/checks/ClaimChecks";
import { Md } from "@/components/checks/Md";
import { judges, longDuration, momentTimes, primaryChecks, runRole, safeUrl, specOf, type JudgeProgram } from "@/components/checks/judges";
import styles from "@/components/checks/checks.module.css";
import { Cell } from "@/components/programs/Cell";
import { OnThisPage } from "@/components/programs/OnThisPage";
import rt from "@/components/programs/programs.module.css";
import { Command } from "@/components/ui/Command";
import { CopyButton } from "@/components/ui/CopyButton";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { PageHead, Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { Empty } from "@/components/ui/States";
import { Time } from "@/components/ui/Time";
import { chainName, sourceUrl } from "@/lib/config";
import { fixtureAgentLetter, provenance } from "@/lib/format";

export async function generateMetadata(): Promise<Metadata> {
  return {
    title: "For judges",
    description:
      "Every claim of Antibody's recorded testnet run, with what to expect, the transactions behind it, the CLI command and a check your browser runs.",
  };
}

/** Whose account signed: our fixture agents and our services are labelled as ours. */
function whose(account: string): string | null {
  const p = provenance(account);
  if (p === "fixture-agent") return `our fixture agent ${fixtureAgentLetter(account) ?? ""}`.trim();
  if (p === "practice") return "a practice try";
  if (p === "ours") return `our ${account.split(".")[0]}`;
  return null;
}

const s1 = (v: number | null) => (v === null ? "—" : `${v.toFixed(1)} s`);
const chain = (id: number | null | undefined) => (typeof id === "number" ? chainName(id) : "—");

/** "Whole run: 4590 s …" with the duration a reader can take in: "1 h 16 min 30 s (4590 s)". */
const readableTiming = (t: string) =>
  t.replace(/^Whole run: (\d+(?:\.\d+)?) s\b/, (m, n: string) => (Number(n) >= 60 ? `Whole run: ${longDuration(Number(n))} (${n} s)` : m));

/** The sandbox's setup note: paragraphs, and fenced code as a command to copy. */
function Setup({ lines }: { lines: string[] }) {
  const blocks: { code: boolean; text: string }[] = [];
  let fenced: string[] | null = null;
  for (const l of lines) {
    if (l.startsWith("```")) {
      if (fenced) blocks.push({ code: true, text: fenced.join(String.fromCharCode(10)) });
      fenced = fenced ? null : [];
    } else if (fenced) fenced.push(l);
    else if (l.trim()) blocks.push({ code: false, text: l });
  }
  return (
    <div className="stack" style={{ ["--gap" as string]: "10px" }}>
      {blocks.map((b, i) =>
        b.code ? (
          <Command key={i} label="Setup" prompt={false}>
            {b.text}
          </Command>
        ) : (
          <p key={i} className={styles.note}>
            <Md text={b.text} />
          </p>
        ),
      )}
    </div>
  );
}

function ExternalIcon() {
  return <Icon name="external" size={11} style={{ display: "inline", verticalAlign: "-1px" }} />;
}

/** The run's judge view of programs; on a phone each row is a card. */
function ProgramsTable({ rows, caption }: { rows: JudgeProgram[]; caption: string }) {
  return (
    <div className={`table-wrap ${rt.cardsWrap}`}>
      <table className={`table ${rt.rtable}`}>
        <caption className="visually-hidden">{caption}</caption>
        <thead>
          <tr>
            <th scope="col">Program</th>
            <th scope="col">Phase</th>
            <th scope="col">Anchor</th>
            <th scope="col">T1-confirmed</th>
            <th scope="col">Payouts</th>
            <th scope="col">Pause / release permits</th>
            <th scope="col">Audit</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.program_id}>
              <Cell label="Program" lead>
                <span className="row" style={{ ["--gap" as string]: "8px" }}>
                  <Link href={`/programs/${r.program_id}`}>Program {r.program_id}</Link>
                  {runRole(r.this_run) ? (
                    <Pill tone="info" size="sm" icon={null}>
                      this run: {runRole(r.this_run)}
                    </Pill>
                  ) : null}
                </span>
                <div className="xsmall muted">{r.name}</div>
              </Cell>
              <Cell label="Phase">
                <code>{r.phase ?? "—"}</code>
              </Cell>
              <Cell label="Anchor">
                <code>{r.anchor ?? "—"}</code>
              </Cell>
              <Cell label="T1-confirmed" className="num">
                {r.t1_confirmed ?? "—"} of {r.runs ?? "—"}
              </Cell>
              <Cell label="Payouts" className="num">
                {r.payouts}
                {r.each_try ? <div className="xsmall muted">each try adds one</div> : null}
              </Cell>
              <Cell label="Pause / release" className="num">
                {r.pauses ?? "—"} / {r.releases ?? "—"}
              </Cell>
              <Cell label="Audit">
                {r.audit_ok ? (
                  <Pill tone="ok" size="sm">
                    ok
                  </Pill>
                ) : (
                  <Pill tone="bad" size="sm">
                    fails
                  </Pill>
                )}
              </Cell>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export default async function JudgesPage() {
  const j = judges;
  const claims = j.claims ?? [];
  const evm = j.evm_checks ?? [];
  const look = j.one_look;
  const times = momentTimes(j);
  const acme = j.programs?.acme;
  const recordedHasFile = /--file\s/.test(j.recorded_cmd ?? "");
  const limits = j.limits?.items ?? [];
  // The worker-image delay is in docs/trust.md; README's list does not name it yet.
  const imageLimitListed = limits.some((l) => /\bimage/i.test(`${l.title} ${l.text}`));
  const claimNos = new Set(claims.map((c) => c.n));
  const looseEvm = evm.filter((e) => typeof e.claim !== "number" || !claimNos.has(e.claim));
  const index: IndexRow[] = claims.map((c) => ({ n: c.n, claim: c.claim, tier: c.tier ?? null, state: c.state ?? null, checks: primaryChecks(c, evm) }));
  const programs = [...(look?.programs ?? [])].sort((a, b) => a.program_id - b.program_id);
  const thisRun = programs.filter((r) => runRole(r.this_run));
  const earlier = programs.filter((r) => !runRole(r.this_run));
  const txs = j.transactions?.items ?? [];
  const firstTx = txs[0];
  const txPattern = firstTx && firstTx.command.includes(firstTx.tx) ? firstTx.command.replace(firstTx.tx, "<transaction>").replace(firstTx.signer, "<signer>") : null;

  return (
    <>
      <PageHead
        crumbs={[{ href: "/judges", label: "For judges" }]}
        eyebrow="For judges"
        title="Check every claim yourself"
        lede={
          <p>
            Five minutes: watch the one moment, try the loop with no account, then run each claim&apos;s check from your browser. The
            claims come from the recorded testnet run of {j.run_utc}; JUDGES.md is generated from the same results file.
          </p>
        }
      >
        <Pill tone={j.all_ok ? "ok" : "bad"}>{j.all_ok ? "recorded run · all checks passed" : "recorded run · not all checks passed"}</Pill>
        <Pill tone="info" icon="layers">
          NEAR {j.network}
        </Pill>
      </PageHead>

      <OnThisPage
        items={[
          { id: "path", label: "Five-minute path" },
          { id: "run", label: "Recorded run" },
          { id: "claims", label: "Claims" },
          { id: "one-look", label: "One look" },
          { id: "transactions", label: "Transactions" },
          { id: "timings", label: "Timings" },
          { id: "not-shown", label: "Not shown" },
          { id: "limits", label: "Limits" },
        ]}
      />

      <Section id="path" title="The five-minute path" lede="Four steps; each one stands on its own.">
        <ol className={styles.path} role="list">
          <li className={styles.pathStep}>
            <span className={styles.pathNo} aria-hidden>
              01
            </span>
            <h3>Watch the one moment</h3>
            <p>
              Our fixture agent A&apos;s Critical, from its commit block: verified at {s1(times.verdict)}, T1-confirmed at {s1(times.t1)}, paid at{" "}
              {s1(times.paid)}, both chains paused by {s1(times.lastPause)}. Recorded, every step a transaction.
            </p>
            <Link href="/moment" className="btn btn-sm">
              Watch the one moment <Icon name="arrow" size={12} />
            </Link>
          </li>
          <li className={styles.pathStep}>
            <span className={styles.pathNo} aria-hidden>
              02
            </span>
            <h3>Try it with no account</h3>
            <p>
              The practice program runs the whole loop on testnet: a practice account is vended, the tutorial proof is sealed, committed and
              verified in the TEE, and the contract pays it. Your try is live.
            </p>
            <Link href="/try" className="btn btn-sm">
              Try it, no account <Icon name="arrow" size={12} />
            </Link>
          </li>
          <li className={styles.pathStep}>
            <span className={styles.pathNo} aria-hidden>
              03
            </span>
            <h3>Check every claim</h3>
            <p>
              {claims.length} claims from the run. Run every check with one button, or one at a time: each shows its raw answer, the
              transactions behind the claim and the CLI command.
            </p>
            <a href="#claims" className="btn btn-sm">
              Go to the claims <Icon name="arrowDown" size={12} />
            </a>
          </li>
          <li className={styles.pathStep}>
            <span className={styles.pathNo} aria-hidden>
              04
            </span>
            <h3>See what is not shown</h3>
            <p>
              What this run does not show, the limits that hold for every program, and <Link href="/status">our own health</Link>, live.
            </p>
            <a href="#not-shown" className="btn btn-sm">
              What is not shown <Icon name="arrowDown" size={12} />
            </a>
          </li>
        </ol>
      </Section>

      <Section id="run" title="The recorded run" lede="Everything below comes from one results file. The run is recorded; the checks read testnet now.">
        <div className={styles.runFacts}>
          <div className="stack" style={{ ["--gap" as string]: "12px" }}>
            {(j.intro ?? []).map((para, i) => (
              <p key={i} className="prose">
                <Md text={para} />
              </p>
            ))}
            <p className="small muted">
              <Md text={j.generated} />
            </p>
          </div>
          <dl className={styles.mini}>
            <div>
              <dt>Network</dt>
              <dd>
                NEAR {j.network}; Acme Lend on {chain(j.chains?.verifies_and_pauses)} (it verifies and pauses) and {chain(j.chains?.pauses)} (it
                pauses)
              </dd>
            </div>
            <div>
              <dt>Run started</dt>
              <dd>{j.started_unix ? <Time t={j.started_unix} /> : j.run_utc}</dd>
            </div>
            <div>
              <dt>Result</dt>
              <dd>
                <Pill tone={j.all_ok ? "ok" : "bad"} size="sm">
                  {j.all_ok ? "all checks passed" : "not all checks passed"}
                </Pill>
              </dd>
            </div>
            <div>
              <dt>Results file</dt>
              <dd>
                <a href={sourceUrl(j.source)} target="_blank" rel="noreferrer noopener" className="mono xsmall">
                  {j.source} <ExternalIcon />
                </a>
              </dd>
            </div>
            <div>
              <dt>Generated from it</dt>
              <dd>
                <a href={sourceUrl("JUDGES.md")} target="_blank" rel="noreferrer noopener">
                  JUDGES.md <ExternalIcon />
                </a>{" "}
                and this page&apos;s claims, by{" "}
                <a href={sourceUrl(j.generated_by)} target="_blank" rel="noreferrer noopener" className="mono xsmall">
                  {j.generated_by} <ExternalIcon />
                </a>
              </dd>
            </div>
            {j.anchor?.block ? (
              <div>
                <dt>Anchor</dt>
                <dd>
                  {chain(j.anchor.chain_id)} block <span className="num">{j.anchor.block.toLocaleString("en-US")}</span>, confirmed by{" "}
                  <span className="mono xsmall">{j.anchor.light_client ?? "the light client"}</span> (<Link href="/trust#anchor">check it</Link>)
                </dd>
              </div>
            ) : null}
          </dl>
        </div>
        <div className="stack" style={{ ["--gap" as string]: "10px", marginTop: 20 }}>
          <p className={styles.note}>Values that only the run recorded (timings, refusals, counts) are printed from its results file:</p>
          <Command label="Print a recorded value">{j.recorded_cmd}</Command>
          {recordedHasFile ? (
            <p className="xsmall muted">
              <code className="nowrap">--file</code> names this run&apos;s results file; without it, <code>recorded.py</code> reads the sandbox run&apos;s.
            </p>
          ) : null}
        </div>
      </Section>

      <Section
        id="claims"
        title="Claims"
        lede={
          <>
            {look?.intro ? (
              <>
                <Md text={look.intro} />{" "}
              </>
            ) : null}
            Each card states the claim as JUDGES.md&apos;s table does, from the same results file, with the transactions behind it and a check
            your browser runs, showing the raw answer.
          </>
        }
      >
        {claims.length ? (
          <ClaimChecksProvider>
            <ClaimIndex rows={index} />
            <ol className={styles.claims}>
              {claims.map((c) => (
                <ClaimCard key={c.n} claim={c} acme={acme} evm={evm} limits={limits} />
              ))}
            </ol>
          </ClaimChecksProvider>
        ) : (
          <Empty>No claims in this results file.</Empty>
        )}
      </Section>

      <Section
        id="one-look"
        title="One look"
        lede="The deployment on one screen: each contract and whether it is locked, the contract's audit, and every program, as the run read them."
      >
        <div className="stack" style={{ ["--gap" as string]: "16px" }}>
          {(look?.setup ?? []).length ? <Setup lines={look.setup} /> : null}
          <Command label="The one-screen summary">{look?.command ?? "antibody judge"}</Command>
          {look?.expected ? (
            <details className={styles.more}>
              <summary>What it prints, as JUDGES.md states it</summary>
              <div className={styles.moreBody}>
                <p className={styles.note}>
                  <Md text={look.expected} />
                </p>
              </div>
            </details>
          ) : null}
          <div className={styles.grid2}>
            <div className="stack" style={{ ["--gap" as string]: "10px" }}>
              <h3 className={styles.subhead}>The contracts</h3>
              {look?.contracts?.length ? (
                <div className={`table-wrap ${rt.cardsWrap}`}>
                  <table className={`table ${rt.rtable}`}>
                    <caption className="visually-hidden">The deployment&apos;s contracts and whether each account is locked</caption>
                    <thead>
                      <tr>
                        <th scope="col">Role</th>
                        <th scope="col">Account</th>
                        <th scope="col">Locked</th>
                      </tr>
                    </thead>
                    <tbody>
                      {look.contracts.map((c) => (
                        <tr key={c.account}>
                          <Cell label="Role" lead>
                            <strong className="small">{c.role}</strong>
                          </Cell>
                          <Cell label="Account" className={styles.cell}>
                            <span className="mono xsmall">{c.account}</span>
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
              ) : (
                <Empty>The run recorded no contracts.</Empty>
              )}
              <p className="small">
                Contract audit:{" "}
                {look?.audit_ok ? (
                  <Pill tone="ok" size="sm">
                    balances
                  </Pill>
                ) : (
                  <Pill tone="bad" size="sm">
                    does not balance
                  </Pill>
                )}
              </p>
            </div>
            <div className="stack" style={{ ["--gap" as string]: "10px" }}>
              <h3 className={styles.subhead}>Check it from your browser</h3>
              {look?.live?.length ? (
                look.live.map((l, i) => (
                  <LiveCheck key={i} check={specOf(l)} expect={l.expect} cli={l.cli && l.cli !== look.command ? l.cli : undefined} label={l.label} />
                ))
              ) : (
                <p className="small muted">This summary is read in a terminal.</p>
              )}
            </div>
          </div>
          {look?.programs_note ? (
            <p className={styles.note}>
              <Md text={look.programs_note} />
            </p>
          ) : null}
          {thisRun.length ? (
            <div className="stack" style={{ ["--gap" as string]: "10px" }}>
              <h3 className={styles.subhead}>This run&apos;s programs</h3>
              <ProgramsTable rows={thisRun} caption="This run's programs, as the run's judge view read them" />
            </div>
          ) : null}
          {earlier.length ? (
            <div className="stack" style={{ ["--gap" as string]: "10px" }}>
              <h3 className={styles.subhead}>Earlier runs on the same deployment</h3>
              <ProgramsTable rows={earlier} caption="Earlier runs' programs on the same deployment, as the run's judge view read them" />
            </div>
          ) : null}
        </div>
      </Section>

      {looseEvm.length ? (
        <Section id="evm-checks" title="Other EVM checks" lede="Read by your browser from each chain's public RPC; the curl command reads the same.">
          <ul className={styles.evmList}>
            {looseEvm.map((e, i) => (
              <li key={`${e.chain_id}-${e.address}-${i}`} className={styles.evmItem}>
                <p>
                  <Md text={e.label} />
                </p>
                {e.live ? (
                  <LiveCheck
                    check={{ kind: "evm", chainId: e.chain_id, method: e.method, params: e.params }}
                    expect={e.expect}
                    cli={e.curl}
                    label={`Read it on ${chainName(e.chain_id)}`}
                  />
                ) : (
                  <Command label="curl command">{e.curl}</Command>
                )}
              </li>
            ))}
          </ul>
        </Section>
      ) : null}

      <Section id="transactions" title="Transactions" lede={<Md text={j.transactions?.intro} />}>
        {txs.length ? (
          <div className="stack" style={{ ["--gap" as string]: "12px" }}>
            {txPattern ? <Command label="Each row's command">{txPattern}</Command> : null}
            <div className={`table-wrap ${rt.cardsWrap}`}>
              <table className={`table ${rt.rtable}`}>
                <caption className="visually-hidden">Each transaction behind the claims, its signer and the command that shows it</caption>
                <thead>
                  <tr>
                    <th scope="col">Step</th>
                    <th scope="col">Transaction</th>
                    <th scope="col">Signer</th>
                    <th scope="col">Command</th>
                  </tr>
                </thead>
                <tbody>
                  {txs.map((t) => (
                    <tr key={`${t.what}-${t.tx}`}>
                      <Cell label="Step" lead className={styles.nowrap}>
                        <strong className="small">{t.what}</strong>
                      </Cell>
                      <Cell label="Transaction">
                        <Hash value={t.tx} href={safeUrl(t.url)} head={10} label={`${t.what} transaction`} />
                      </Cell>
                      <Cell label="Signer" className={styles.cell}>
                        <span className="mono xsmall">{t.signer}</span>
                        {whose(t.signer) ? <div className="xsmall muted">{whose(t.signer)}</div> : null}
                      </Cell>
                      <Cell label="Command">
                        <CopyButton value={t.command} label={`Copy the command for ${t.what}`} />
                      </Cell>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        ) : (
          <Empty>No transactions in this results file.</Empty>
        )}
      </Section>

      <Section id="timings" title="Timings" lede="As the run recorded them: A's from its commit's block; the others are durations on the run's own clock.">
        {j.timings?.length ? (
          <ul className={styles.items}>
            {j.timings.map((t, i) => (
              <li key={i}>
                <Icon name="clock" size={14} />
                <span>
                  <Md text={readableTiming(t)} />
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <Empty>No timings in this results file.</Empty>
        )}
      </Section>

      <Section id="not-shown" title="What this does not show">
        {j.does_not_show?.length ? (
          <ul className={styles.items}>
            {j.does_not_show.map((t, i) => (
              <li key={i}>
                <Icon name="info" size={14} />
                <span>
                  <Md text={t} />
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <Empty>Nothing listed.</Empty>
        )}
      </Section>

      <Section
        id="limits"
        title="Limits"
        lede={
          <>
            README.md&apos;s limitations, item by item{imageLimitListed ? "" : ", and the worker-image delay from docs/trust.md"}. They hold for
            every program, not only this run.{" "}
            <a href={`${sourceUrl(j.limits?.source ?? "README.md")}#${j.limits?.anchor ?? "limitations"}`} target="_blank" rel="noreferrer noopener">
              Read them in README.md <ExternalIcon />
            </a>
          </>
        }
      >
        {limits.length ? (
          <ul className={`${styles.items} ${styles.limits}`}>
            {limits.map((l, i) => (
              <li key={i} id={`limit-${i}`}>
                <Icon name="alert" size={14} />
                <span>
                  {l.title ? (
                    <strong>
                      <Md text={l.title} />
                    </strong>
                  ) : null}
                  {l.title && !/^[,.;:]/.test(l.text ?? "") ? " " : ""}
                  <Md text={l.text} />
                </span>
              </li>
            ))}
            {imageLimitListed ? null : (
              <li id="limit-images">
                <Icon name="alert" size={14} />
                <span>
                  <strong>Worker images.</strong> A program accepts a worker image OutLayer approves after its creation only after the image
                  delay (2 days on testnet), once near-antibody lists it. Until then a run on that image gets no T1, and its payout waits the
                  challenge window (
                  <a href={sourceUrl("docs/trust.md")} target="_blank" rel="noreferrer noopener">
                    docs/trust.md <ExternalIcon />
                  </a>
                  ).
                </span>
              </li>
            )}
          </ul>
        ) : (
          <Empty>
            The limitations are listed in{" "}
            <a href={sourceUrl("README.md")} target="_blank" rel="noreferrer noopener">
              README.md
            </a>
            .
          </Empty>
        )}
        <p className="small" style={{ marginTop: 16 }}>
          What applies right now, such as worker images inside their delay and the services&apos; alerts, is on{" "}
          <Link href="/status">the status page</Link>, which your browser refreshes every 30 s.
        </p>
        <div className="callout" style={{ marginTop: 20 }}>
          <Icon name="info" size={16} />
          <p>
            The run is recorded, not live: it ran once, starting {j.run_utc}, and{" "}
            <a href={sourceUrl(j.source)} target="_blank" rel="noreferrer noopener">
              its results file <ExternalIcon />
            </a>{" "}
            holds what it saw. JUDGES.md and the claims on this page are generated from that file by <code>{j.generated_by}</code>, so they
            cannot drift apart. Everything runs on testnets: wNEAR has no value, and Acme Lend and the practice vaults are our own deliberately
            vulnerable fixtures.
          </p>
        </div>
      </Section>
    </>
  );
}
