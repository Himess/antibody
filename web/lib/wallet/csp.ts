/**
 * What the account page's wallet connection needs from the Content-Security-Policy (web/proxy.ts adds it to
 * connect-src). A wallet's sandboxed iframe is a srcdoc document, so it inherits this policy: every request
 * its script makes is held to the same list.
 *
 * The policy is the same on every page because a client-side navigation to /account keeps the policy of
 * the page the visitor landed on. Only the account page's code ever connects to these.
 *
 * Kept in step with lib/wallet/manifest.ts by lib/wallet/wallet.test.ts.
 */
export const WALLET_CONNECT_SRC: readonly string[] = [
  // The connector scripts, each by its exact path (a CSP path match), so no other file on these hosts
  // can be fetched. Three are fixed to a commit; Intear Wallet serves its own and cannot be pinned.
  "https://raw.githubusercontent.com/Meteor-Wallet/meteor_wallet_sdk/ef1be35fc3712f2de60a7563aed543d78e73f763/storage/meteor-near-connect-latest.js",
  "https://raw.githubusercontent.com/hot-dao/near-selector/9cee11c04cfd42f5a420688f808d4aca612b5fde/repository/nightly.js",
  "https://raw.githubusercontent.com/near/near-cli-connect/2b8c4fdc5da03d7e6688a7f84ebce99e480c9c26/dist/near-cli.js",
  "https://wallet.intear.tech/near-selector.js",
  // What those scripts reach from their sandbox: Meteor Wallet's mobile bridge, Intear Wallet's desktop
  // app bridge, and the testnet RPCs Meteor Wallet, Nightly and the NEAR CLI connector read.
  "https://mc.meteorwallet.app",
  "wss://logout-bridge-service.intear.tech",
  "https://test.rpc.fastnear.com",
  "https://rpc.testnet.near.org",
];
