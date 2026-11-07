/**
 * The wallet connection: NEAR Connect (npm @hot-labs/near-connect, loaded on demand, only on /account) with
 * the pinned wallet list of ./manifest.ts. The page holds no key: it learns an account id at sign-in and
 * asks the wallet to sign the transactions a person reviewed. It never asks for a message signature (a
 * plugin refuses any), never adds an access key, and checks every outcome itself on NEAR RPC.
 */
import type { DataStorage, NearConnector, NearWalletBase, WalletPlugin } from "@hot-labs/near-connect";
import { config } from "@/lib/config";
import { nearRpc } from "@/lib/near/client";
import { events, outcomeFailures, returnValue } from "@/lib/near/rpc";
import { isTestnetAccount } from "./account";
import { WALLET_NETWORK } from "./contracts";
import { PINNED_IDS, pinnedManifest } from "./manifest";
import { toConnectorActions, type Plan, type PlannedTx } from "./tx";

/** This site's keys in local storage (the connector's choice of wallet; nothing secret). */
const PREFIX = "antibody.wallet.";

export interface WalletChoice {
  id: string;
  name: string;
  /** Announced by a browser extension rather than listed in the pinned manifest. */
  injected: boolean;
}

export interface Session {
  accountId: string;
  walletId: string;
  walletName: string;
}

export type FailureKind = "rejected" | "not-installed" | "popup-blocked" | "load" | "network" | "no-account" | "wallet";

/** A wallet step that did not complete, said plainly, with the wallet's own words in `detail`. */
export class WalletFailure extends Error {
  readonly kind: FailureKind;
  readonly detail?: string;

  constructor(kind: FailureKind, message: string, detail?: string) {
    super(message);
    this.name = "WalletFailure";
    this.kind = kind;
    this.detail = detail && detail !== message ? detail : undefined;
  }
}

export interface TxResult {
  index: number;
  receiver: string;
  says: string;
  hash: string | null;
  status: "ok" | "failed" | "refunded" | "unknown" | "not-sent";
  detail?: string;
}

const obj = (v: unknown): Record<string, unknown> | null => (typeof v === "object" && v !== null && !Array.isArray(v) ? (v as Record<string, unknown>) : null);

/** A thrown value's words: wallets throw strings, plain objects (through postMessage) or errors. */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  const o = obj(e);
  if (o && typeof o.message === "string") return o.message;
  if (o && typeof o.error === "string") return o.error;
  try {
    return JSON.stringify(e) ?? String(e);
  } catch {
    return String(e);
  }
}

/** Wallet errors, by what a person can do about them. */
export function classify(e: unknown): WalletFailure {
  if (e instanceof WalletFailure) return e;
  const t = errorText(e);
  if (/user (rejected|denied|cancel|closed)|rejected|cancell?ed|denied|declined|wallet closed|closed by (the )?user|abort/i.test(t))
    return new WalletFailure("rejected", "You closed or rejected the request in your wallet. Nothing was signed.", t);
  if (/not installed|install the|no extension|extension (is )?not/i.test(t))
    return new WalletFailure("not-installed", "That wallet is not installed in this browser. Install it, or pick another wallet.", t);
  if (/pop-?up|window (was )?blocked/i.test(t))
    return new WalletFailure("popup-blocked", "The browser blocked the wallet's window. Allow pop-ups for this site, then try again.", t);
  if (/failed to load executor|failed to fetch|networkerror|load failed|content security policy|refused to connect/i.test(t))
    return new WalletFailure("load", "The wallet's connector script could not be loaded. Check your connection, then try again.", t);
  if (/mainnet|wrong network|network mismatch|unsupported network/i.test(t))
    return new WalletFailure("network", "The wallet is set to another network. Switch it to NEAR testnet, then connect again.", t);
  if (/no accounts? found|no wallet selected|not signed in/i.test(t))
    return new WalletFailure("no-account", "The wallet returned no account. Unlock it, or add a testnet account to it, then connect again.", t);
  return new WalletFailure("wallet", "The wallet reported an error.", t);
}

