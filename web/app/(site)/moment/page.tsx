import type { Metadata } from "next";
import Link from "next/link";
import { MomentHeader } from "@/components/moment/MomentHeader";
import { OneMoment } from "@/components/moment/OneMoment";
import { Cell } from "@/components/programs/Cell";
import cards from "@/components/programs/programs.module.css";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { PageHead, Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { TxLink } from "@/components/ui/TxLink";
import { config, sourceUrl } from "@/lib/config";
import { cap, utc } from "@/lib/format";
import { committer, getMoments, laneName, possessive, recordedMoment, stepsWithCommit, timedNote, txNote } from "@/lib/moment";
import styles from "./moment.module.css";

export const metadata: Metadata = {
  title: "The one moment",
  description:
    "From A's commit block on NEAR testnet: verified at 18.5 s, T1-confirmed at 29.1 s, paid at 71.0 s, and both chains paused by 90.6 s. Every step links to its transaction.",
};

export default async function MomentPage({ searchParams }: { searchParams: Promise<{ run?: string }> }) {
  const sp = await searchParams;
  const { data, live } = await getMoments();
  // The recorded full run unless a run is picked: drills are listed below it, each labelled.
  const m = data.moments.find((x) => x.id === sp.run) ?? recordedMoment(data);
  const steps = stepsWithCommit(m);
  const { name } = committer(m);
  const asked = steps.find((s) => s.key === "pause_asked");
  const paid = steps.find((s) => s.key === "paid");
  return (
    <>
      <PageHead
        crumbs={[{ href: "/moment", label: "The one moment" }]}
        eyebrow="Recorded on testnet · from its results file"
        title="One sealed proof, verified, paid, and the protocol paused on every chain"
        lede={
          <>
            Recorded on NEAR testnet, Base Sepolia and Sepolia with the real services: OutLayer&apos;s TEE workers, their TDX quotes verified on
            NEAR, the MPC network and the Ethereum light client.
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

      <section className={`page ${styles.panelWrap}`} aria-labelledby="run-title">
        <div className={`card ${styles.panel}`}>
          <MomentHeader moment={m} live={live} titleId="run-title" />
          <OneMoment moment={m} variant="full" />
        </div>
      </section>

      <Section id="runs" title="The runs" lede="Each run of the one moment, from its results file: the recorded full run, then the automated drills that run the same loop on testnet. The panel shows the recorded run, or the run you pick.">
        <ul className={styles.runs}>
          {data.moments.map((r) => (
            <li key={r.id} className={r.id === m.id ? styles.current : undefined}>
              <Link href={`/moment?run=${encodeURIComponent(r.id)}`} aria-current={r.id === m.id ? "true" : undefined}>
                <span className="mono">{r.id}</span>
              </Link>
              <Pill size="sm" tone={r.kind === "drill" ? "ok" : "neutral"} icon={r.kind === "drill" ? "bolt" : "clock"}>
                {r.kind === "drill" ? "automated drill" : "recorded"}
              </Pill>
              {r.id === m.id ? (
                <Pill size="sm" tone="accent" icon="eye">
                  in the panel
                </Pill>
              ) : null}
              <span className="small muted">
                {utc(r.commit.block_ts_ns, "ns")} · program {r.program_id}, commit {r.commit_id} ·{" "}
                {r.all_ok ? "all checks passed" : "checks failed"}
              </span>
            </li>
          ))}
        </ul>
      </Section>

      <Section
        id="steps"
        title="Every step, with its transaction"
        lede={`Seconds from ${possessive(name)} commit block. NEAR steps are timed at their blocks; each pause when the relayer got its chain's receipt, which comes after the block an explorer shows.`}
      >
        <div className={`table-wrap ${cards.cardsWrap}`}>
          <table className={`table ${cards.rtable}`}>
            <caption className="visually-hidden">The one moment&apos;s steps, in seconds from the commit&apos;s block, each with its transaction</caption>
            <thead>
              <tr>
                <th scope="col">From the commit · step</th>
                <th scope="col">Chain</th>
                <th scope="col">What happened</th>
                <th scope="col">Transaction</th>
              </tr>
            </thead>
            <tbody>
              {steps.map((s) => {
                const chainId = s.chain.startsWith("evm:") ? Number(s.chain.slice(4)) : undefined;
                const timed = timedNote(s);
                const note = txNote(s, steps, name);
                return (
                  <tr key={s.key}>
                    <Cell label="Step" lead>
                      <span className={styles.stepHead}>
                        <span className={`${styles.secs} mono num`}>{s.seconds_exact.toFixed(3)} s</span>
                        <strong>{s.label}</strong>
                      </span>
                      {timed ? <span className={styles.sub}>{timed}</span> : null}
                    </Cell>
                    <Cell label="Chain">
                      <span className={styles.chain}>{laneName(s.chain)}</span>
                    </Cell>
                    <Cell label="What happened">
                      <span className="small muted">{s.detail?.by ? cap(String(s.detail.by)) : "—"}</span>
                    </Cell>
                    <Cell label="Transaction">
                      <TxLink hash={s.tx} chainId={chainId} href={s.explorer} head={10} />
                      {note ? <span className={styles.sub}>{note}</span> : null}
                    </Cell>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        <ul className={`small muted ${styles.notes}`}>
          {asked && paid && asked.seconds_exact < paid.seconds_exact ? <li>The pause is asked for before the payment.</li> : null}
          <li>
            Seconds are the results file&apos;s <code>one_moment.seconds_from_commit</code>
            {asked ? (
              <>
                , except Pause asked, which the API reads from the indexer&apos;s <code>pause_ticket</code> event
              </>
            ) : null}
            ; the API serves them all at{" "}
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
              expect={{ path: "row.state", equals: "Done:Paid", says: "state Done:Paid" }}
              cli={`antibody status --commit ${m.commit_id}`}
            />
          </div>
          <div className="card stack">
            <h3>Its verification run is T1 on NEAR</h3>
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
