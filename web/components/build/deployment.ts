import { api } from "@/lib/api";
import { config } from "@/lib/config";

/**
 * The deployment's contract ids, read from the API's /health. When the API cannot answer, the locked
 * testnet deployment's ids (README: Deployment) stand in, so a check still names the right contract.
 */
export async function deployment() {
  const h = await api.health();
  const c = h.ok ? h.data?.contracts : undefined;
  return {
    antibody: c?.antibody || "bounty.antibody.testnet",
    guard: c?.verdict_guard || "guard.antibody.testnet",
    vending: c?.practice?.vending || "practice.antibody.testnet",
  };
}

/** The first public RPC the site uses for an EVM chain, for the copyable curl beside a check. */
export const evmRpc = (chainId: number) =>
  config.evmRpcs[chainId]?.[0] ??
  (chainId === 84532 ? "https://base-sepolia-rpc.publicnode.com" : "https://ethereum-sepolia-rpc.publicnode.com");

/** The same JSON-RPC call as a curl command anyone can paste. */
export const curlRpc = (chainId: number, method: string, params: unknown[]) =>
  `curl -s ${evmRpc(chainId)} -H 'content-type: application/json' -d '${JSON.stringify({ jsonrpc: "2.0", id: 1, method, params })}'`;
