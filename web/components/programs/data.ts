import { cache } from "react";
import { api } from "@/lib/api";

/**
 * The API reads behind the programs pages (server components only), memoized per request with React's
 * `cache`: `generateMetadata`, the page and the sections that stream in after it share one read of each
 * endpoint. `get` passes an abort signal, which opts out of fetch deduplication, hence this; the reads
 * themselves sit in the Data Cache with short revalidation (see `get` in lib/api).
 */
export const getPrograms = cache(() => api.programs());
export const getProgram = cache((id: number) => api.program(id));
export const getReflex = cache((id: number) => api.reflex(id));
export const getHealth = cache(() => api.health());
export const getT2 = cache(() => api.t2());
export const getJudge = cache(() => api.judge());

/** The contracts the checks call, from /health, with the locked deployment's names as the fallback. */
export async function contractsOf() {
  const h = await getHealth();
  const c = h.ok ? h.data.contracts : undefined;
  return {
    antibody: c?.antibody ?? "bounty.antibody.testnet",
    lightClient: c?.light_client ?? "client-eth2.sepolia.testnet",
    verdictGuard: c?.verdict_guard ?? "guard.antibody.testnet",
    practiceProgram: c?.practice?.program ?? null,
  };
}