// ------------------------------------------------------------------ the connector

const storage: DataStorage = {
  async get(key) {
    try {
      return window.localStorage.getItem(PREFIX + key);
    } catch {
      return null;
    }
  },
  async set(key, value) {
    try {
      window.localStorage.setItem(PREFIX + key, value);
    } catch {
      /* storage unavailable (a private window): the session lasts until the page closes */
    }
  },
  async remove(key) {
    try {
      window.localStorage.removeItem(PREFIX + key);
    } catch {
      /* nothing to remove */
    }
  },
};

/** Whether a wallet was connected here before (read without loading the connector). */
export function hasSavedSession(): boolean {
  try {
    return Boolean(window.localStorage.getItem(PREFIX + "selected-wallet"));
  } catch {
    return false;
  }
}

/**
 * This request's CSP nonce. A wallet's iframe is a srcdoc document and inherits the page's policy, so its
 * two inline scripts (the connector's bridge and the wallet's script) need it. Browsers hide the attribute;
 * the IDL property keeps the value.
 */
function cspNonce(): string | undefined {
  for (const s of Array.from(document.querySelectorAll("script"))) if (s.nonce) return s.nonce;
  return undefined;
}

/** Message signatures are refused before they reach any wallet (threat model P13). */
const refuse = (): never => {
  throw new WalletFailure("wallet", "This site never asks a wallet to sign a message.");
};
const noMessageSigning: WalletPlugin = { signMessage: refuse, signInAndSignMessage: refuse, signDelegateActions: refuse };

let connector: Promise<NearConnector> | null = null;
let isInjected: (w: NearWalletBase) => boolean = () => false;
/** A pinned wallet in its sandbox, or an extension's own wallet object: nothing announced by another window. */
let isOffered: (w: NearWalletBase) => boolean = () => false;
let current: { wallet: NearWalletBase; session: Session } | null = null;

/** The connector, created once, on first use (its code is a separate chunk of this page). */
export function getConnector(): Promise<NearConnector> {
  connector ??= (async () => {
    const mod = await import("@hot-labs/near-connect");
    isInjected = (w) => w instanceof mod.InjectedWallet;
    isOffered = (w) => (w instanceof mod.SandboxWallet && PINNED_IDS.has(w.manifest.id)) || w instanceof mod.InjectedWallet;
    const c = new mod.NearConnector({
      network: WALLET_NETWORK,
      manifest: pinnedManifest(),
      providers: { testnet: [...config.nearRpcs], mainnet: [] },
      features: { testnet: true },
      autoConnect: false,
      footerBranding: null,
      cspNonce: cspNonce(),
      storage,
    });
    await c.whenManifestLoaded;
    await c.use(noMessageSigning);
    return c;
  })().catch((e: unknown) => {
    connector = null;
    throw new WalletFailure("load", "The wallet connector could not be loaded.", errorText(e));
  });
  return connector;
}

/** The wallets to offer: the pinned ones, and any testnet wallet a browser extension announces. */
export async function listWallets(): Promise<WalletChoice[]> {
  const c = await getConnector();
  return c.availableWallets.filter(isOffered).map((w) => ({ id: w.manifest.id, name: w.manifest.name, injected: isInjected(w) }));
}

/** Accepts the wallet's first account if it is a testnet account that exists. */
async function adopt(c: NearConnector, wallet: NearWalletBase, accounts: { accountId: string }[]): Promise<Session> {
  const id = accounts[0]?.accountId;
  if (!id) {
    await storage.remove("selected-wallet");
    throw new WalletFailure("no-account", "The wallet returned no account. Unlock it, or add a testnet account to it, then connect again.");
  }
  if (!isTestnetAccount(id)) {
    await c.disconnect(wallet).catch(() => undefined);
    throw new WalletFailure("network", `The wallet answered with ${id}, which is not a NEAR testnet account. Switch the wallet to testnet, then connect again.`);
  }
  let exists: boolean | null = null;
  try {
    exists = (await nearRpc().viewAccount(id)) !== null;
  } catch {
    exists = null; // the RPC could not say: the wallet's answer stands
  }
  if (exists === false) {
    await c.disconnect(wallet).catch(() => undefined);
    throw new WalletFailure("no-account", `${id} does not exist on NEAR testnet yet. Fund it with testnet NEAR, then connect again.`);
  }
  const session: Session = { accountId: id, walletId: wallet.manifest.id, walletName: wallet.manifest.name };
  current = { wallet, session };
  return session;
}

