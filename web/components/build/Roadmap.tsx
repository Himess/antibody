import { Suspense } from "react";
import { Pill } from "@/components/ui/Pill";
import { api, t2NotYet } from "@/lib/api";
import { DocLink } from "./DocLink";
import { Done, NotYet } from "./Labels";
import styles from "./build.module.css";

type Item = { title: string; detail?: string; parts?: { done: boolean; text: string }[]; t2?: boolean };

/** README's roadmap, in its order. Only T2 has parts the README marks done. */
const ROADMAP: Item[] = [
  {
    title: "Antibody for pull requests",
    detail:
      "A CI check, then a repository app, on repositories whose owner installs it. Immune memory: every paid proof of a program re-runs against each pull request's build (sealed ones inside the TEE, where the result is all anyone sees), and the check goes red if one still breaks a rule. A bounty window per pull request: a fork-mode round for its build with a small escrow, announced to agents; when it ends, the check reads “held N hours, X at stake, 0 valid findings”, or blocks the merge on a valid Critical. Sealed proofs never appear in public logs.",
  },
  { title: "NEAR contracts as a second target VM", detail: "Proofs against NEAR wasm contracts from committed, proven state." },
  {
    title: "T2: validity proofs of disclosed verdicts, verified on NEAR",
    detail: "Later, zk by default above a cap. A validity proof of a disclosed verdict: no privacy claim, and no payout in the locked deployment depends on it.",
    t2: true,
    parts: [
      { done: true, text: "The feasibility study: the verifier runs in SP1 6.10.0 and reproduces the TEE's receipts in about 30 million cycles." },
      {
        done: true,
        text: "near-t2, which checks a Groth16 proof in about 34 Tgas and compares it with near-antibody's records; antibody zk prove, submit and status; the API; a prover service for every practice try.",
      },
      { done: false, text: "The four testnet runs proven on SP1's prover network, and near-t2 on testnet." },
    ],
  },
  { title: "Multi-block proofs with a bounded time warp", detail: "Interest, TWAP and time-dependent bugs." },
  { title: "The next Ethereum execution rules", detail: "An EVM upgrade and fork-switch execution." },
  { title: "L2 state verification", detail: "Through MPC foreign-chain verification or L2 light clients; more chains for pause and release." },
  { title: "Several independent TEE operators by default", detail: "A strict “no quote, no pay” mode once attestation is reliable." },
  { title: "One sub-account per program", detail: "Isolated escrow, keys and signing path." },
  { title: "Payouts in any asset through NEAR Intents", detail: "On mainnet." },
  { title: "Agent capital", detail: "Backer vaults fund bonds for agents with verified records; bond discounts from proof-based reputation." },
  { title: "Security jobs on NEAR's agent market", detail: "Whose acceptance test is an Antibody verdict." },
  { title: "A public verdict registry as a NEAR primitive", detail: "For insurance, lending markets and DAOs." },
  { title: "Continuous programs for live protocols", detail: "In mainnet mode, with honest latency limits." },
  { title: "Private and invite-only programs", detail: "Optional checks before large withdrawals; safe-harbor templates." },
  { title: "Post-quantum readiness", detail: "ML-DSA accounts and hybrid ML-KEM sealing for long embargoes." },
  { title: "The mainnet path", detail: "External audits, Antibody's own adapters under an Antibody program, coverage partners, the fee model in production." },
  { title: "A paid tier for the drafter, rule strength and analytics" },
];

/**
 * near-t2's status on this deployment, from /v1/t2. Until the API names a near-t2 contract it answers an
 * error, shown as "not yet on testnet", and so is a near-t2 with no record yet (never a zero count); once
 * records exist, the same code counts them.
 */
export async function T2Status() {
  const r = await api.t2();
  if (r.ok) {
    const records = Array.isArray(r.data?.records) ? r.data.records : [];
    if (!records.length) return <Pill tone="pending" size="sm">not yet on testnet</Pill>;
    const confirmed = records.filter((x) => x?.outcome === "confirmed").length;
    const contradicts = records.filter((x) => x?.outcome === "contradicts").length;
    return (
      <Pill tone="t2" size="sm" title="near-t2's records on testnet (/v1/t2)">
        {confirmed} run{confirmed === 1 ? "" : "s"} confirmed on testnet
        {contradicts ? ` · ${contradicts} contradicted` : ""}
      </Pill>
    );
  }
  if (t2NotYet(r)) return <Pill tone="pending" size="sm">not yet on testnet</Pill>;
  return (
    <Pill tone="warn" size="sm" title={r.error}>
      testnet status unavailable
    </Pill>
  );
}

export function Roadmap() {
  return (
    <ol className={styles.roadmap} aria-label="Roadmap" role="list">
      {ROADMAP.map((item, i) => (
        <li key={item.title} className={styles.rmItem}>
          <span className={styles.rmNo} aria-hidden>
            {String(i + 1).padStart(2, "0")}
          </span>
          <h3 className={styles.rmTitle}>{item.title}</h3>
          <span className={`${styles.labelRow} ${styles.rmStatus}`}>
            {item.parts?.some((p) => p.done) ? <Pill tone="info" size="sm">partly done</Pill> : <NotYet />}
            {item.t2 ? (
              <Suspense fallback={<Pill tone="pending" size="sm">reading testnet…</Pill>}>
                <T2Status />
              </Suspense>
            ) : null}
          </span>
          {item.detail || item.parts || item.t2 ? (
            <div className={styles.rmBody}>
              {item.detail ? <p className={styles.rmText}>{item.detail}</p> : null}
              {item.parts ? (
                <ul className={styles.rmParts}>
                  {item.parts.map((p) => (
                    <li key={p.text}>
                      {p.done ? <Done /> : <NotYet>next</NotYet>}
                      <span>{p.text}</span>
                    </li>
                  ))}
                </ul>
              ) : null}
              {item.t2 ? (
                <p className="xsmall muted">
                  How T2 works and what it checks: <DocLink path="docs/zk.md">docs/zk.md</DocLink>.
                </p>
              ) : null}
            </div>
          ) : null}
        </li>
      ))}
    </ol>
  );
}
