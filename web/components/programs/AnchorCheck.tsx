"use client";

import { useState } from "react";
import { Icon } from "@/components/ui/Icon";
import { Pill } from "@/components/ui/Pill";
import { nearRpc } from "@/lib/near/client";
import styles from "./programs.module.css";

const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");

/** The view's argument: the block number as 8 little-endian bytes (Borsh u64), as crates/cli's anchor_check sends it. */
function blockArg(block: number): Uint8Array {
  const out = new Uint8Array(8);
  new DataView(out.buffer).setBigUint64(0, BigInt(block), true);
  return out;
}

type State =
  | { status: "idle" }
  | { status: "running" }
  | { status: "done"; bytes: Uint8Array; height: number; ms: number }
  | { status: "error"; error: string; ms: number };

/**
 * Asks NEAR's Ethereum light client, from the browser, for the hash it holds for the anchor block
 * (`block_hash_safe`: Borsh `Option<[u8; 32]>`, byte 0 = 1 then the 32-byte hash) and compares it with the
 * program's anchor hash. The raw bytes are shown as the RPC returned them.
 */
export function AnchorCheck({ lightClient, block, hash }: { lightClient: string; block: number; hash: string }) {
  const [s, setS] = useState<State>({ status: "idle" });
  const arg = blockArg(block);
  const want = hash.toLowerCase().replace(/^0x/, "");

  const run = async () => {
    setS({ status: "running" });
    const t0 = performance.now();
    try {
      const r = await nearRpc().viewFunction(lightClient, "block_hash_safe", arg, "final");
      setS({ status: "done", bytes: r.bytes, height: r.blockHeight, ms: performance.now() - t0 });
    } catch (e) {
      setS({ status: "error", error: e instanceof Error ? e.message : String(e), ms: performance.now() - t0 });
    }
  };

  let outcome: { held: string | null; verdict: "match" | "differs" | "absent" | "malformed" } | null = null;
  if (s.status === "done") {
    const b = s.bytes;
    if (b.length === 33 && b[0] === 1) {
      const held = hex(b.slice(1));
      outcome = { held, verdict: held === want ? "match" : "differs" };
    } else if (b.length === 1 && b[0] === 0) {
      outcome = { held: null, verdict: "absent" };
    } else {
      outcome = { held: null, verdict: "malformed" };
    }
  }

  return (
    <div className={styles.check}>
      <div className={styles.checkHead}>
        {/* aria-disabled, not disabled, while it asks: a disabled button would drop keyboard focus to the page. */}
        <button
          type="button"
          className="btn btn-sm"
          onClick={() => {
            if (s.status !== "running") void run();
          }}
          aria-disabled={s.status === "running" || undefined}
          aria-busy={s.status === "running"}
        >
          <Icon name={s.status === "running" ? "refresh" : "play"} size={13} />
          {s.status === "running" ? "Asking the light client…" : s.status === "idle" ? "Ask the light client" : "Ask again"}
        </button>
        <span className={styles.what}>
          view {lightClient}.block_hash_safe({block} as 8 little-endian bytes: 0x{hex(arg)})
        </span>
      </div>
      <p className="xsmall muted">Expected: byte 0 is 1 (the client holds the block), then 32 bytes equal to the anchor hash.</p>
      {/* The live region says the result; the raw bytes follow it, so a screen reader does not read 33 numbers out. */}
      <div aria-live="polite">
        {s.status === "done" && outcome ? (
          <div className="stack" style={{ ["--gap" as string]: "8px" }}>
            <div className="row" style={{ ["--gap" as string]: "8px" }}>
              {outcome.verdict === "match" ? (
                <Pill tone="ok" size="sm">
                  matches the anchor hash
                </Pill>
              ) : outcome.verdict === "differs" ? (
                <Pill tone="bad" size="sm">
                  differs from the anchor hash
                </Pill>
              ) : outcome.verdict === "absent" ? (
                <Pill tone="warn" size="sm">
                  not held: outside the light client&apos;s window
                </Pill>
              ) : (
                <Pill tone="warn" size="sm">
                  unexpected answer
                </Pill>
              )}
              <span className="xsmall muted num">
                {Math.round(s.ms)} ms · read by your browser{s.height ? ` at NEAR block ${s.height}` : ""}
              </span>
            </div>
            <dl className={styles.compare}>
              <div>
                <dt>The light client holds</dt>
                <dd>{outcome.held ? `0x${outcome.held}` : "nothing (None)"}</dd>
              </div>
              <div>
                <dt>The program&apos;s anchor hash</dt>
                <dd>0x{want}</dd>
              </div>
            </dl>
            {outcome.verdict === "absent" ? (
              <p className="xsmall muted">
                The client keeps a window of recent blocks. The check that counts ran when the program was created; its
                anchor_checked event is linked above.
              </p>
            ) : null}
          </div>
        ) : s.status === "error" ? (
          <div className="stack" style={{ ["--gap" as string]: "6px" }}>
            <Pill tone="warn" size="sm">
              could not check
            </Pill>
            <p className="xsmall mono">{s.error}</p>
            <p className="xsmall muted">Public RPCs rate-limit; try again in a few seconds, or run the command below.</p>
          </div>
        ) : null}
      </div>
      {s.status === "done" ? (
        <pre className={styles.raw} tabIndex={0} aria-label="Raw bytes returned by the view">
          {JSON.stringify(Array.from(s.bytes))}
        </pre>
      ) : null}
    </div>
  );
}
