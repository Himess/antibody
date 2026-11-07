"use client";

import { sha256 } from "@noble/hashes/sha2.js";
import { useState } from "react";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { Pill } from "@/components/ui/Pill";
import { Sealed } from "@/components/ui/Verdict";
import { fromHex, toHex } from "@/lib/practice/bytes";
import { TUTORIAL_CALL } from "@/lib/practice/envelope";
import { bundleOf, type TrySnapshot } from "@/lib/practice/flow";
import { openWithKey, practiceContentKey } from "@/lib/practice/seal";
import styles from "./try.module.css";

interface Opened {
  programKey: string;
  hunter: string;
  envelopeBytes: number;
  envelopeSha256: string;
  /** The opened envelope is the one this tab built (null: built before a reload, not kept). */
  sameAsBuilt: boolean | null;
  callInside: boolean;
  vaultInside: boolean | null;
  reportBytes: number;
}

/**
 * The sealed proof: visible, and until disclosure readable only by the TEE (and, once paid, the project),
 * except that a practice try is sealed under a public content key (blob.rs `practice_content_key`), so
 * anyone, T2's prover included, can open it.
 * "Open it" does that here, to show it is the public tutorial proof.
 */
export function SealedProof({ s }: { s: TrySnapshot }) {
  const [opened, setOpened] = useState<Opened | null>(null);
  const [error, setError] = useState<string | null>(null);
  const sealed = s.sealed;
  const programKey = s.programKey ?? "";
  if (!sealed) {
    return (
      <div className={`card ${styles.panel}`}>
        <h3>The sealed proof</h3>
        <p className="small muted">It appears here once your browser has built and sealed it, before anything goes on chain.</p>
      </div>
    );
  }
  const open = () => {
    try {
      const p = openWithKey(fromHex(sealed.blobHex), practiceContentKey(programKey, s.account), programKey);
      const env = toHex(p.envelope);
      setOpened({
        programKey: p.programKey,
        hunter: p.hunter,
        envelopeBytes: p.envelope.length,
        envelopeSha256: toHex(sha256(p.envelope)),
        sameAsBuilt: sealed.envelopeHex ? env === sealed.envelopeHex : null,
        callInside: env.includes(TUTORIAL_CALL.slice(2)),
        vaultInside: s.vaultAddress ? env.includes(s.vaultAddress.slice(2).toLowerCase()) : null,
        reportBytes: p.report.length,
      });
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  const formula = `sha256("antibody/practice/public-content-key/v1|${programKey}|${s.account}")`;
  return (
    <div className={`card ${styles.panel}`}>
      <div className={styles.panelHead}>
        <h3>The sealed proof</h3>
        <Pill tone="neutral" icon="lock" size="sm">
          ABSEAL1 · {sealed.bytes} bytes
        </Pill>
      </div>
      <Sealed seed={sealed.sha256} label="Sealed to the program's key" />
      <dl className={`kv ${styles.kvTight}`}>
        <dt>sha256</dt>
        <dd>
          <Hash value={sealed.sha256} head={16} label="sealed blob sha256" />
        </dd>
        <dt>commitment</dt>
        <dd>
          <Hash value={sealed.commitment} head={16} label="commitment" />
        </dd>
        {sealed.saltHex ? (
          <>
            <dt>salt</dt>
            <dd>
              <Hash value={sealed.saltHex} head={16} label="sender salt" />
            </dd>
          </>
        ) : null}
      </dl>
      <p className={`small muted ${styles.wrapAny}`}>
        The commitment binds the program key, the round, your account and this blob&apos;s sha256. The blob&apos;s content key is wrapped to the program&apos;s sealing
        key, which only the TEE holds. On a real program that content key is random: until disclosure, only the TEE opens the proof, and the project once it has paid
        for it (through its release wrap). A practice try uses a public content key instead, <code>{formula}</code>, so anyone can open it, and T2 can prove it with
        nothing handed over.
      </p>
      <div className="row" style={{ ["--gap" as string]: "8px" }}>
        <button type="button" className="btn btn-sm" onClick={open} aria-expanded={opened !== null} aria-controls="opened-proof">
          <Icon name="key" size={13} /> Open it with the public content key
        </button>
      </div>
      <div id="opened-proof" aria-live="polite">
        {error ? (
          <p className="small" role="alert">
            Could not open it: {error}
          </p>
        ) : null}
        {opened ? (
          <div className={styles.opened}>
            <p className="small">
              <strong>Opened in this tab.</strong> It is the public tutorial proof: one call, <code>sweep(0x…dEaD)</code>, to your vault.
            </p>
            <dl className={`kv ${styles.kvTight}`}>
              <dt>program key</dt>
              <dd className="mono">{opened.programKey}</dd>
              <dt>hunter</dt>
              <dd className="mono">{opened.hunter}</dd>
              <dt>envelope</dt>
              <dd>
                {opened.envelopeBytes} bytes, sha256 <Hash value={opened.envelopeSha256} head={12} label="envelope sha256" />
                {opened.sameAsBuilt === true ? <span className="muted"> · byte for byte the one built here</span> : null}
              </dd>
              <dt>the call</dt>
              <dd>
                {opened.callInside ? "sweep(0x…dEaD) is in the envelope" : "the tutorial call is not in it"}
                {opened.vaultInside === true ? `, to ${s.vaultAddress}` : ""}
              </dd>
              <dt>report</dt>
              <dd>{opened.reportBytes === 0 ? "none (the verifier never reads reports)" : `${opened.reportBytes} bytes`}</dd>
            </dl>
            {s.vaultAddress ? (
              <pre className={styles.code} tabIndex={0} aria-label="The tutorial bundle">
                {JSON.stringify(bundleOf(s.vaultAddress), null, 2)}
              </pre>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  );
}
