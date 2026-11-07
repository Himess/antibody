import type { Metadata } from "next";
import type { ReactNode } from "react";
import Link from "next/link";
import { FullHash } from "@/components/checks/FullHash";
import { Md } from "@/components/checks/Md";
import styles from "@/components/checks/checks.module.css";
import { Cell } from "@/components/programs/Cell";
import rt from "@/components/programs/programs.module.css";
import { Command } from "@/components/ui/Command";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { PageHead, Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { Empty, ErrorState } from "@/components/ui/States";
import { Time } from "@/components/ui/Time";
import { api } from "@/lib/api";
import { sourceUrl } from "@/lib/config";

export async function generateMetadata(): Promise<Metadata> {
  return {
    title: "How mainnet works",
    description:
      "What a mainnet program would change, with a live read-only check of NEAR mainnet's Ethereum light client. There is no mainnet deployment, and no mainnet transaction is sent.",
  };
}

type Obj = Record<string, unknown>;
const num = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);
const str = (v: unknown): string | null => (typeof v === "string" ? v : null);

function Ext({ href, children, className }: { href: string; children: ReactNode; className?: string }) {
  return (
    <a href={href} target="_blank" rel="noreferrer noopener" className={className}>
      {children} <Icon name="external" size={11} style={{ display: "inline", verticalAlign: "-1px" }} />
    </a>
  );
}

const cap = (s: string) => (s ? s.charAt(0).toUpperCase() + s.slice(1) : s);

/** The site's one name for NEAR's signing network: the API's text says "Chain Signatures". */
const mpc = (s: string) => s.replace(/\bChain Signatures\b/g, "the MPC network");

