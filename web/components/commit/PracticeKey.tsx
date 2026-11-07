"use client";

import { sha256 } from "@noble/hashes/sha2.js";
import { useState } from "react";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { Pill } from "@/components/ui/Pill";
import { nearArchival, nearRpc } from "@/lib/near/client";
import { events as txEvents, type TxOutcome } from "@/lib/near/rpc";
import { fromHex, toHex } from "@/lib/practice/bytes";
import { contentFromTx, parseLocation } from "@/lib/practice/fastfs";
import { openWithKey, practiceContentKey, SealError } from "@/lib/practice/seal";
import { isObj, runRequestOf } from "./model";
import styles from "./commit.module.css";

type State =
  | { status: "idle" | "running" }
  | { status: "opens"; envelopeBytes: number; envelopeSha: string; reportBytes: number }
  | { status: "closed" }
  | { status: "error"; error: string };

async function read(hash: string, signer: string): Promise<TxOutcome> {
  try {
    return await nearArchival().txStatus(hash, signer, "FINAL");
  } catch {
    // A transaction from the last few seconds may not be on the archival nodes yet.
    return await nearRpc().txStatus(hash, signer, "FINAL");
  }
}

/**
 * Whether a practice commit opens with the public practice content key (blob.rs `practice_content_key`):
 * the browser reads the reveal's request from NEAR, the sealed blob it names from FastFS (checked against
 * the commit's blob sha256), and tries the key. `antibody try` and the site's try seal with it; a try sealed
 * another way, or before that key existed, does not open.
 */
export function PracticeKey({ tx, signer, programKey, hunter, blobSha }: { tx: string; signer: string; programKey: string; hunter: string; blobSha: string | null }) {
  const [st, setSt] = useState<State>({ status: "idle" });
  const formula = `sha256("antibody/practice/public-content-key/v1|${programKey}|${hunter}")`;

  const check = async () => {
    setSt({ status: "running" });
    try {
      const r = runRequestOf(txEvents(await read(tx, signer)));
      if (!r?.input) throw new Error("the reveal transaction carries no OutLayer request");
      const req = JSON.parse(r.input) as unknown;
      const ref = isObj(req) ? req.blob : null;
      let blob: Uint8Array;
      if (isObj(ref) && typeof ref.inline === "string") {
        blob = fromHex(ref.inline);
      } else {
        const loc = isObj(ref) && typeof ref.near_tx === "string" ? parseLocation(ref.near_tx) : null;
        if (!loc) throw new Error("the request names no sealed blob");
        blob = contentFromTx(await read(loc.hash, loc.signer));
      }
      const sha = toHex(sha256(blob));
      if (blobSha && sha !== blobSha.toLowerCase()) throw new Error(`the blob read hashes to ${sha.slice(0, 12)}…, not to the commit's blob sha256`);
      try {
        const p = openWithKey(blob, practiceContentKey(programKey, hunter), programKey);
        setSt({ status: "opens", envelopeBytes: p.envelope.length, envelopeSha: toHex(sha256(p.envelope)), reportBytes: p.report.length });
      } catch (e) {
        if (e instanceof SealError && e.kind === "crypto") setSt({ status: "closed" });
        else throw e;
      }
    } catch (e) {
      setSt({ status: "error", error: e instanceof Error ? e.message : String(e) });
    }
  };

  return (
    <div className={styles.finder}>
      <div className={styles.finderHead}>
        {/* aria-disabled, not disabled, while it reads: a disabled button would drop keyboard focus to the page. */}
        <button
          type="button"
          className="btn btn-sm"
          onClick={() => {
            if (st.status !== "running") void check();
          }}
          aria-disabled={st.status === "running" || undefined}
          aria-busy={st.status === "running"}
        >
          <Icon name={st.status === "running" ? "refresh" : "key"} size={13} />
          {st.status === "running" ? "Reading…" : st.status === "idle" ? "Try the public practice key" : "Try again"}
        </button>
        <span className="xsmall muted mono">reveal and blob · NEAR archival RPC</span>
      </div>
      <p className={`xsmall muted ${styles.wrapAny}`}>
        Key: <code>{formula}</code>
      </p>
      <div aria-live="polite" className={styles.finderOut}>
        {st.status === "opens" ? (
          <ul className={styles.finderList}>
            <li>
              <Pill tone="ok" size="sm">
                opens
              </Pill>{" "}
              Sealed with the public practice key: anyone can open it. Opened in your browser: a {st.envelopeBytes}-byte proof, sha256{" "}
              <Hash value={st.envelopeSha} head={12} label="the opened proof's sha256" />
              {st.reportBytes ? `, and a ${st.reportBytes}-byte report` : ", and no report"}.
            </li>
          </ul>
        ) : st.status === "closed" ? (
          <ul className={styles.finderList}>
            <li>
              <Pill size="sm">does not open</Pill> The blob is the commit&apos;s, and this key does not open it: it was sealed with a random content key.
            </li>
          </ul>
        ) : st.status === "error" ? (
          <div className="stack" style={{ ["--gap" as string]: "4px" }}>
            <Pill tone="warn" size="sm">
              could not check
            </Pill>
            <p className="xsmall mono">{st.error}</p>
            <p className="xsmall muted">Public RPCs rate-limit; try again in a few seconds.</p>
          </div>
        ) : null}
      </div>
    </div>
  );
}
