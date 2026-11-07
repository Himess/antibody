/**
 * The wallets this page offers, pinned: a copy of the connector's wallet manifest (the four entries that
 * support NEAR testnet, sign with a NEAR account, and need neither a relay service nor USB, which this
 * site's Permissions-Policy turns off), with each wallet's connector script fixed to a commit wherever its
 * host allows. The connector never fetches a manifest: it gets this list.
 *
 * What runs where: when someone picks a wallet, the connector fetches that wallet's script (`executor`) and
 * runs it in a sandboxed iframe (`sandbox="allow-scripts"`, an opaque origin: no access to this page, its
 * cookies or its storage). The script talks to the page only through the bridge calls its `permissions`
 * allow (open one of the `allowsOpen` URLs in a new window, keep its own session in this site's storage
 * under its id, reach an extension object named in `external`). Nothing is fetched before that click.
 *
 * Changes to this list must keep lib/wallet/csp.ts in step (a unit test checks it). Upstream sources, read
 * on 2026-11-07: manifest at hot-dao/near-selector@9cee11c, executors at the commits below.
 */
import type { WalletManifest } from "@hot-labs/near-connect";

/** A neutral monogram, inline (no image request to a wallet's server). */
function monogram(letter: string): string {
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="#23272a"/>` +
    `<text x="32" y="42" font-family="system-ui,sans-serif" font-size="28" font-weight="600" fill="#edeeeb" text-anchor="middle">${letter}</text></svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

/** How a wallet is used, in plain words, and what its script contacts besides this page. */
export interface WalletInfo {
  id: string;
  name: string;
  kind: string;
  /** Where the connector script comes from, and whether that URL is fixed to a commit. */
  script: { url: string; pinned: boolean; commit?: string; bytes: string };
  /** Origins the wallet's script reaches from its sandbox (the page's CSP must allow them). */
  contacts: { origin: string; why: string }[];
  /** Windows it may open (outside the page; not governed by its CSP). */
  opens: string[];
}

export const WALLET_INFO: WalletInfo[] = [
  {
    id: "meteor-wallet",
    name: "Meteor Wallet",
    kind: "Browser extension or web wallet",
    script: {
      url: "https://raw.githubusercontent.com/Meteor-Wallet/meteor_wallet_sdk/ef1be35fc3712f2de60a7563aed543d78e73f763/storage/meteor-near-connect-latest.js",
      pinned: true,
      commit: "ef1be35",
      bytes: "2.0 MB",
    },
    contacts: [
      { origin: "https://test.rpc.fastnear.com", why: "reads NEAR testnet" },
      { origin: "https://mc.meteorwallet.app", why: "its bridge to the mobile app" },
    ],
    opens: ["https://wallet.meteorwallet.app", "https://meteorwallet.app", "https://chromewebstore.google.com"],
  },
  {
    id: "intear-wallet",
    name: "Intear Wallet",
    kind: "Web wallet or desktop app",
    script: {
      url: "https://wallet.intear.tech/near-selector.js",
      pinned: false,
      bytes: "14 KB",
    },
    contacts: [{ origin: "wss://logout-bridge-service.intear.tech", why: "its bridge to the desktop app" }],
    opens: ["https://wallet.intear.tech"],
  },
  {
    id: "nightly-wallet",
    name: "Nightly",
    kind: "Desktop browser extension",
    script: {
      url: "https://raw.githubusercontent.com/hot-dao/near-selector/9cee11c04cfd42f5a420688f808d4aca612b5fde/repository/nightly.js",
      pinned: true,
      commit: "9cee11c",
      bytes: "1.2 MB",
    },
    contacts: [{ origin: "https://test.rpc.fastnear.com", why: "reads NEAR testnet" }],
    opens: ["https://nightly.app", "https://chromewebstore.google.com"],
  },
  {
    id: "near-cli",
    name: "NEAR CLI",
    kind: "Your terminal: you run the command it shows and paste the result",
    script: {
      url: "https://raw.githubusercontent.com/near/near-cli-connect/2b8c4fdc5da03d7e6688a7f84ebce99e480c9c26/dist/near-cli.js",
      pinned: true,
      commit: "2b8c4fd",
      bytes: "56 KB",
    },
    contacts: [{ origin: "this site's NEAR RPC", why: "looks up the transaction you sent" }],
    opens: [],
  },
];

const features = (f: Partial<WalletManifest["features"]>): WalletManifest["features"] => ({
  signMessage: false,
  signTransaction: false,
  signAndSendTransaction: true,
  signAndSendTransactions: true,
  signInWithoutAddKey: true,
  signInAndSignMessage: false,
  signInWithFunctionCallKey: false,
  signDelegateActions: false,
  mainnet: false,
  testnet: true,
  ...f,
});

const scriptOf = (id: string) => WALLET_INFO.find((w) => w.id === id)!.script;

/**
 * The manifest entries. `version` carries the script's commit: the connector caches a script under
 * `<id>:<version>`, so a new pin is never served from an old cache. Permissions are upstream's, less the
 * development hosts. `features` declares only what this page uses: no message signing, no keys added.
 */
export const PINNED_WALLETS: WalletManifest[] = [
  {
    id: "meteor-wallet",
    name: "Meteor Wallet",
    icon: monogram("M"),
    description: "Browser extension or web wallet",
    website: "https://meteorwallet.app/",
    version: `1.1.1+${scriptOf("meteor-wallet").commit}`,
    executor: scriptOf("meteor-wallet").url,
    type: "sandbox",
    platform: [],
    features: features({}),
    permissions: {
      storage: true,
      allowsOpen: ["https://wallet.meteorwallet.app", "https://meteorwallet.app", "https://chromewebstore.google.com", "meteorwallet://"],
      external: ["meteorCom", "meteorComV2"],
    },
  },
  {
    id: "intear-wallet",
    name: "Intear Wallet",
    icon: monogram("I"),
    description: "Web wallet or desktop app",
    website: "https://wallet.intear.tech/",
    version: "1.0.0+unpinned",
    executor: scriptOf("intear-wallet").url,
    type: "sandbox",
    platform: [],
    features: features({}),
    permissions: {
      storage: true,
      allowsOpen: ["https://wallet.intear.tech", "intear://", "https://logout-bridge-service.intear.tech"],
    },
  },
  {
    id: "nightly-wallet",
    name: "Nightly",
    icon: monogram("N"),
    description: "Desktop browser extension",
    website: "https://nightly.app",
    version: `1.0.0+${scriptOf("nightly-wallet").commit}`,
    executor: scriptOf("nightly-wallet").url,
    type: "sandbox",
    platform: [],
    features: features({}),
    permissions: {
      storage: true,
      external: ["nightly.near"],
      allowsOpen: ["https://nightly.app", "https://chromewebstore.google.com/detail/nightly/fiikommddbeccaoicoejoniammnalkfa"],
    },
  },
  {
    id: "near-cli",
    name: "NEAR CLI",
    icon: monogram(">"),
    description: "Sign in your terminal",
    website: "https://near.cli.rs",
    version: `1.0.0+${scriptOf("near-cli").commit}`,
    executor: scriptOf("near-cli").url,
    type: "sandbox",
    platform: [],
    features: features({}),
    permissions: { storage: true, clipboardWrite: true },
  },
];

export const PINNED_IDS = new Set(PINNED_WALLETS.map((w) => w.id));

/** A fresh copy for the connector, which rewrites the list it is given. */
export function pinnedManifest(): { version: string; wallets: WalletManifest[] } {
  return { version: "antibody-1", wallets: PINNED_WALLETS.map((w) => JSON.parse(JSON.stringify(w)) as WalletManifest) };
}

export const walletInfo = (id: string | null | undefined) => WALLET_INFO.find((w) => w.id === id);
