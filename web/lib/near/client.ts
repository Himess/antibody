import { config } from "@/lib/config";
import { NearRpc } from "./rpc";

let rpc: NearRpc | null = null;
let archival: NearRpc | null = null;

/** The browser's NEAR RPC client (public endpoints, in order, with backoff). */
export function nearRpc(): NearRpc {
  rpc ??= new NearRpc(config.nearRpcs, { timeoutMs: 20_000, attempts: 6 });
  return rpc;
}

/** Archival RPCs: FastFS uploads (bases, blobs) live in old transactions. */
export function nearArchival(): NearRpc {
  archival ??= new NearRpc(config.nearArchivalRpcs, { timeoutMs: 30_000, attempts: 4 });
  return archival;
}

/** An EVM JSON-RPC read against the chain's public endpoints, in order. */
export async function evmRead(chainId: number, method: string, params: unknown[]): Promise<unknown> {
  const urls = config.evmRpcs[chainId] ?? [];
  let last: unknown = new Error(`no RPC for chain ${chainId}`);
  for (const url of urls) {
    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        const res = await fetch(url, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
          signal: AbortSignal.timeout(15_000),
        });
        const body = (await res.json()) as { result?: unknown; error?: { message?: string } };
        if (body.error) throw new Error(body.error.message ?? "RPC error");
        return body.result;
      } catch (e) {
        last = e;
        await new Promise((r) => setTimeout(r, 300 * 2 ** attempt));
      }
    }
  }
  throw last instanceof Error ? last : new Error(String(last));
}
