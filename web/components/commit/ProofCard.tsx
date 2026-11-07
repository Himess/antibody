import type { ReactNode } from "react";
import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { TxLink } from "@/components/ui/TxLink";
import { Sealed } from "@/components/ui/Verdict";
import type { IndexedEvent } from "@/lib/api";
import { sourceUrl } from "@/lib/config";
import { keyFile, sealedLabel, type KeyState } from "./model";
import { PracticeKey } from "./PracticeKey";
import styles from "./commit.module.css";

type Outcome = "paid" | "duplicate" | "known" | "other";

/** The verify run as the page read it: answered, still in flight, failed, or not readable (null). */
type RunStatus = "answered" | "pending" | "failed" | null;

/** Checks that live elsewhere on the commit page: the state under "What it settled", T1 in its tier card. */
const ELSEWHERE = [/^antibody status --commit\b/, /^antibody tee check\b/, /^outlayer-verify\b/];

/** When the run's facts are not there, the page says why rather than guess whether a disclosure opens it. */
function notLoaded(runId: number | null, status: RunStatus): string {
  if (runId === null) return "It has no verification run yet.";
  if (status === "pending") return `Whether a disclosure key will open it depends on the verdict: run ${runId} is still in flight.`;
  if (status === "failed") return `Run ${runId} failed, with no verdict: a disclosure key follows only a verdict that holds a key.`;
  return `Whether a disclosure key can open it is in run ${runId}'s facts, which did not load: reload to see it.`;
}

/** What NEAR does with it next (the project's release wrap, a disclosure), from its run's facts. */
function onChain(outcome: Outcome, wrapD: boolean | null, runId: number | null, status: RunStatus): string {
  if (outcome === "known") return "The project, which declared this issue itself, is never sent it: it reads only proofs it paid for.";
  if (wrapD === null) return notLoaded(runId, status);
  if (wrapD === false) return "Its run's facts carry no disclosure wrap, so no disclosure key opens it on chain.";
  if (outcome === "paid") return "After payment, the project reads it through its release wrap; it opens to everyone at disclosure, one key per tag and round.";
  if (outcome === "duplicate") return "A duplicate is never wrapped for the project; at disclosure it opens with the claim it duplicates.";
  return "The project reads only proofs it paid for; it opens to everyone if its tag is disclosed.";
}

/** A proof whose key is already public off chain: what NEAR itself does with it. */
function publishedOnChain(outcome: Outcome, wrapD: boolean | null, runId: number | null, status: RunStatus): string {
  if (wrapD === null) return notLoaded(runId, status);
  if (wrapD === false) return "On chain, no disclosure key opens it: its run's facts carry no disclosure wrap.";
  if (outcome === "paid") return "On chain, the project reads it through its release wrap, and a disclosure opens it to everyone, one key per tag and round.";
  if (outcome === "duplicate") return "On chain, a duplicate is never wrapped for the project; a disclosure opens it with the claim it duplicates.";
  return "On chain, a disclosure of its tag opens it to everyone.";
}

/** A proof only the TEE can open: who else may read it, and when. */
function sealedAbout(outcome: Outcome, wrapD: boolean | null, runId: number | null, status: RunStatus): string {
  if (outcome === "known") return `Only the TEE could open it. ${onChain(outcome, wrapD, runId, status)}`;
  if (wrapD === false)
    return "Only the TEE could open it before its verdict. Its run's facts carry no disclosure wrap, so no disclosure key opens it: the proof stays sealed, and the project reads only proofs it paid for.";
  if (wrapD === null) {
    const who =
      outcome === "paid"
        ? "Only the TEE could open it before its verdict; after payment, the project reads it through its release wrap."
        : outcome === "duplicate"
          ? "Only the TEE could open it. A duplicate is never wrapped for the project."
          : `Only the TEE ${runId === null || status === "pending" ? "can" : "could"} open it before its verdict, and the project reads only proofs it paid for.`;
    return `${who} ${notLoaded(runId, status)}`;
  }
  return `${outcome === "duplicate" ? "Only the TEE could open it." : "Only the TEE could open it before its verdict."} ${onChain(outcome, wrapD, runId, status)}`;
}

/**
 * The proof card: what can be seen of a sealed proof (its commitment, the blob's hash, the receipt), who can
 * open it today, and how to check each part. It says only what is known: a published key, the run's
 * disclosure wrap (present, absent, or not loaded), or a practice key the browser can try.
 */
