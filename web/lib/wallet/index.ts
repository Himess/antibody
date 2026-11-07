/**
 * The account page's NEAR wallet connection.
 *
 * The connector: NEAR Connect (npm @hot-labs/near-connect, 0.12.0, MIT, pinned exactly in package.json).
 * Why it, over the modular selector packages (a core, a modal and one npm module per wallet):
 * - Maintained: releases every few weeks, the latest on 2026-10-26; the modular packages' latest is from
 *   2025-12-04. Neither is deprecated.
 * - Small: no runtime dependencies (about 100 KB of unminified JS, loaded as its own chunk on /account
 *   only, when someone connects or a saved session is restored). The modular packages pull in the NEAR JS
 *   libraries and a module per wallet.
 * - Isolated: each wallet's code runs in a sandboxed iframe (an opaque origin, no access to this page, its
 *   cookies or its storage) and reaches the page only through the bridge calls its manifest permits. The
 *   modular packages run every wallet module in the page itself.
 * - It works with this site's nonce-based CSP: it takes the request's nonce for the iframe's inline
 *   scripts (`cspNonce`).
 *
 * The cost, and how it is contained: by default the connector fetches its wallet list from a branch of a
 * repository at run time, and each wallet's script from that wallet's chosen URL. Here it never fetches a
 * list (./manifest.ts gives it a pinned copy: the four wallets that support testnet, sign with a NEAR
 * account and need neither a relay service nor USB), three of the four scripts are fixed to a commit, and
 * the CSP allows each script by its exact path (./csp.ts). Nothing loads before a person picks a wallet.
 * The connector's own wallet picker is not used (it is not keyboard-accessible and offers pasting a custom
 * wallet manifest): the page draws its own, and refuses any wallet another window announces.
 *
 * What the page does with a wallet: reads the account id, and asks it to sign the transactions a person
 * reviewed (./tx.ts), each checked afterwards on NEAR RPC. It keeps no key and adds none, and never asks
 * for a message signature (docs/threat-model.md P13).
 */
export * from "./amounts";
export * from "./contracts";
export * from "./manifest";
export * from "./tx";
export * from "./types";
export { checkContracts, isImplicit, isTestnetAccount, loadAccount, loadActivity, loadHistory, loadNearBalance, loadTokenRegistration, validAccountId } from "./account";
export {
  WalletFailure,
  classify,
  connectWallet,
  disconnectWallet,
  failureText,
  getConnector,
  hasSavedSession,
  listWallets,
  onWalletSignOut,
  restoreSession,
  sendPlan,
} from "./connector";
export type { FailureKind, Session, TxResult, WalletChoice } from "./connector";
