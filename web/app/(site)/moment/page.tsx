import type { Metadata } from "next";
import Link from "next/link";
import { MomentHeader } from "@/components/moment/MomentHeader";
import { OneMoment } from "@/components/moment/OneMoment";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { PageHead, Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { TxLink } from "@/components/ui/TxLink";
import { config, sourceUrl } from "@/lib/config";
import { utc } from "@/lib/format";
import { getMoments, laneName, latestMoment, stepsWithCommit } from "@/lib/moment";
import styles from "./moment.module.css";

export const metadata: Metadata = {
  title: "The one moment",
  description:
    "From A's commit block on NEAR testnet: verified at 18.5 s, T1-confirmed at 29.1 s, paid at 71.0 s, Base Sepolia paused at 87.2 s and Sepolia at 90.6 s. Every step a transaction.",
};

export default async function MomentPage({ searchParams }: { searchParams: Promise<{ run?: string }> }) {
  const sp = await searchParams;
  const { data, live } = await getMoments();
  const m = data.moments.find((x) => x.id === sp.run) ?? latestMoment(data);
  const steps = stepsWithCommit(m);
  return (
    <>
      <PageHead
        crumbs={[{ href: "/moment", label: "The one moment" }]}
        eyebrow="Signature moment"
        title="One sealed proof, verified, paid, and the protocol paused on every chain"
        lede={
          <>
            Agent A, over MCP, committed a sealed proof of a Critical against Acme Lend, our deliberately vulnerable fixture.
            Everything below happened on NEAR testnet, Base Sepolia and Sepolia, with the real services: OutLayer&apos;s TEE
            workers, their TDX quotes verified on NEAR, the MPC network and the Ethereum light client.
          </>
        }
      >
        <Link href="/moment/present" className="btn btn-sm">
          <Icon name="play" size={13} /> Presentation mode
        </Link>
        <a className="btn btn-sm btn-ghost" href={sourceUrl(m.results_file)} target="_blank" rel="noreferrer noopener">
          The results file <Icon name="external" size={12} />
        </a>
      </PageHead>

      <section className="page" aria-labelledby="run-title">
        <div className={`card ${styles.panel}`}>
          <MomentHeader moment={m} live={live} titleId="run-title" />
          <OneMoment moment={m} variant="full" />
        </div>
      </section>

      <Section id="runs" title="The runs" lede="The panel shows the latest full run. Recorded runs come from their results files; a scheduled drill, once a later backend adds it, appears here with no change to the site.">
        <ul className={styles.runs}>
          {data.moments.map((r) => (
            <li key={r.id} className={r.id === m.id ? styles.current : undefined}>
              <Link href={`/moment?run=${encodeURIComponent(r.id)}`} aria-current={r.id === m.id ? "true" : undefined}>
                <span className="mono">{r.id}</span>
              </Link>
              <Pill size="sm" tone={r.kind === "drill" ? "ok" : "neutral"} icon={r.kind === "drill" ? "bolt" : "clock"}>
                {r.kind === "drill" ? "scheduled drill" : "recorded"}
              </Pill>
              <span className="small muted">
                {utc(r.commit.block_ts_ns, "ns")} · program {r.program_id}, commit {r.commit_id} ·{" "}
                {r.all_ok ? "all checks passed" : "checks failed"}
              </span>
            </li>
          ))}
        </ul>
      </Section>

      <Section id="steps" title="Every step, with its transaction" lede={m.clock}>
        <div className="table-wrap">
          <table className="table">
            <thead>
              <tr>
                <th scope="col">From the commit</th>
                <th scope="col">Step</th>
                <th scope="col">Chain</th>
                <th scope="col">What happened</th>
                <th scope="col">Transaction</th>
              </tr>
            </thead>
            <tbody>
              {steps.map((s) => {
                const chainId = s.chain.startsWith("evm:") ? Number(s.chain.slice(4)) : undefined;
                return (
                  <tr key={s.key}>
                    <td className="mono num">{s.seconds_exact.toFixed(3)} s</td>
                    <td>
                      <strong>{s.label}</strong>
                    </td>
                    <td className="small">{laneName(s.chain)}</td>
                    <td className="small muted">{s.detail?.by ? String(s.detail.by) : "—"}</td>
                    <td>
                      <TxLink hash={s.tx} chainId={chainId} href={s.explorer} head={10} />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        <ul className={`small muted ${styles.notes}`}>
          {m.notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
          <li>
            Seconds are the results file&apos;s <code>one_moment.seconds_from_commit</code>; the API serves the same at{" "}
            <a href={`${config.apiBase}/v1/moments`} target="_blank" rel="noreferrer noopener">
              /v1/moments
            </a>
            .
          </li>
        </ul>
      </Section>

      <Section id="check" title="Check it yourself" lede="Your browser asks the API and NEAR directly; the CLI command gives the same answer from a terminal.">
        <div className="grid grid-2">
          <div className="card stack">
            <h3>The commit settled as paid</h3>
            <LiveCheck
              check={{ kind: "api", path: `/v1/commits/${m.commit_id}` }}
              expect={{ path: "row.state", equals: "Done:Paid", says: "state Done:Paid, tier T1" }}
              cli={`antibody status --commit ${m.commit_id}`}
            />
          </div>
          <div className="card stack">
            <h3>Its verify run is T1 on NEAR</h3>
            <LiveCheck
              check={{ kind: "near-view", contract: "bounty.antibody.testnet", method: "get_run", args: { run_id: m.run_id ?? 0 } }}
              expect={{ path: "outcome.Answered.tier", equals: "T1", says: "the run answered, tier T1 (its TDX quote verified on NEAR)" }}
              cli={`antibody status --run ${m.run_id ?? "<run>"}`}
            />
          </div>
        </div>
        <p className="small muted" style={{ marginTop: 16 }}>
          More checks, one per claim: <Link href="/judges">For judges</Link>. The program behind this run, with its chains and rounds:{" "}
          <Link href={`/programs/${m.program_id}`}>program {m.program_id}</Link>.
        </p>
      </Section>
    </>
  );
}