export function ProofCard({
  commitId,
  runId,
  receipt,
  commitment,
  blobSha,
  keys,
  wrapD,
  runStatus,
  outcome,
  checks,
  requested,
  disclosed,
  contract,
  programKey,
  hunter,
  programId,
}: {
  commitId: number;
  runId: number | null;
  receipt: string | null;
  commitment: string | null;
  blobSha: string | null;
  /** Who can open it besides the TEE. */
  keys: KeyState;
  /** Whether the verify run's facts carry a disclosure wrap; null when the run has not answered or could not be read. */
  wrapD: boolean | null;
  /** The verify run as read: answered, in flight, failed, or null when it could not be read. */
  runStatus: RunStatus;
  /** Who may read it: a paid finding's project (through its release wrap), never a duplicate's. */
  outcome: Outcome;
  checks: string[];
  requested?: IndexedEvent;
  disclosed?: IndexedEvent;
  contract: string;
  programKey: string | null;
  hunter: string;
  /** With the hunter, it names whose practice try a sealed blob is. */
  programId: number;
}) {
  const seed = blobSha ?? commitment ?? receipt ?? "";
  const isOpen = keys.kind === "disclosed";
  const published = keys.kind === "published" ? keys.fixture : null;
  let about: ReactNode;
  if (isOpen) {
    about = "The verdict can now be recomputed by anyone, and a T2 proof of it made.";
  } else if (published) {
    about = (
      <>
        Not disclosed on chain, but its content key is published in the repository for the T2 study (
        <a href={sourceUrl(keyFile(published))} target="_blank" rel="noreferrer noopener">
          <code>{keyFile(published)}</code>
        </a>
        ): anyone can open it and re-run it. {publishedOnChain(outcome, wrapD, runId, runStatus)}
      </>
    );
  } else if (keys.kind === "practice") {
    about = (
      <>
        <code>antibody try</code> and this site seal a practice try under a public content key, so anyone can open it; a try sealed another way, or
        before that key existed, used a random one. {requested && programKey ? "The check below tells which. " : ""}
        {onChain(outcome, wrapD, runId, runStatus)}
      </>
    );
  } else {
    about = sealedAbout(outcome, wrapD, runId, runStatus);
  }
  const other = checks.filter((c) => !ELSEWHERE.some((re) => re.test(c)));
  return (
    <>
      <div className={styles.proofGrid}>
        <div className={styles.sealedWrap}>
          {isOpen ? (
            <div className="callout callout-ok" role="status">
              <span>
                <strong>Disclosed.</strong> Its tag&apos;s disclosure key was released: anyone can open the sealed blob and re-run it.
              </span>
            </div>
          ) : (
            <Sealed seed={seed} label={outcome === "known" && keys.kind === "sealed" ? "Sealed" : sealedLabel(keys, wrapD, hunter, programId)} lines={5} />
          )}
          <p className="small muted">{about}</p>
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
            {wrapD === true ? (
              "present: if its tag and round are disclosed, their key opens it"
            ) : wrapD === false ? (
              "none: no disclosure key opens it"
            ) : runId === null ? (
              <span className="muted">—</span>
            ) : runStatus === "pending" ? (
              <span className="muted">not yet: run {runId} is in flight</span>
            ) : runStatus === "failed" ? (
              <span className="muted">none: run {runId} failed</span>
            ) : (
              <span className="muted">in run {runId}&apos;s facts, which did not load</span>
            )}
          </dd>
        </dl>
      </div>

      <h3 className={styles.subhead}>How to check it</h3>
      <div className={styles.checks}>
        {commitment ? (
          <LiveCheck
            check={{ kind: "near-view", contract, method: "get_commit", args: { commit_id: commitId } }}
            expect={{ path: "commitment", equals: commitment, says: `near-antibody stores commitment ${commitment.slice(0, 12)}… for commit ${commitId}` }}
            label="Check the commitment on NEAR"
          />
        ) : null}
        {isOpen && runId !== null ? (
          <Command label="Re-run the disclosed proof">{`antibody zk prove --run ${runId} --key <the disclosed content key> --mode none`}</Command>
        ) : null}
        {published && runId !== null ? (
          <>
            <Command label="Open it and re-run it with the published key, from a clone of the repository">
              {`antibody zk prove --run ${runId} --key "$(cat ${keyFile(published)})"${requested ? ` --tx ${requested.tx}` : ""} --mode none`}
            </Command>
            <p className="xsmall muted">It reads the run&apos;s request and blob from NEAR, opens the blob and re-runs it natively: the receipt it prints is the TEE&apos;s.</p>
          </>
        ) : null}
        {keys.kind === "practice" && requested && programKey ? (
          <PracticeKey tx={requested.tx} signer={requested.signer ?? hunter} programKey={programKey} hunter={hunter} blobSha={blobSha} />
        ) : null}
        {other.map((c, i) => (
          <Command key={`${i}-${c}`} label="Check command">
            {c}
          </Command>
        ))}
        <p className="xsmall muted">
          The state and the payout are checked under <a href="#outcome">What it settled</a>; the run&apos;s TDX quote (<code>antibody tee check</code>,{" "}
          <code>outlayer-verify</code>) under <a href="#tiers">Who vouches</a>.
        </p>
      </div>
    </>
  );
}