/** Signs in with `walletId` (the wallet opens; no key is added). */
export async function connectWallet(walletId: string): Promise<Session> {
  const c = await getConnector();
  // The connector looks wallets up by id, and another window could announce one under a pinned id.
  const target = c.availableWallets.find((w) => w.manifest.id === walletId);
  if (!target || !isOffered(target)) throw new WalletFailure("wallet", "That wallet is not available here. Pick one from the list.");
  const got: { accounts: { accountId: string }[] } = { accounts: [] };
  const onSignIn = (e: { accounts: { accountId: string }[] }) => {
    got.accounts = e.accounts ?? [];
  };
  c.on("wallet:signIn", onSignIn);
  let wallet: NearWalletBase;
  try {
    wallet = await c.connect({ walletId });
    if (!got.accounts.length) got.accounts = await wallet.getAccounts({ network: WALLET_NETWORK });
  } catch (e) {
    // The connector records the choice before the wallet answers: forget it, so no reload restores it.
    await storage.remove("selected-wallet");
    throw classify(e);
  } finally {
    c.off("wallet:signIn", onSignIn);
  }
  return adopt(c, wallet, got.accounts);
}

/** Restores the session of a wallet connected here before, or null. */
export async function restoreSession(): Promise<Session | null> {
  if (!hasSavedSession()) return null;
  const c = await getConnector();
  try {
    const id = await storage.get("selected-wallet");
    const wallet = c.availableWallets.find((w) => w.manifest.id === id);
    if (!wallet || !isOffered(wallet)) throw new WalletFailure("wallet", "The saved wallet is not one this page offers.");
    return await adopt(c, wallet, await wallet.getAccounts({ network: WALLET_NETWORK }));
  } catch (e) {
    await storage.remove("selected-wallet");
    if (e instanceof WalletFailure && e.kind === "network") throw e;
    return null;
  }
}

/** Signs out of the wallet and forgets it here (its own session data under its id is cleared too). */
export async function disconnectWallet(): Promise<void> {
  const wallet = current?.wallet;
  current = null;
  const c = connector ? await connector.catch(() => null) : null;
  if (c && wallet) await c.disconnect(wallet).catch(() => undefined);
  await storage.remove("selected-wallet");
}

/** Calls `cb` when the wallet signs out on its own (from its own window, say). */
export async function onWalletSignOut(cb: () => void): Promise<() => void> {
  const c = await getConnector();
  const h = () => {
    current = null;
    cb();
  };
  c.on("wallet:signOut", h);
  return () => c.off("wallet:signOut", h);
}

// ------------------------------------------------------------------ transactions

/** The transaction hash in whatever a wallet returns for a sent transaction. */
function hashOf(o: unknown): string | null {
  if (typeof o === "string" && /^[1-9A-HJ-NP-Za-km-z]{43,44}$/.test(o)) return o;
  const outcomes = obj(o)?.outcomes;
  if (Array.isArray(outcomes) && outcomes.length) return hashOf(outcomes[0]);
  const h = obj(obj(o)?.transaction)?.hash ?? obj(obj(o)?.transaction_outcome)?.id;
  return typeof h === "string" ? h : null;
}

function asList(r: unknown): unknown[] {
  if (Array.isArray(r)) return r;
  const outcomes = obj(r)?.outcomes;
  return Array.isArray(outcomes) ? outcomes : [r];
}

