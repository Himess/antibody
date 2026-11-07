/**
 * Public configuration, from the environment at build time (NEXT_PUBLIC_* are inlined into the client).
 * Nothing here is a secret: the API, the RPCs and the explorers are public, and the practice vend key is
 * public by design (docs/practice.md). Origins are configurable so a custom domain attaches with no code
 * change.
 */

const list = (v: string | undefined, fallback: string[]): string[] => {
  const items = (v ?? "")
    .split(",")
    .map((s) => s.trim().replace(/\/+$/, ""))
    .filter(Boolean);
  return items.length ? items : fallback;
};

export const config = {
  /** The Antibody API (antibody-services api), over HTTPS. */
  apiBase: (process.env.NEXT_PUBLIC_API_BASE ?? "http://127.0.0.1:8787").replace(/\/+$/, ""),
  /** The site's own origin (OG images, canonical URLs). */
  siteUrl: (
    process.env.NEXT_PUBLIC_SITE_URL ??
    (process.env.VERCEL_PROJECT_PRODUCTION_URL ? `https://${process.env.VERCEL_PROJECT_PRODUCTION_URL}` : "http://localhost:3000")
  ).replace(/\/+$/, ""),
  network: process.env.NEXT_PUBLIC_NEAR_NETWORK ?? "testnet",
  /** NEAR RPCs the browser uses for explicit checks and the try, tried in order with backoff. */
  nearRpcs: list(process.env.NEXT_PUBLIC_NEAR_RPCS, [
    "https://rpc.testnet.fastnear.com",
    "https://test.rpc.fastnear.com",
    "https://rpc.testnet.near.org",
  ]),
  /** Archival RPCs: blobs and base witnesses uploaded with FastFS live in old transactions. */
  nearArchivalRpcs: list(process.env.NEXT_PUBLIC_NEAR_ARCHIVAL_RPCS, [
    "https://archival-rpc.testnet.fastnear.com",
    "https://archival-rpc.testnet.near.org",
  ]),
  evmRpcs: {
    11155111: list(process.env.NEXT_PUBLIC_SEPOLIA_RPCS, ["https://ethereum-sepolia-rpc.publicnode.com"]),
    84532: list(process.env.NEXT_PUBLIC_BASE_SEPOLIA_RPCS, ["https://base-sepolia-rpc.publicnode.com"]),
  } as Record<number, string[]>,
  repo: "https://github.com/Himess/antibody",
  /** The commit the deployed site was built from (Vercel sets it), for "view source" links. */
  commit: process.env.NEXT_PUBLIC_GIT_SHA ?? process.env.VERCEL_GIT_COMMIT_SHA ?? "main",
} as const;

export const explorers = {
  nearTx: (hash: string) =>
    config.network === "mainnet"
      ? `https://nearblocks.io/txns/${hash}`
      : `https://testnet.nearblocks.io/txns/${hash}`,
  nearAccount: (account: string) =>
    config.network === "mainnet"
      ? `https://nearblocks.io/address/${account}`
      : `https://testnet.nearblocks.io/address/${account}`,
  evmTx: (chainId: number, hash: string) => {
    const base: Record<number, string> = {
      11155111: "https://sepolia.etherscan.io",
      84532: "https://sepolia.basescan.org",
      1: "https://etherscan.io",
      8453: "https://basescan.org",
    };
    return base[chainId] ? `${base[chainId]}/tx/${hash}` : undefined;
  },
  evmBlock: (chainId: number, block: number | string) => {
    const base: Record<number, string> = {
      11155111: "https://sepolia.etherscan.io",
      84532: "https://sepolia.basescan.org",
    };
    const n = typeof block === "string" && block.startsWith("0x") ? parseInt(block, 16) : Number(block);
    return base[chainId] && Number.isFinite(n) ? `${base[chainId]}/block/${n}` : undefined;
  },
  evmAddress: (chainId: number, address: string) => {
    const base: Record<number, string> = {
      11155111: "https://sepolia.etherscan.io",
      84532: "https://sepolia.basescan.org",
    };
    return base[chainId] ? `${base[chainId]}/address/${address}` : undefined;
  },
};

export const chainName = (id: number | string): string =>
  ({ 11155111: "Sepolia", 84532: "Base Sepolia", 1: "Ethereum", 8453: "Base" })[Number(id)] ??
  `Chain ${id}`;

/** A link to a file in the repository at the deployed commit. */
export const sourceUrl = (path: string, line?: number) =>
  `${config.repo}/blob/${config.commit}/${path.replace(/^\/+/, "")}${line ? `#L${line}` : ""}`;
