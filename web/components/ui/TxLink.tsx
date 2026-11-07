import { chainName, explorers } from "@/lib/config";
import { Hash } from "./Hash";

/** A transaction with its explorer link: NEAR testnet, or an EVM chain by id. */
export function TxLink({ hash, chainId, href, head = 8 }: { hash: string | null | undefined; chainId?: number; href?: string | null; head?: number }) {
  if (!hash) return <span className="muted">—</span>;
  const url = href ?? (chainId ? explorers.evmTx(chainId, hash) : explorers.nearTx(hash));
  return <Hash value={hash} href={url} head={head} label={chainId ? `${chainName(chainId)} transaction` : "NEAR transaction"} />;
}

export function AccountLink({ account }: { account: string | null | undefined }) {
  if (!account) return <span className="muted">—</span>;
  return <Hash value={account} href={explorers.nearAccount(account)} head={40} copy label="account" />;
}
