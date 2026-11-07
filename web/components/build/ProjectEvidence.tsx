import { Empty } from "@/components/ui/States";
import { TxLink } from "@/components/ui/TxLink";
import { getMoments, laneName, latestMoment } from "@/lib/moment";
import { deployment } from "./deployment";
import { OfflineCopy, Recorded } from "./Labels";
import { Check } from "./Check";
import styles from "./build.module.css";

/** The recorded pause: when it was asked for, the payment, and each chain's pause, each with its transaction. */
export async function PauseEvidence() {
  const { data, live } = await getMoments();
  const m = latestMoment(data);
  const steps = Array.isArray(m?.steps) ? m.steps : [];
  const pauses = steps.filter((s) => typeof s.chain === "string" && s.chain.startsWith("evm:")).sort((a, b) => a.seconds - b.seconds);
  if (!m || !pauses.length) return <Empty>No recorded pause to show yet.</Empty>;
  const asked = steps.find((s) => s.key === "pause_asked") ?? steps.find((s) => s.key === "permits_signed");
  const paid = steps.find((s) => s.key === "paid");
  const before = asked && paid ? asked.seconds < paid.seconds : false;
  return (
    <>
      <div className={styles.labelRow}>
        <Recorded />
        {live ? null : <OfflineCopy />}
        <span className="xsmall muted">
          program {m.program_id} (Acme Lend, our fixture), commit {m.commit_id}: seconds from its commit block
        </span>
      </div>
      <ul className={styles.txList}>
        {asked ? (
          <li>
            <span>
              {asked.key === "pause_asked" ? "Pause asked for" : "Pause permits signed"} <span className="num">{asked.seconds.toFixed(1)} s</span>
            </span>
            <TxLink hash={asked.tx} href={asked.explorer} />
          </li>
        ) : null}
        {paid ? (
          <li>
            <span>
              Paid <span className="num">{paid.seconds.toFixed(1)} s</span>
            </span>
            <TxLink hash={paid.tx} href={paid.explorer} />
          </li>
        ) : null}
        {pauses.map((s) => (
          <li key={s.key}>
            <span>
              {laneName(s.chain)} paused <span className="num">{s.seconds.toFixed(1)} s</span>
            </span>
            <TxLink hash={s.tx} chainId={Number(s.chain.slice(4))} href={s.explorer} />
          </li>
        ))}
      </ul>
      <p className="xsmall muted">
        {before ? <>The pause was asked for before the payment. </> : null}
        NEAR steps are timed at their blocks; each pause when the relayer got its chain&apos;s receipt.
      </p>
    </>
  );
}

/** guard.antibody.testnet's state, read from NEAR: frozen on A's Critical. */
export async function GuardCheck() {
  const d = await deployment();
  return (
    <Check
      check={{ kind: "near-view", contract: d.guard, method: "get_state" }}
      expect={{ path: "frozen.commit_id", equals: 13, says: "frozen on commit 13 (A's T1-confirmed Critical), slot 0" }}
      cli={`antibody view --contract ${d.guard} --method get_state`}
    />
  );
}

/** Program 5's pinned fees, read from near-antibody. */
export async function FeeCheck() {
  const d = await deployment();
  return (
    <Check
      check={{ kind: "near-view", contract: d.antibody, method: "get_program", args: { program_id: 5 } }}
      expect={{
        path: "params.protocol_fee_bps",
        equals: 0,
        says: "program 5 pins its payout fee in its params: 0 bps on this testnet fixture (the release fee is params.release_fee)",
      }}
      cli={`antibody view --contract ${d.antibody} --method get_program --args '{"program_id":5}'`}
    />
  );
}