/** A failure's words: the contract's panic message when there is one. */
export function failureText(f: string): string {
  const m = /Smart contract panicked: ((?:[^"\\]|\\.)+)/.exec(f) ?? /"ExecutionError":"((?:[^"\\]|\\.)+)"/.exec(f);
  if (m) return m[1].replace(/\\"/g, '"');
  const k = /"kind":\{"([A-Za-z]+)"/.exec(f);
  return k ? k[1].replace(/([a-z])([A-Z])/g, "$1 $2") : f.slice(0, 300);
}

/** The transaction's outcome as this site's RPC sees it (a few tries: the wallet may send elsewhere). */
async function outcomeOf(hash: string, signer: string): Promise<unknown> {
  let last: unknown;
  for (let i = 0; i < 6; i++) {
    try {
      return await nearRpc().txStatus(hash, signer, "EXECUTED_OPTIMISTIC");
    } catch (e) {
      last = e;
      await new Promise((r) => setTimeout(r, 1500));
    }
  }
  throw last;
}

/** Reads one sent transaction back from NEAR and says what happened. */
async function verify(index: number, tx: PlannedTx, returned: unknown, signer: string): Promise<TxResult> {
  const hash = hashOf(returned);
  const base: TxResult = { index, receiver: tx.receiver, says: tx.says, hash, status: "unknown" };
  if (!hash) return { ...base, detail: "The wallet did not return a transaction hash: check your wallet's activity, then refresh this page." };
  let o: unknown;
  try {
    o = await outcomeOf(hash, signer);
  } catch (e) {
    return { ...base, detail: `Sent. Your browser could not read its outcome yet (${errorText(e)}); the link shows it.` };
  }
  const fails = outcomeFailures(o);
  if (tx.expect === "deposit") {
    const used = returnValue(o);
    if (used === "0" || (fails.length && used !== tx.amount?.toString())) {
      return { ...base, status: "refunded", detail: `near-antibody refused the tokens and wrap.testnet returned them to your wallet${fails.length ? `: ${failureText(fails[0])}` : "."}` };
    }
  }
  if (tx.expect === "withdraw") {
    const refunded = events(o).some((ev) => obj(ev)?.event === "withdraw_refunded");
    if (refunded) return { ...base, status: "refunded", detail: "wrap.testnet refused the transfer, and near-antibody credited the amount back." };
  }
  if (fails.length) return { ...base, status: "failed", detail: failureText(fails[0]) };
  return { ...base, status: "ok" };
}

/**
 * Asks the wallet to sign and send a reviewed plan: one transaction, or several in one request when the
 * wallet can, else one after another. Every outcome is then read back from NEAR. A wallet error after some
 * transactions landed returns those, with `error`.
 */
export async function sendPlan(plan: Plan, onSent?: () => void): Promise<{ results: TxResult[]; error?: WalletFailure }> {
  if (!current) throw new WalletFailure("no-account", "No wallet is connected.");
  const { wallet, session } = current;
  const signerId = session.accountId;
  const txs = plan.txs.map((t) => ({ receiverId: t.receiver, actions: toConnectorActions(t) }));
  const returned: unknown[] = [];
  let error: WalletFailure | undefined;
  try {
    if (txs.length === 1) {
      returned.push(await wallet.signAndSendTransaction({ ...txs[0], signerId, network: WALLET_NETWORK }));
    } else if (wallet.manifest.features?.signAndSendTransactions) {
      returned.push(...asList(await wallet.signAndSendTransactions({ transactions: txs, signerId, network: WALLET_NETWORK })));
    } else {
      for (const t of txs) returned.push(await wallet.signAndSendTransaction({ ...t, signerId, network: WALLET_NETWORK }));
    }
  } catch (e) {
    error = classify(e);
    if (!returned.length) throw error;
  }
  onSent?.();
  const results = await Promise.all(plan.txs.map((t, i) => (i < returned.length ? verify(i, t, returned[i], signerId) : Promise.resolve<TxResult>({ index: i, receiver: t.receiver, says: t.says, hash: null, status: "not-sent" }))));
  return { results, error };
}
