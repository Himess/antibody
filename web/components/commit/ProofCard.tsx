import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { TxLink } from "@/components/ui/TxLink";
import { Sealed } from "@/components/ui/Verdict";
import type { IndexedEvent } from "@/lib/api";
import styles from "./commit.module.css";

/**
 * The proof card: what can be seen of a sealed proof (its commitment, the blob's hash, the receipt) and how
 * to check each, while its content stays unreadable until disclosure.
 */
export function ProofCard({
  commitId,
  runId,
  receipt,
  commitment,
  blobSha,
  sealed,
  wrapD,
  outcome,
  checks,
  requested,
  disclosed,
  contract,
}: {
  commitId: number;
  runId: number | null;
  receipt: string | null;
  commitment: string | null;
  blobSha: string | null;
  sealed: boolean;
  /** Whether the verify run's facts carry a disclosure wrap; null when the run could not be read. */
  wrapD: boolean | null;
  /** Who may read it: a paid finding's project (through its release wrap), never a duplicate's. */
  outcome: "paid" | "duplicate" | "known" | "other";
  checks: string[];
  requested?: IndexedEvent;
  disclosed?: IndexedEvent;
  contract: string;
}) {
  const seed = blobSha ?? commitment ?? receipt ?? "";
  return (
    <>
      <div className={styles.proofGrid}>
        <div className={styles.sealedWrap}>
          {sealed ? (
            <Sealed seed={seed} label="Sealed until disclosure" lines={5} />
          ) : (
            <div className="callout callout-ok" role="status">
              <span>
                <strong>Disclosed.</strong> Its tag&apos;s disclosure key was released: anyone can open the sealed blob and re-run it.
              </span>
            </div>
          )}
          <p className="small muted">
            {!sealed
              ? "The verdict can now be recomputed by anyone, and a T2 proof of it made."
              : wrapD === false
                ? "Only the TEE could open it before its verdict. Its run's facts carry no disclosure wrap, so no disclosure key opens it: the proof stays sealed, and the project reads only proofs it paid for."
                : outcome === "paid"
                  ? "Only the TEE could open it before its verdict; after payment, the project reads it through its release wrap. It opens to everyone at disclosure, one key per tag and round."
                  : outcome === "duplicate"
                    ? "Only the TEE could open it. A duplicate is never wrapped for the project; at disclosure it opens with the claim it duplicates."
                    : outcome === "known"
                      ? "Only the TEE could open it. The project, which declared this issue itself, is never sent it: it reads only proofs it paid for."
                      : "Only the TEE could open it before its verdict, and the project reads only proofs it paid for. It opens to everyone when its tag is disclosed."}
          </p>
          {disclosed ? (
            <p className="small">
              Disclosed in <TxLink hash={disclosed.tx} href={disclosed.explorer} />
            </p>
          ) : null}
        </div>
        <dl className={`kv ${styles.facts}`}>
          <dt>Receipt sha256</dt>
          <dd>
            <Hash value={receipt} head={16} label="receipt sha256" />
            <div className="xsmall muted">The hash of the verifier&apos;s full receipt: any re-run of the same input reproduces it, byte for byte.</div>
          </dd>
          <dt>Commitment</dt>
          <dd>
            <Hash value={commitment} head={16} label="commitment" />
            <div className="xsmall muted">Committed before the reveal: a hash of the program, the round, the hunter and the blob&apos;s sha256, so it cannot be copied.</div>
          </dd>
          <dt>Blob sha256</dt>
          <dd>{blobSha ? <Hash value={blobSha} head={16} label="sealed blob sha256" /> : <span className="small muted">no separate blob: it is inline in the request</span>}</dd>
          <dt>Where it lives</dt>
          <dd>
            {requested ? (
              <>
                <TxLink hash={requested.tx} href={requested.explorer} />
                <div className="xsmall muted">The reveal: its request names the sealed blob, which <code>antibody program read --api</code> fetches.</div>
              </>
            ) : (
              <span className="muted">—</span>
            )}
          </dd>
          <dt>Disclosure wrap</dt>
          <dd className="small">
            {wrapD === true ? "present: one key per tag and round opens it at disclosure" : wrapD === false ? "none in its run's facts" : <span className="muted">—</span>}
          </dd>
        </dl>
      </div>

      <h3 className={styles.subhead}>How to check it</h3>
      <div className={styles.checks}>
        {checks.length ? (
          checks.map((c, i) => (
            <Command key={`${i}-${c}`} label={c.startsWith("antibody tee") ? "T1 check (the request id is found under T1 below)" : "Check command"}>
              {c}
            </Command>
          ))
        ) : (
          <Command label="Check command">{`antibody status --commit ${commitId}`}</Command>
        )}
        {!sealed && runId !== null ? (
          <Command label="Re-run the disclosed proof">{`antibody zk prove --run ${runId} --key <the disclosed content key> --mode none`}</Command>
        ) : null}
        {commitment ? (
          <LiveCheck
            check={{ kind: "near-view", contract, method: "get_commit", args: { commit_id: commitId } }}
            expect={{ path: "commitment", equals: commitment, says: `near-antibody stores commitment ${commitment.slice(0, 12)}… for commit ${commitId}` }}
            label="Read the commit from NEAR"
          />
        ) : null}
      </div>
    </>
  );
}
