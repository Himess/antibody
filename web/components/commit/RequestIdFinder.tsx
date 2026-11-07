"use client";

import { sha256 } from "@noble/hashes/sha2.js";
import { useState } from "react";
import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { Pill } from "@/components/ui/Pill";
import { config, sourceUrl } from "@/lib/config";
import { shortHash } from "@/lib/format";
import { nearArchival, nearRpc } from "@/lib/near/client";
import { events as txEvents, type TxOutcome } from "@/lib/near/rpc";
import { isObj, runRequestOf } from "./model";
import styles from "./commit.module.css";

/** What OutLayer's `execution_requested` event in the requesting transaction says about the run. */
interface Found {
  requestId: number;
  /** sha256 of the request's `input_data`, computed here: near-antibody records it as the run's request_sha256. */
  inputSha: string | null;
  /** The verifier build the request names (its wasm sha256). */
  wasm: string | null;
  /** The sha256 of the sealed blob the request names, when the blob is not inline. */
  blobSha: string | null;
}

const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");

function parse(out: TxOutcome): Found | null {
  const r = runRequestOf(txEvents(out));
  if (!r) return null;
  let blobSha: string | null = null;
  if (r.input) {
    try {
      const req = JSON.parse(r.input) as unknown;
      if (isObj(req) && isObj(req.blob) && typeof req.blob.sha256 === "string") blobSha = req.blob.sha256;
    } catch {
      blobSha = null;
    }
  }
  return {
    requestId: r.requestId,
    inputSha: r.input === null ? null : hex(sha256(new TextEncoder().encode(r.input))),
    wasm: r.wasm,
    blobSha,
  };
}

const PLACEHOLDER = "<the run's OutLayer request>";

/**
 * Finds the OutLayer request behind a run in the transaction that requested it (as `antibody tee confirm
 * --tx` does), so the T1 commands below carry the real id. The browser reads the transaction from a NEAR
 * archival RPC and checks the request against near-antibody's record of the run. A request id already
 * recorded in a results file (`known`) fills the commands from the start.
 */
export function RequestIdFinder({
  tx,
  signer,
  requestSha,
  blobSha,
  wasmSha,
  known,
}: {
  tx: string;
  signer: string;
  requestSha?: string | null;
  blobSha?: string | null;
  /** The verifier build the deployment records (/health), to compare with the one the request names. */
  wasmSha?: string | null;
  /** The request id a results file records for this run, and that file. */
  known?: { requestId: number; source: string } | null;
}) {
  const [st, setSt] = useState<{ status: "idle" | "running" | "done" | "error"; found?: Found; error?: string }>({ status: "idle" });

  const find = async () => {
    setSt({ status: "running" });
    try {
      let out: TxOutcome;
      try {
        out = await nearArchival().txStatus(tx, signer, "FINAL");
      } catch {
        // A transaction from the last few seconds may not be on the archival nodes yet.
        out = await nearRpc().txStatus(tx, signer, "FINAL");
      }
      const found = parse(out);
      if (!found) throw new Error("this transaction has no OutLayer execution_requested event");
      setSt({ status: "done", found });
    } catch (e) {
      setSt({ status: "error", error: e instanceof Error ? e.message : String(e) });
    }
  };

  const f = st.found;
  const requestId = f?.requestId ?? known?.requestId ?? null;
  const id = requestId !== null ? String(requestId) : PLACEHOLDER;
  const reqHolds = f?.inputSha && requestSha ? f.inputSha === requestSha.toLowerCase() : null;
  const blobHolds = f?.blobSha && blobSha ? f.blobSha.toLowerCase() === blobSha.toLowerCase() : null;
  const wasmHolds = f?.wasm && wasmSha ? f.wasm.toLowerCase() === wasmSha.toLowerCase() : null;

  return (
    <div className={styles.finder}>
      <div className={styles.finderHead}>
        {/* aria-disabled, not disabled, while it works: a disabled button would drop keyboard focus to the page. */}
        <button
          type="button"
          className="btn btn-sm"
          onClick={() => {
            if (st.status !== "running") void find();
          }}
          aria-disabled={st.status === "running" || undefined}
          aria-busy={st.status === "running"}
        >
          <Icon name={st.status === "running" ? "refresh" : "play"} size={13} />
          {st.status === "running" ? "Reading…" : st.status === "done" ? "Read again" : "Find the OutLayer request"}
        </button>
        <span className="xsmall muted mono">
          tx {shortHash(tx, 10)} · NEAR archival RPC
        </span>
      </div>
      <div aria-live="polite" className={styles.finderOut}>
        {st.status === "done" && f ? (
          <ul className={styles.finderList}>
            <li>
              <Pill tone="ok" size="sm">
                found
              </Pill>{" "}
              OutLayer request <strong className="mono">{f.requestId}</strong>, in this run&apos;s requesting transaction.
            </li>
            {reqHolds !== null ? (
              <li>
                <Pill tone={reqHolds ? "ok" : "bad"} size="sm">
                  {reqHolds ? "holds" : "differs"}
                </Pill>{" "}
                Its input hashes to the run&apos;s request sha256 <Hash value={f.inputSha} head={12} label="the request's sha256, computed in your browser" /> (computed in your
                browser).
              </li>
            ) : null}
            {blobHolds !== null ? (
              <li>
                <Pill tone={blobHolds ? "ok" : "bad"} size="sm">
                  {blobHolds ? "holds" : "differs"}
                </Pill>{" "}
                It names the commit&apos;s sealed blob, sha256 <Hash value={f.blobSha} head={12} label="the blob sha256 the request names" />.
              </li>
            ) : null}
            {f.wasm ? (
              <li>
                {wasmHolds !== null ? (
                  <Pill tone={wasmHolds ? "ok" : "bad"} size="sm">
                    {wasmHolds ? "holds" : "differs"}
                  </Pill>
                ) : (
                  <Pill size="sm">verifier</Pill>
                )}{" "}
                It runs the verifier wasm with sha256 <Hash value={f.wasm} head={12} label="the verifier wasm sha256 the request names" />
                {wasmHolds !== null ? ", the build this deployment records" : ""}.
              </li>
            ) : null}
          </ul>
        ) : st.status === "error" ? (
          <div className="stack" style={{ ["--gap" as string]: "4px" }}>
            <Pill tone="warn" size="sm">
              could not read
            </Pill>
            <p className="xsmall mono">{st.error}</p>
            <p className="xsmall muted">
              Public RPCs rate-limit; try again in a few seconds.{requestId === null ? " The placeholder below stays until the request is found." : ""}
            </p>
          </div>
        ) : null}
      </div>
      <Command label="T1 check">{`antibody tee check --request-id ${id}`}</Command>
      {requestId !== null ? <Command label="OutLayer's own check">{`outlayer-verify job ${requestId} --network ${config.network}`}</Command> : null}
      {known && !f ? (
        <p className="xsmall muted">
          Request {known.requestId}, as{" "}
          <a href={sourceUrl(known.source)} target="_blank" rel="noreferrer noopener">
            {known.source.split("/").pop()}
          </a>{" "}
          records it; the button reads it from the transaction itself.
        </p>
      ) : null}
    </div>
  );
}
