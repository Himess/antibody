"use client";

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { Icon } from "@/components/ui/Icon";
import { config } from "@/lib/config";
import {
  WalletFailure,
  connectWallet,
  disconnectWallet,
  hasSavedSession,
  listWallets,
  onWalletSignOut,
  restoreSession,
  type Session,
  type WalletChoice,
} from "@/lib/wallet/connector";
import { ANTIBODY, WALLET_NETWORK, WNEAR } from "@/lib/wallet/contracts";
import { AccountOverview } from "./AccountOverview";
import { Busy, FailureNote, type Load } from "./shared";
import { WalletPicker } from "./WalletPicker";
import styles from "./wallet.module.css";

/** What the API's /health names as the contracts, for comparison with the ones fixed in this page. */
export type ApiContracts = { antibody: string | null; token: string | null } | null;

const noSubscribe = () => () => undefined;

/**
 * The account page's client part: connect a wallet (or restore the last one), then the account. Before
 * that click nothing loads from any wallet; the connector itself loads only when needed.
 */
export function AccountApp({ apiContracts }: { apiContracts: ApiContracts }) {
  // Read after hydration (the server renders "no saved session"), so the first paint matches.
  const saved = useSyncExternalStore(noSubscribe, hasSavedSession, () => false);
  const [session, setSession] = useState<Session | null>(null);
  const [restored, setRestored] = useState(false);
  const [connecting, setConnecting] = useState<WalletChoice | null>(null);
  const [failure, setFailure] = useState<WalletFailure | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [wallets, setWallets] = useState<Load<WalletChoice[]>>({ status: "loading" });
  const [announce, setAnnounce] = useState("");
  const [byClick, setByClick] = useState(false);
  const connectBtn = useRef<HTMLButtonElement>(null);

  // A saved session from an earlier visit: restore it (the wallet's script loads from its cache). Not after
  // a connect made on this visit, which saves the same choice.
  useEffect(() => {
    if (!saved || byClick) return;
    let live = true;
    restoreSession()
      .then((s) => {
        if (!live) return;
        setSession(s);
        if (s) setAnnounce(`Connected as ${s.accountId}.`);
      })
      .catch((e: unknown) => {
        if (live) setFailure(e instanceof WalletFailure ? e : new WalletFailure("wallet", "Could not restore your wallet session.", String(e)));
      })
      .finally(() => {
        if (live) setRestored(true);
      });
    return () => {
      live = false;
    };
  }, [saved, byClick]);

  // The wallet may sign out on its own (from its window or extension).
  useEffect(() => {
    if (!session) return;
    let stop: (() => void) | undefined;
    let live = true;
    onWalletSignOut(() => {
      setSession(null);
      setAnnounce("Your wallet signed out.");
    })
      .then((s) => {
        if (live) stop = s;
        else s();
      })
      .catch(() => undefined);
    return () => {
      live = false;
      stop?.();
    };
  }, [session]);

  const openPicker = useCallback(() => {
    setFailure(null);
    setPickerOpen(true);
    setWallets({ status: "loading" });
    listWallets()
      .then((list) => setWallets({ status: "ok", data: list }))
      .catch((e: unknown) => setWallets({ status: "error", error: e instanceof WalletFailure ? `${e.message}${e.detail ? ` ${e.detail}` : ""}` : String(e) }));
  }, []);

  const closePicker = useCallback(() => {
    setPickerOpen(false);
    connectBtn.current?.focus();
  }, []);

  const pick = useCallback(async (w: WalletChoice) => {
    setPickerOpen(false);
    setConnecting(w);
    setFailure(null);
    setAnnounce(`Opening ${w.name}…`);
    try {
      const s = await connectWallet(w.id);
      setByClick(true);
      setSession(s);
      setAnnounce(`Connected as ${s.accountId}.`);
    } catch (e) {
      setFailure(e instanceof WalletFailure ? e : new WalletFailure("wallet", "The wallet reported an error.", String(e)));
      setAnnounce("Not connected.");
      // Back to the button once it is enabled again (after this render).
      requestAnimationFrame(() => connectBtn.current?.focus());
    } finally {
      setConnecting(null);
    }
  }, []);

  const disconnect = useCallback(async () => {
    await disconnectWallet();
    setSession(null);
    setAnnounce("Disconnected. This page no longer knows your account.");
    requestAnimationFrame(() => connectBtn.current?.focus());
  }, []);

  const wrongNetwork = config.network !== WALLET_NETWORK;
  const apiMismatch = apiContracts && ((apiContracts.antibody && apiContracts.antibody !== ANTIBODY) || (apiContracts.token && apiContracts.token !== WNEAR));
  const restoring = saved && !restored && !session;

  return (
    <>
      <p className="visually-hidden" aria-live="polite">
        {announce}
      </p>
      {session ? (
        <AccountOverview
          key={session.accountId}
          session={session}
          onDisconnect={disconnect}
          blocked={wrongNetwork ? "network" : apiMismatch ? "api" : null}
          apiContracts={apiContracts}
          focusOnMount={byClick}
        />
      ) : (
        <section className="page" aria-labelledby="connect-title" style={{ paddingBottom: 40 }}>
          <div className={`card ${styles.connect}`}>
            <div className={styles.connectHead}>
              <div className="stack" style={{ ["--gap" as string]: "8px" }}>
                <p className="eyebrow">NEAR testnet only</p>
                <h2 id="connect-title">Connect a NEAR wallet</h2>
                <p className="muted small" style={{ maxWidth: "62ch" }}>
                  Your wallet tells this page your account id. The page then reads your credit, commits and payouts, and asks the
                  wallet to sign only the transactions you review first. wNEAR on testnet has no value.
                </p>
              </div>
              <button
                ref={connectBtn}
                type="button"
                className="btn btn-primary"
                onClick={openPicker}
                disabled={Boolean(connecting) || restoring || wrongNetwork}
                aria-haspopup="dialog"
              >
                <Icon name="wallet" size={16} /> Connect a NEAR wallet
              </button>
            </div>
            {wrongNetwork ? (
              <div className="callout callout-warn" role="alert">
                <Icon name="alert" size={16} />
                <span>
                  This site is built for NEAR {config.network}, and the account page signs on NEAR testnet only. Connecting is off.
                </span>
              </div>
            ) : null}
            {restoring ? <Busy>Reconnecting to the wallet you used here before…</Busy> : null}
            {connecting ? <Busy>Waiting for {connecting.name}. Approve or reject the request in its window.</Busy> : null}
            {failure ? <FailureNote kind={failure.kind} message={failure.message} detail={failure.detail} title="Not connected" /> : null}
            <ul className={styles.points}>
              <li>
                <h3>What you see</h3>
                <ul>
                  <li>Your token credit and NEAR credit at near-antibody, and the wNEAR in your wallet</li>
                  <li>Your commits and payouts, each linked to its record</li>
                  <li>Your agent statement, or the agents you registered</li>
                </ul>
              </li>
              <li>
                <h3>What you can do</h3>
                <ul>
                  <li>Wrap NEAR and deposit wNEAR into your credit</li>
                  <li>Withdraw token credit or NEAR credit</li>
                  <li>Register an agent, or change its budget</li>
                </ul>
              </li>
              <li>
                <h3>What never happens</h3>
                <ul>
                  <li>No key is stored here or asked for: your wallet keeps its keys</li>
                  <li>No message signature is asked for, ever</li>
                  <li>Nothing is signed that you did not review here first</li>
                </ul>
              </li>
            </ul>
            <p className="xsmall muted">
              Nothing loads from a wallet until you pick one; then only that wallet&apos;s connector script, from the address listed
              under <a href="#how-it-connects">How the wallet connects</a>.
            </p>
          </div>
        </section>
      )}
      <WalletPicker open={pickerOpen} wallets={wallets} onClose={closePicker} onPick={pick} />
    </>
  );
}
