import type { Metadata } from "next";
import Link from "next/link";
import { ClaimCard } from "@/components/checks/ClaimCard";
import { Md } from "@/components/checks/Md";
import { judges, momentTimes, runRole, safeUrl, specOf } from "@/components/checks/judges";
import styles from "@/components/checks/checks.module.css";
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

export default async function JudgesPage() {
  const j = judges;
  const claims = j.claims ?? [];
  const evm = j.evm_checks ?? [];
  const look = j.one_look;
  const times = momentTimes(j);
  const acme = j.programs?.acme;
  const recordedHasFile = /--file\s/.test(j.recorded_cmd ?? "");
  const evmClaims = Array.from(new Set(evm.map((e) => e.claim).filter((n): n is number => typeof n === "number")));
  const limits = j.limits?.items ?? [];

  return (
    <>
      <PageHead
        eyebrow="For judges"
        title="Check every claim yourself"
        lede={
          <p>
            Five minutes: watch the one moment, try the loop with no account, then run each claim&apos;s check from your browser. The
            claims come from the recorded testnet run of {j.run_utc}; JUDGES.md is generated from the same results file.
          </p>
        }
      >
        <Pill tone="neutral" icon="clock">
          recorded run
        </Pill>
        <Pill tone={j.all_ok ? "ok" : "bad"}>{j.all_ok ? "all checks passed" : "not all checks passed"}</Pill>
        <Pill tone="info" icon="layers">
          NEAR {j.network}
        </Pill>
      </PageHead>

      <Section id="path" title="The five-minute path" lede="Four steps; each one stands on its own.">
        <ol className={styles.path}>
          <li className={styles.pathStep}>
            <span className={styles.pathNo} aria-hidden>
              1
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
              2
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
              3
            </span>
            <h3>Check every claim with one click</h3>
            <p>
              {claims.length} claims from the run, each with its expected result, the transactions behind it, the CLI command and a check
              your browser runs.
            </p>
            <a href="#claims" className="btn btn-sm">
              Go to the claims <Icon name="arrowDown" size={12} />
            </a>
          </li>
          <li className={styles.pathStep}>
            <span className={styles.pathNo} aria-hidden>
              4
            </span>
            <h3>See what is not shown</h3>
            <p>What this run does not show, and the limits that hold for every program.</p>
            <a href="#not-shown" className="btn btn-sm">
              What is not shown <Icon name="arrowDown" size={12} />
            </a>
          </li>
        </ol>
      </Section>

      <Section id="run" title="The recorded run" lede="Everything below comes from one results file. The run is recorded; the checks read testnet now.">
        <div className={styles.runFacts}>
          <div className="stack" style={{ ["--gap" as string]: "12px" }}>
            {(j.intro ?? []).map((p, i) => (
              <p key={i} className="prose">
                <Md text={p} />
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
          <p className={styles.note}>Values only the run recorded (timings, refusals, counts) print from its results file:</p>
          <Command label="Print a recorded value">{j.recorded_cmd}</Command>
          {recordedHasFile ? (
            <p className="xsmall muted">
              <code>--file</code> names this run&apos;s results file; without it, <code>recorded.py</code> reads the sandbox run&apos;s.
            </p>
          ) : null}
        </div>
      </Section>

      <Section id="one-look" title="One look" lede={<Md text={look?.intro} />}>
        <div className="stack" style={{ ["--gap" as string]: "16px" }}>
          {(look?.setup ?? []).length ? <Setup lines={look.setup} /> : null}
          <Command label="The one-screen summary">{look?.command ?? "antibody judge"}</Command>
          {look?.expected ? (
            <p className={styles.note}>
              <Md text={look.expected} />
            </p>
          ) : null}
          <div className={styles.grid2}>
            <div className="stack" style={{ ["--gap" as string]: "10px" }}>
              <h3 className={styles.subhead}>The contracts</h3>
              {look?.contracts?.length ? (
                <div className="table-wrap">
                  <table className="table">
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
                          <td>{c.role}</td>
                          <td className={styles.cell}>
                            <span className="mono xsmall">{c.account}</span>
                          </td>
                          <td>
                            {c.locked ? (
                              <Pill tone="ok" icon="lock" size="sm">
                                locked
                              </Pill>
                            ) : (
                              <Pill tone="neutral" size="sm">
                                {c.role === "payout token" ? "not ours; not locked" : "not locked"}
                              </Pill>
                            )}
                          </td>
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
                look.live.map((l, i) => <LiveCheck key={i} check={specOf(l)} expect={l.expect} cli={l.cli} label={l.label} />)
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
          {look?.programs?.length ? (
            <div className="table-wrap">
              <table className="table">
                <caption className="visually-hidden">Every program on the deployment, as the run&apos;s judge view read it</caption>
                <thead>
                  <tr>
                    <th scope="col">Program</th>
                    <th scope="col">Run</th>
                    <th scope="col">Phase</th>
                    <th scope="col">Anchor</th>
                    <th scope="col">T1-confirmed</th>
                    <th scope="col">Payouts</th>
                    <th scope="col">Pause / release permits</th>
                    <th scope="col">Audit</th>
                  </tr>
                </thead>
                <tbody>
                  {look.programs.map((p) => (
                    <tr key={p.program_id}>
                      <td className={styles.cell}>
                        <Link href={`/programs/${p.program_id}`}>Program {p.program_id}</Link>
                        <div className="xsmall muted">{p.name}</div>
                      </td>
                      <td>
                        {runRole(p.this_run) ? (
                          <Pill tone="info" size="sm" icon={null}>
                            this run: {runRole(p.this_run)}
                          </Pill>
                        ) : (
                          <span className="xsmall muted">earlier run</span>
                        )}
                      </td>
                      <td>
                        <code>{p.phase ?? "—"}</code>
                      </td>
                      <td>
                        <code>{p.anchor ?? "—"}</code>
                      </td>
                      <td className="num">
                        {p.t1_confirmed ?? "—"} of {p.runs ?? "—"}
                      </td>
                      <td className="num">
                        {p.payouts}
                        {p.each_try ? <div className="xsmall muted">each try adds one</div> : null}
                      </td>
                      <td className="num">
                        {p.pauses ?? "—"} / {p.releases ?? "—"}
                      </td>
                      <td>
                        {p.audit_ok ? (
                          <Pill tone="ok" size="sm">
                            ok
                          </Pill>
                        ) : (
                          <Pill tone="bad" size="sm">
                            fails
                          </Pill>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : null}
        </div>
      </Section>

      <Section
        id="claims"
        title="Claims"
        lede="Each claim as JUDGES.md's table states it, generated from the same results file: what to expect, the transactions behind it, the CLI command, and a check your browser runs where one exists, showing the raw answer."
      >
        {claims.length ? (
          <ol className={styles.claims}>
            {claims.map((c) => (
              <ClaimCard key={c.n} claim={c} acme={acme} evmCount={evm.filter((e) => e.claim === c.n).length} />
            ))}
          </ol>
        ) : (
          <Empty>No claims in this results file.</Empty>
        )}
      </Section>

      <Section
        id="evm-checks"
        title="The EVM checks"
        lede={
          <>
            Read by your browser from each chain&apos;s public RPC; the <code>curl</code> command reads the same.
            {evmClaims.length ? (
              <>
                {" "}
                They back{" "}
                {evmClaims.map((n, i) => (
                  <span key={n}>
                    {i ? " and " : ""}
                    <a href={`#claim-${n}`}>claim {n}</a>
                  </span>
                ))}
                .
              </>
            ) : null}
          </>
        }
      >
        {evm.length ? (
          <ul className={styles.evmList}>
            {evm.map((e, i) => (
              <li key={`${e.chain_id}-${e.address}-${i}`} className={styles.evmItem}>
                <p>
                  <Md text={e.label} />
                  {typeof e.claim === "number" ? (
                    <>
                      {" "}
                      <a href={`#claim-${e.claim}`} className="xsmall">
                        claim {e.claim}
                      </a>
                    </>
                  ) : null}
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
        ) : (
          <Empty>This run recorded no EVM checks.</Empty>
        )}
      </Section>

      <Section id="transactions" title="Transactions" lede={<Md text={j.transactions?.intro} />}>
        {j.transactions?.items?.length ? (
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">Step</th>
                  <th scope="col">Transaction</th>
                  <th scope="col">Signer</th>
                  <th scope="col">CLI</th>
                </tr>
              </thead>
              <tbody>
                {j.transactions.items.map((t) => (
                  <tr key={`${t.what}-${t.tx}`}>
                    <td className={styles.nowrap}>{t.what}</td>
                    <td>
                      <Hash value={t.tx} href={safeUrl(t.url)} head={10} label={`${t.what} transaction`} />
                    </td>
                    <td className={styles.cell}>
                      <span className="mono xsmall">{t.signer}</span>
                      {whose(t.signer) ? <div className="xsmall muted">{whose(t.signer)}</div> : null}
                    </td>
                    <td>
                      <span className="row" style={{ ["--gap" as string]: "4px", flexWrap: "nowrap" }}>
                        <code className="xsmall">admin tx</code>
                        <CopyButton value={t.command} label={`Copy the command for ${t.what}`} />
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <Empty>No transactions in this results file.</Empty>
        )}
      </Section>

      <Section id="timings" title="Timings" lede="As the run recorded them, on each chain's own clock.">
        {j.timings?.length ? (
          <ul className={styles.items}>
            {j.timings.map((t, i) => (
              <li key={i}>
                <Icon name="clock" size={14} />
                <span>
                  <Md text={t} />
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
            README.md&apos;s limitations, item by item. They hold for every program, not only this run.{" "}
            <a href={`${sourceUrl(j.limits?.source ?? "README.md")}#${j.limits?.anchor ?? "limitations"}`} target="_blank" rel="noreferrer noopener">
              Read them in README.md <ExternalIcon />
            </a>
          </>
        }
      >
        {limits.length ? (
          <ul className={`${styles.items} ${styles.limits}`}>
            {limits.map((l, i) => (
              <li key={i}>
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