export default async function MainnetPage() {
  const r = await api.mainnet();
  const data = r.ok ? r.data : null;
  const check: Obj = data?.light_client_check && typeof data.light_client_check === "object" ? (data.light_client_check as Obj) : {};
  const sent = data ? (num(data.transactions_sent) ?? 0) : 0;
  const lc = str(check.light_client) ?? "client-eth2.bridge.near";
  const last = num(check.last_block);
  const finalized = num(check.ethereum_finalized);
  const lag = num(check.lag_blocks);
  const equal = typeof check.equal === "boolean" ? check.equal : null;
  const checkError = str(check.error);

  return (
    <>
      <PageHead
        crumbs={[{ href: "/mainnet", label: "How mainnet works" }]}
        eyebrow="How mainnet works"
        title="What a mainnet program would change"
        lede={
          <p>
            There is no mainnet deployment, and no mainnet transaction is sent. This page explains what a mainnet program would change, and shows
            a live, read-only check of the one piece its trust rests on that can be checked today: NEAR mainnet&apos;s Ethereum light client.
          </p>
        }
      >
        <Pill tone="neutral" icon="info">
          no mainnet deployment
        </Pill>
        <Pill tone={sent === 0 ? "ok" : "bad"}>
          {sent} mainnet transaction{sent === 1 ? "" : "s"} sent
        </Pill>
        <Pill tone="info" icon="eye">
          view calls only
        </Pill>
      </PageHead>

      {!r.ok ? (
        <section className="page block" aria-label="The explainer could not be read">
          <ErrorState what="how mainnet works (/v1/mainnet)" error={r.error} />
          <p className="small" style={{ marginTop: 14 }}>
            The same explainer is in <Ext href={sourceUrl("docs/mainnet.md")}>docs/mainnet.md</Ext>, and the check runs from a terminal:
          </p>
          <div style={{ marginTop: 10 }}>
            <Command label="CLI command">antibody mainnet-check</Command>
          </div>
        </section>
      ) : (
        <>
          <Section id="same" title="What stays the same" lede={data?.status ? <Md text={`${cap(mpc(data.status))}.`} /> : undefined}>
            {data?.same?.length ? (
              <ul className={styles.items}>
                {data.same.map((s, i) => (
                  <li key={i}>
                    <Icon name="check" size={14} />
                    <span>
                      <Md text={cap(mpc(s))} />
                    </span>
                  </li>
                ))}
              </ul>
            ) : (
              <Empty>The API lists nothing here.</Empty>
            )}
          </Section>

          <Section id="changes" title="What changes" lede="Each part a mainnet program would use, against what testnet uses now.">
            {data?.changes?.length ? (
              <div className={`table-wrap ${rt.cardsWrap}`}>
                <table className={`table ${rt.rtable}`}>
                  <caption className="visually-hidden">Each part a mainnet program would use, on testnet now and on mainnet</caption>
                  <thead>
                    <tr>
                      <th scope="col">Part</th>
                      <th scope="col">Testnet, now</th>
                      <th scope="col">Mainnet</th>
                    </tr>
                  </thead>
                  <tbody>
                    {data.changes.map((c) => (
                      <tr key={c.part}>
                        <Cell label="Part" lead>
                          <strong>{cap(c.part)}</strong>
                        </Cell>
                        <Cell label="Testnet, now" className={styles.cell}>
                          <Md text={mpc(c.testnet)} />
                        </Cell>
                        <Cell label="Mainnet" className={styles.cell}>
                          <Md text={mpc(c.mainnet)} />
                        </Cell>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <Empty>The API lists no changes.</Empty>
            )}
            {data?.latency ? (
              <div className="callout" style={{ marginTop: 16 }}>
                <Icon name="clock" size={16} />
                <p>
                  <strong>Latency.</strong> <Md text={`${cap(mpc(data.latency))}.`} />
                </p>
              </div>
            ) : null}
          </Section>

          <Section
            id="check"
            title="The live check"
            lede="The latest Ethereum block NEAR mainnet's light client holds, that block's hash there and on Ethereum, and how far the client trails Ethereum's finalized head. The API reads it with view calls and an Ethereum RPC, and signs nothing."
          >
            <div className={styles.split}>
              <div className="stack" style={{ ["--gap" as string]: "14px" }}>
                {checkError ? (
                  <ErrorState what="NEAR mainnet's light client" error={checkError}>
                    <span className="small">The API reports the check failed this time; run it from a terminal below.</span>
                  </ErrorState>
                ) : (
                  <dl className={styles.mini}>
                    <div>
                      <dt>Light client</dt>
                      <dd>
                        <Ext href={`https://nearblocks.io/address/${lc}`} className="mono xsmall">
                          {lc}
                        </Ext>{" "}
                        <span className="xsmall muted">on NEAR {str(check.network) ?? "mainnet"}</span>
                      </dd>
                    </div>
                    <div>
                      <dt>Latest block it holds</dt>
                      <dd>
                        {last !== null ? (
                          <Ext href={`https://etherscan.io/block/${last}`} className="num">
                            {last.toLocaleString("en-US")}
                          </Ext>
                        ) : (
                          "—"
                        )}
                      </dd>
                    </div>
                    <div>
                      <dt>Its hash on NEAR</dt>
                      <dd>
                        <FullHash value={str(check.hash_on_near)} label="hash on NEAR" />
                      </dd>
                    </div>
                    <div>
                      <dt>Its hash on Ethereum</dt>
                      <dd>
                        <FullHash value={str(check.hash_on_ethereum)} label="hash on Ethereum" />
                      </dd>
                    </div>
                    <div>
                      <dt>Equal</dt>
                      <dd>
                        {equal === true ? (
                          <Pill tone="ok" size="sm">
                            equal
                          </Pill>
                        ) : equal === false ? (
                          <Pill tone="bad" size="sm">
                            differ
                          </Pill>
                        ) : (
                          "—"
                        )}
                      </dd>
                    </div>
                    <div>
                      <dt>Ethereum&apos;s finalized head</dt>
                      <dd className="num">{finalized !== null ? finalized.toLocaleString("en-US") : "—"}</dd>
                    </div>
                    <div>
                      <dt>Lag</dt>
                      <dd className="num">{lag !== null ? `${lag} block${lag === 1 ? "" : "s"} behind` : "—"}</dd>
                    </div>
                    <div>
                      <dt>Checked</dt>
                      <dd>{num(check.checked_at_unix) !== null ? <Time t={num(check.checked_at_unix)} /> : "—"}</dd>
                    </div>
                    <div>
                      <dt>Read through</dt>
                      <dd className="mono xsmall">
                        {str(check.near_rpc) ?? "—"}
                        <br />
                        {str(check.ethereum_rpc) ?? "—"}
                      </dd>
                    </div>
                    <div>
                      <dt>Transactions sent</dt>
                      <dd className="num">{num(check.transactions_sent) ?? sent}</dd>
                    </div>
                  </dl>
                )}
                <p className="xsmall muted">
                  A mainnet program&apos;s anchor would be checked against this client when the program is created, as testnet&apos;s are against{" "}
                  <code>client-eth2.sepolia.testnet</code>. <Link href="/trust#anchor">Run the testnet check</Link>
                </p>
              </div>
              <div className="stack" style={{ ["--gap" as string]: "12px" }}>
                <LiveCheck
                  check={{ kind: "api", path: "/v1/mainnet" }}
                  expect={{ path: "light_client_check.equal", equals: true, says: "the two hashes are equal (the API re-reads NEAR mainnet and Ethereum)" }}
                  cli="antibody mainnet-check"
                  label="Read it again through the API"
                />
                <p className="xsmall muted">The API recomputes the check every few minutes; checked_at_unix says when.</p>
                <details className={styles.more}>
                  <summary>The check as data</summary>
                  <div className={styles.moreBody}>
                    <pre className={styles.raw} tabIndex={0} aria-label="The light client check, as the API returned it">
                      {JSON.stringify(check, null, 2)}
                    </pre>
                  </div>
                </details>
              </div>
            </div>
          </Section>

          <Section id="mock" title="What is mock" lede="Nothing on this page.">
            <ul className={styles.items}>
              <li>
                <Icon name="check" size={14} />
                <span>
                  The check reads NEAR mainnet and Ethereum mainnet directly: view calls and <code>eth_getBlockByNumber</code>, nothing signed.
                </span>
              </li>
              <li>
                <Icon name="info" size={14} />
                <span>
                  Mocks exist only in the offline sandbox demo (MOCK OutLayer, DCAP and MPC, TEST keys). Testnet uses the real services.{" "}
                  <Link href="/trust#mock">What is mock, in full</Link>
                </span>
              </li>
              <li>
                <Icon name="alert" size={14} />
                <span>
                  The roadmap&apos;s mainnet path includes external audits. Everything that runs today runs on testnets, where wNEAR has no value.{" "}
                  <Link href="/judges#limits">The limits</Link>
                </span>
              </li>
            </ul>
            <p className="small muted" style={{ marginTop: 14 }}>
              The same explainer: <Ext href={sourceUrl("docs/mainnet.md")}>docs/mainnet.md</Ext>
            </p>
          </Section>
        </>
      )}
    </>
  );
}
