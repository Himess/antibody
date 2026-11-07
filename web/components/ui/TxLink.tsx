import { chainName, explorers } from "@/lib/config";
import { Hash } from "./Hash";

/** A transaction with its explorer link: NEAR testnet, or an EVM chain by id. */
export function TxLink({ hash, chainId, href, head = 8 }: { hash: string | null | undefined; chainId?: number; href?: string | null; head?: number }) {
  if (!hash) return <span className="muted">—</span>;
  const url = href ?? (chainId ? explorers.evmTx(chainId, hash) : explorers.nearTx(hash));
  return <Hash value={hash} href={url} head={head} label={chainId ? `${chainName(chainId)} transaction` : "NEAR transaction"} />;
}

/**
 * An account: it opens its read-only view on this site; its explorer page is the icon beside it (new tab).
 * `site={false}` where the page already is that view, or for a contract, whose view on this site has nothing
 * to show: the name then links to the explorer only.
 */
export function AccountLink({ account, site = true }: { account: string | null | undefined; site?: boolean }) {
  if (!account) return <span className="muted">—</span>;
  return (
    <Hash
      value={account}
      to={site ? `/account?view=${encodeURIComponent(account)}` : undefined}
      href={explorers.nearAccount(account)}
      head={40}
      copy
      label="account"
    />
  );
}
