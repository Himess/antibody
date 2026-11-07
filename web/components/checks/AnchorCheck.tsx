"use client";

import { useState } from "react";
import { base64 } from "@scure/base";
import { Command } from "@/components/ui/Command";
import { Icon } from "@/components/ui/Icon";
import { Pill } from "@/components/ui/Pill";
import { chainName } from "@/lib/config";
import { evmRead, nearRpc } from "@/lib/near/client";
import styles from "./checks.module.css";

type Read<T> = { ok: true; value: T } | { ok: false; error: string };

/** The light client's argument: the block number as 8 little-endian bytes (Borsh u64). */
function leU64(n: number): Uint8Array {
  const out = new Uint8Array(8);
  let v = BigInt(n);
  for (let i = 0; i < 8; i++) {
    out[i] = Number(v & BigInt(0xff));
    v >>= BigInt(8);
  }
  return out;
}

/**
 * The RPCs the printed commands name: public, keyless endpoints (a configured endpoint may carry a key,
 * and a command on the page must never show one).
 */
const SHOWN_NEAR_RPC = "https://rpc.testnet.fastnear.com";
const SHOWN_EVM_RPC: Record<number, string> = {
  11155111: "https://ethereum-sepolia-rpc.publicnode.com",
  84532: "https://base-sepolia-rpc.publicnode.com",
};

const hex = (b: ArrayLike<number>) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
const norm = (h: string | null | undefined) => (h ?? "").toLowerCase().replace(/^0x/, "");
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

interface LcAnswer {
  bytes: number[];
  /** Borsh Option<[u8; 32]>: 0x01 then the hash, or 0x00 for none. */
  hash: string | null;
  height: number;
}

interface EthBlock {
  number: string | null;
  hash: string | null;
  parentHash: string | null;
  stateRoot: string | null;
  timestamp: string | null;
}

/** Both reads at once (each may fail alone), and how long they took. */
async function readAnchor(lightClient: string, block: number, chainId: number): Promise<{ lc: Read<LcAnswer>; eth: Read<EthBlock>; ms: number }> {
  const t0 = performance.now();
  const [lc, eth] = await Promise.all([
    nearRpc()
      .viewFunction(lightClient, "block_hash_safe", leU64(block), "final")
      .then(
        (r): Read<LcAnswer> => ({
          ok: true,
          value: { bytes: Array.from(r.bytes), hash: r.bytes[0] === 1 && r.bytes.length === 33 ? `0x${hex(r.bytes.slice(1))}` : null, height: r.blockHeight },
        }),
        (e): Read<LcAnswer> => ({ ok: false, error: message(e) }),
      ),
    evmRead(chainId, "eth_getBlockByNumber", [`0x${block.toString(16)}`, false]).then(
      (b): Read<EthBlock> => {
        const o = (b && typeof b === "object" ? b : {}) as Record<string, unknown>;
        const s = (k: string) => (typeof o[k] === "string" ? (o[k] as string) : null);
        if (!s("hash")) return { ok: false, error: `${chainName(chainId)} returned no block ${block}` };
        return { ok: true, value: { number: s("number"), hash: s("hash"), parentHash: s("parentHash"), stateRoot: s("stateRoot"), timestamp: s("timestamp") } };
      },
      (e): Read<EthBlock> => ({ ok: false, error: message(e) }),
    ),
  ]);
  return { lc, eth, ms: performance.now() - t0 };
}

/**
 * The anchor against the light client, run by the browser: NEAR's Ethereum light client's hash for the
 * anchor block (a view call with Borsh arguments), Sepolia's own hash for it (a public RPC), and the hash
 * the program committed. near-antibody made the same comparison when the program was created.
 */
