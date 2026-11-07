import { Cell } from "@/components/programs/Cell";
import rt from "@/components/programs/programs.module.css";
import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { Pill } from "@/components/ui/Pill";
import { Empty, ErrorState } from "@/components/ui/States";
import { Time } from "@/components/ui/Time";
import { AccountLink } from "@/components/ui/TxLink";
import { t2NotYet, type Result, type T2Panel, type T2Record } from "@/lib/api";
import { sourceUrl } from "@/lib/config";
import { FullHash } from "./FullHash";
import { Md } from "./Md";
import { ZK } from "./trust-data";
import styles from "./checks.module.css";

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : {});
const str = (v: unknown): string | null => (typeof v === "string" ? v : null);
const num = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);

function Outcome({ outcome }: { outcome: string | null | undefined }) {
  if (outcome === "confirmed") return <Pill tone="t2">confirmed</Pill>;
  if (outcome === "contradicts") return <Pill tone="bad">contradicts the TEE</Pill>;
  return <Pill tone="pending">{outcome ?? "unknown"}</Pill>;
}

/**
 * T2 on NEAR: near-t2's pins and records from `/v1/t2`. Until this deployment names a near-t2 contract the
 * API answers an error, and the block says "not yet on testnet"; with records, the same code shows them,
 * each checkable with a view call the browser runs.
 */
export function T2Records({ r }: { r: Result<T2Panel> }) {
  if (t2NotYet(r)) {
    return (
      <div className="stack" style={{ ["--gap" as string]: "12px" }}>
        <p className="row" style={{ ["--gap" as string]: "8px" }}>
          <Pill tone="pending">not yet on testnet</Pill>
          <span className="xsmall muted">
            <code>/v1/t2</code> answers: {r.ok ? "" : r.error}
          </span>
        </p>
        <p className="small" style={{ maxWidth: "72ch" }}>
          near-t2 is built and tested: its unit tests check the public values of four testnet runs, whose content keys are published in{" "}
          <code>fixtures/zk</code>, against near-antibody&apos;s testnet records, and the sandbox run confirms an honest run and records a lying
          MOCK TEE&apos;s contradiction. Its testnet deployment waits for the first proofs from SP1&apos;s prover network. When it holds records, each
          appears here with near-t2&apos;s pins and a view call your browser runs.
        </p>
        <p className="small muted" style={{ maxWidth: "72ch" }}>
          T2 is a validity proof of a disclosed verdict: it claims no privacy, and payouts in the locked deployment do not depend on it.{" "}
          <a href={sourceUrl(ZK.doc)} target="_blank" rel="noreferrer noopener">
            docs/zk.md
          </a>
        </p>
        <Command label="Once near-t2 is on testnet">antibody zk status --run 33</Command>
      </div>
    );
  }
  if (!r.ok) return <ErrorState what="near-t2's records (/v1/t2)" error={r.error} />;

  const t = r.data;
  const pins = obj(t.pins);
  const groth = obj(pins.groth16);
  const vkey = str(pins.program_vkey);
  const stats = obj(t.stats);
  const records: T2Record[] = Array.isArray(t.records) ? t.records : [];
  const latest = records.length ? records[records.length - 1] : null;
  return (
    <div className="stack" style={{ ["--gap" as string]: "16px" }}>
      <dl className={styles.mini}>
        <div>
          <dt>near-t2</dt>
          <dd>
            <AccountLink account={t.contract} site={false} />
          </dd>
        </div>
        <div>
          <dt>Program vkey (pinned)</dt>
          <dd className="stack" style={{ ["--gap" as string]: "4px" }}>
            <FullHash value={vkey} label="program vkey" />
            {vkey ? (
              vkey.toLowerCase() === ZK.programVkey ? (
                <Pill tone="ok" size="sm">
                  the docker build&apos;s vkey
                </Pill>
              ) : (
                <Pill tone="bad" size="sm">
                  not the docker build&apos;s vkey
                </Pill>
              )
            ) : null}
          </dd>
        </div>
        <div>
          <dt>Groth16 key</dt>
          <dd>
            {groth.is_sp1_v6_10_0 === true ? (
              <Pill tone="ok" size="sm">
                SP1 {ZK.sp1}&apos;s, built in
              </Pill>
            ) : groth.is_sp1_v6_10_0 === false ? (
              <Pill tone="bad" size="sm">
                not SP1 {ZK.sp1}&apos;s key
              </Pill>
            ) : (
              <span className="muted">—</span>
            )}
          </dd>
        </div>
        <div>
          <dt>Reads records from</dt>
          <dd>
            <AccountLink account={str(pins.antibody)} site={false} />
          </dd>
        </div>
        <div>
          <dt>Records</dt>
          <dd className="num">
            {num(stats.records) ?? records.length} in all · {num(stats.confirmed) ?? "—"} confirmed · {num(stats.contradicted) ?? "—"} contradicted
          </dd>
        </div>
      </dl>
      {records.length ? (
        <div className={`table-wrap ${rt.cardsWrap}`}>
          <table className={`table ${rt.rtable}`}>
            <caption className="visually-hidden">near-t2&apos;s records: each run&apos;s outcome and the receipt its re-run gave</caption>
            <thead>
              <tr>
                <th scope="col">Run</th>
                <th scope="col">Outcome</th>
                <th scope="col">Receipt (re-run)</th>
                <th scope="col">Recorded</th>
              </tr>
            </thead>
            <tbody>
              {records.map((rec, i) => (
                <tr key={`${rec.run_id ?? i}`}>
                  <Cell label="Run" lead className="num">
                    <strong>Run {String(rec.run_id ?? "—")}</strong>
                  </Cell>
                  <Cell label="Outcome">
                    <Outcome outcome={rec.outcome} />
                    {rec.differ?.length ? <div className="xsmall muted">differ: {rec.differ.join(", ")}</div> : null}
                  </Cell>
                  <Cell label="Receipt">
                    <Hash value={rec.receipt_sha256 ?? null} head={12} label="receipt sha256" />
                  </Cell>
                  <Cell label="Recorded">{num(rec.at) ? <Time t={num(rec.at)} /> : <span className="muted">—</span>}</Cell>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <Empty>near-t2 is deployed and holds no record yet.</Empty>
      )}
      <div className={styles.grid2}>
        <LiveCheck
          check={{ kind: "near-view", contract: t.contract, method: "get_pins", args: {} }}
          expect={{ path: "program_vkey", equals: ZK.programVkey, says: `program_vkey is the docker build's, ${ZK.programVkey.slice(0, 12)}…` }}
          cli={`antibody view --contract ${t.contract} --method get_pins`}
          label="Read near-t2's pins"
        />
        {latest?.run_id !== undefined && latest?.run_id !== null ? (
          <LiveCheck
            check={{ kind: "near-view", contract: t.contract, method: "get_record", args: { run_id: String(latest.run_id) } }}
            expect={{ path: "outcome", equals: latest.outcome, says: `run ${latest.run_id}'s record is ${latest.outcome}` }}
            cli={`antibody zk status --run ${latest.run_id}`}
            label={`Read run ${latest.run_id}'s record`}
          />
        ) : null}
      </div>
      {t.note ? (
        <p className="small muted">
          <Md text={t.note} />
        </p>
      ) : null}
    </div>
  );
}