export function AnchorCheck({ block, expected, lightClient, chainId, program }: { block: number; expected: string; lightClient: string; chainId: number; program?: number }) {
  const [state, setState] = useState<{ status: "idle" | "running" | "done"; lc?: Read<LcAnswer>; eth?: Read<EthBlock>; ms?: number }>({ status: "idle" });
  const args = leU64(block);
  const tag = `0x${block.toString(16)}`;
  const chain = chainName(chainId);
  const evmRpc = SHOWN_EVM_RPC[chainId] ?? "<an RPC for this chain>";
  const nearRpcUrl = SHOWN_NEAR_RPC;

  const run = async () => {
    setState({ status: "running" });
    const r = await readAnchor(lightClient, block, chainId);
    setState({ status: "done", ...r });
  };

  const lc = state.lc;
  const eth = state.eth;
  const lcHash = lc?.ok ? lc.value.hash : null;
  const lcEqual = lc?.ok && lcHash !== null ? norm(lcHash) === norm(expected) : null;
  const ethEqual = eth?.ok ? norm(eth.value.hash) === norm(expected) : null;

  let verdict: { tone: "ok" | "bad" | "warn"; text: string; detail: string } | null = null;
  if (state.status === "done") {
    if (lcEqual === false || ethEqual === false) {
      verdict = { tone: "bad", text: "differs", detail: "A hash differs from the one the program committed." };
    } else if (lc?.ok && lcHash === null) {
      verdict = {
        tone: "warn",
        text: "outside the window",
        detail: `The light client no longer holds block ${block}: it keeps a window of recent blocks. near-antibody compared the hashes when the program was created; ${chain}'s own hash still checks the anchor.`,
      };
    } else if (lcEqual && ethEqual) {
      verdict = { tone: "ok", text: "holds", detail: `All three hashes are equal: the program's anchor, the light client's and ${chain}'s.` };
    } else if (lcEqual) {
      verdict = { tone: "ok", text: "holds on NEAR", detail: `The light client holds the committed hash; ${chain}'s RPC did not answer this time.` };
    } else if (ethEqual) {
      verdict = { tone: "warn", text: "half checked", detail: `${chain} agrees with the committed hash; the NEAR RPC did not answer this time.` };
    } else {
      verdict = { tone: "warn", text: "could not check", detail: "Public RPCs rate-limit; try again in a few seconds, or run a command below." };
    }
  }

  const raw =
    state.status === "done"
      ? [
          `view ${lightClient}.block_hash_safe(${block} as 8 LE bytes: ${hex(args)})`,
          lc?.ok
            ? `  → ${lc.value.bytes.length} bytes at NEAR block ${lc.value.height}: ${hex(lc.value.bytes)} (${lc.value.hash ? "Some(hash)" : "None"})`
            : `  → error: ${lc?.error}`,
          "",
          `eth_getBlockByNumber("${tag}", false) on ${chain}`,
          eth?.ok ? `  → ${JSON.stringify(eth.value, null, 2).replace(/\n/g, "\n    ")}` : `  → error: ${eth?.error}`,
        ].join("\n")
      : "";

  return (
    <div className="stack" style={{ ["--gap" as string]: "12px" }}>
      <div className="row" style={{ ["--gap" as string]: "10px" }}>
        <button type="button" className="btn btn-sm" onClick={run} disabled={state.status === "running"} aria-busy={state.status === "running"}>
          <Icon name={state.status === "running" ? "refresh" : "play"} size={13} />
          {state.status === "running" ? "Checking…" : state.status === "idle" ? "Run the anchor check" : "Run again"}
        </button>
        <span className="xsmall muted">Two reads by your browser: a NEAR view call and a {chain} RPC call.</span>
      </div>
      <dl className={styles.anchorRows}>
        <div className={styles.anchorRow}>
          <dt>
            <span>The anchor {typeof program === "number" ? `program ${program} committed` : "the program committed"}</span>
            <span className="mono">
              {chain} block {block.toLocaleString("en-US")}
            </span>
          </dt>
          <dd>{expected}</dd>
        </div>
        <div className={`${styles.anchorRow} ${lc ? "" : styles.anchorRowIdle}`}>
          <dt>
            <span>
              NEAR&apos;s light client, <span className="mono">{lightClient}</span>
            </span>
            {lcEqual === true ? (
              <Pill tone="ok" size="sm">
                equal
              </Pill>
            ) : lcEqual === false ? (
              <Pill tone="bad" size="sm">
                differs
              </Pill>
            ) : lc?.ok ? (
              <Pill tone="warn" size="sm">
                none
              </Pill>
            ) : lc ? (
              <Pill tone="warn" size="sm">
                no answer
              </Pill>
            ) : null}
          </dt>
          <dd>{lc ? (lc.ok ? (lcHash ?? "None: outside its window") : lc.error) : <span className="muted">read when you run the check</span>}</dd>
        </div>
        <div className={`${styles.anchorRow} ${eth ? "" : styles.anchorRowIdle}`}>
          <dt>
            <span>{chain}&apos;s own block, from a public RPC</span>
            {ethEqual === true ? (
              <Pill tone="ok" size="sm">
                equal
              </Pill>
            ) : ethEqual === false ? (
              <Pill tone="bad" size="sm">
                differs
              </Pill>
            ) : eth ? (
              <Pill tone="warn" size="sm">
                no answer
              </Pill>
            ) : null}
          </dt>
          <dd>{eth ? (eth.ok ? eth.value.hash : eth.error) : <span className="muted">read when you run the check</span>}</dd>
        </div>
      </dl>
      <div aria-live="polite" className="stack" style={{ ["--gap" as string]: "8px" }}>
        {verdict ? (
          <>
            <div className="row" style={{ ["--gap" as string]: "8px" }}>
              <Pill tone={verdict.tone} size="sm">
                {verdict.text}
              </Pill>
              <span className="xsmall muted num">{Math.round(state.ms ?? 0)} ms · read by your browser</span>
            </div>
            <p className="small">{verdict.detail}</p>
            <pre className={styles.raw} tabIndex={0} aria-label="Raw answers">
              {raw}
            </pre>
          </>
        ) : null}
      </div>
      <Command label="CLI command">{`antibody anchor-check --block ${block}`}</Command>
      <details className={styles.more}>
        <summary>The same two reads with curl, no Antibody code</summary>
        <div className={styles.moreBody}>
          <Command label="NEAR view call">
            {`curl -s ${nearRpcUrl} -H 'content-type: application/json' -d '${JSON.stringify({
              jsonrpc: "2.0",
              id: 1,
              method: "query",
              params: { request_type: "call_function", finality: "final", account_id: lightClient, method_name: "block_hash_safe", args_base64: base64.encode(args) },
            })}'`}
          </Command>
          <p className="xsmall muted">
            <code>result.result</code> is the Borsh answer as bytes: 1, then the 32-byte hash (0 alone means the block is outside the window).
          </p>
          <Command label={`${chain} RPC call`}>
            {`curl -s ${evmRpc} -H 'content-type: application/json' -d '${JSON.stringify({ jsonrpc: "2.0", id: 1, method: "eth_getBlockByNumber", params: [tag, false] })}'`}
          </Command>
        </div>
      </details>
    </div>
  );
}
