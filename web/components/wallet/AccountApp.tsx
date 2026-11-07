"use client";

import { useCallback, useEffect, useRef, useState, useSyncExternalStore, type FormEvent } from "react";
import { Icon } from "@/components/ui/Icon";
import { config } from "@/lib/config";
import { isTestnetAccount } from "@/lib/wallet/account";
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

/** Accounts to read with no wallet: our fixture agent A (JUDGES.md) and a practice try from the recorded run. */
const EXAMPLES = [
  { id: "agent-a.antibody.testnet", note: "Our fixture agent A, paid in the one moment" },
  { id: "judge-1-p6.practice.antibody.testnet", note: "A recorded practice try: an agent with a budget and a cap" },
] as const;

/** The account a link names for the read-only view (`/account?view=<id>`), if it is a testnet account. */
function viewParam(): string | null {
  try {
    const id = (new URLSearchParams(window.location.search).get("view") ?? "").trim().toLowerCase();
    return id && isTestnetAccount(id) ? id : null;
  } catch {
    return null;
  }
}

/** Keeps `?view=` in step with the read-only view, so its address can be shared. No navigation. */
function setViewParam(id: string | null) {
  try {
    const u = new URL(window.location.href);
    if (id) u.searchParams.set("view", id);
    else u.searchParams.delete("view");
    window.history.replaceState(null, "", `${u.pathname}${u.search}${u.hash}`);
  } catch {
    // The address is a convenience: the view works without it.
  }
}

/**
 * The account page's client part: connect a wallet (or restore the last one), then the account. Before
 * that click nothing loads from any wallet; the connector itself loads only when needed. With no wallet,
 * any account can be read: the same public reads, read-only, with nothing to sign.
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

  // The read-only view: the account a link names, until the person picks one here (null: closed).
  const linked = useSyncExternalStore(noSubscribe, viewParam, () => null);
  const [picked, setPicked] = useState<string | null | undefined>(undefined);
  const viewing = picked === undefined ? linked : picked;
  const [viewKey, setViewKey] = useState(0);
  const [draft, setDraft] = useState("");
  const [lookupError, setLookupError] = useState<string | null>(null);
  const [backToOwn, setBackToOwn] = useState(false);
  const lookupInput = useRef<HTMLInputElement>(null);

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
      // The person's own account replaces any read-only view.
      setPicked(null);
      setViewParam(null);
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

  const openView = useCallback((id: string) => {
    setLookupError(null);
    setPicked(id);
    setViewKey((k) => k + 1);
    setViewParam(id);
    setAnnounce(`Showing ${id}, read-only.`);
  }, []);

  const closeView = useCallback(() => {
    setPicked(null);
    setViewParam(null);
    setAnnounce("Read-only view closed.");
    if (session) setBackToOwn(true);
    else requestAnimationFrame(() => lookupInput.current?.focus());
  }, [session]);

  const submitView = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const id = draft.trim().toLowerCase();
    if (!isTestnetAccount(id)) {
      setLookupError("Enter a NEAR testnet account: a name ending in .testnet, or an implicit account.");
      lookupInput.current?.focus();
      return;
    }
    openView(id);
  };

  const wrongNetwork = config.network !== WALLET_NETWORK;
  const apiMismatch = apiContracts && ((apiContracts.antibody && apiContracts.antibody !== ANTIBODY) || (apiContracts.token && apiContracts.token !== WNEAR));
  const restoring = saved && !restored && !session;

  const readOnly = viewing ? (
    <AccountOverview
      key={`${viewing}:${viewKey}`}
      accountId={viewing}
      session={null}
      onClose={closeView}
      closeLabel={session ? "Back to your account" : "Close"}
      blocked={null}
      apiContracts={apiContracts}
      focusOnMount
    />
  ) : null;

  return (
    <>
      <p className="visually-hidden" aria-live="polite">
        {announce}
      </p>
      {session ? (
        (readOnly ?? (
          <AccountOverview
            key={session.accountId}
            accountId={session.accountId}
            session={session}
            onClose={disconnect}
            blocked={wrongNetwork ? "network" : apiMismatch ? "api" : null}
            apiContracts={apiContracts}
            focusOnMount={byClick || backToOwn}
          />
        ))
      ) : (
        <>
          <section className="page" aria-labelledby="connect-title" style={{ paddingBottom: 40 }}>
            <div className={`card ${styles.connect}`}>
              <div className={styles.connectHead}>
                <div className="stack" style={{ ["--gap" as string]: "8px" }}>
                  <p className="eyebrow">NEAR testnet only</p>
                  <h2 id="connect-title">Connect a NEAR wallet</h2>
                  <p className="muted small" style={{ maxWidth: "62ch" }}>
                    Your wallet tells this page your account id. The page then reads your credit, commits and payouts, and asks the
                    wallet to sign only the transactions you review first.
                  </p>
                </div>
                <div className={styles.connectActions}>
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
                  <a href="#lookup" className={styles.connectAlt}>
                    No wallet? Read any account
                  </a>
                </div>
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

            <form id="lookup" className={`card-flat ${styles.lookup}`} onSubmit={submitView} noValidate aria-labelledby="lookup-title">
              <div className="stack" style={{ ["--gap" as string]: "6px" }}>
                <h2 id="lookup-title" className={styles.lookupTitle}>
                  No wallet? Read any account
                </h2>
                <p className="muted small" style={{ maxWidth: "62ch" }}>
                  Credit, agent statements, commits and payouts at near-antibody are public. This view reads them with no wallet, and has
                  nothing to sign.
                </p>
              </div>
              <div className={`${styles.field} ${styles.lookupField}`}>
                <label htmlFor="lookup-id" className="label">
                  NEAR testnet account
                </label>
                <div className={styles.inputRow}>
                  <input
                    ref={lookupInput}
                    id="lookup-id"
                    className="input mono"
                    value={draft}
                    onChange={(e) => {
                      setDraft(e.target.value);
                      setLookupError(null);
                    }}
                    placeholder="name.testnet"
                    autoComplete="off"
                    autoCapitalize="none"
                    autoCorrect="off"
                    spellCheck={false}
                    enterKeyHint="go"
                    aria-invalid={lookupError ? true : undefined}
                    aria-describedby={lookupError ? "lookup-error" : undefined}
                  />
                  <button type="submit" className="btn">
                    <Icon name="eye" size={16} /> View
                  </button>
                </div>
                {lookupError ? (
                  <p id="lookup-error" className={styles.error} role="alert">
                    {lookupError}
                  </p>
                ) : null}
              </div>
              <div className="stack" style={{ ["--gap" as string]: "8px" }}>
                <p className="xsmall muted">Or open an example:</p>
                <ul className={styles.examples}>
                  {EXAMPLES.map((x) => (
                    <li key={x.id}>
                      <button
                        type="button"
                        className={styles.example}
                        onClick={() => {
                          setDraft(x.id);
                          openView(x.id);
                        }}
                      >
                        <span className={styles.exampleId}>{x.id}</span>
                        <span className={styles.exampleNote}>{x.note}</span>
                      </button>
                    </li>
                  ))}
                </ul>
              </div>
            </form>
          </section>
          {readOnly}
        </>
      )}
      <WalletPicker open={pickerOpen} wallets={wallets} onClose={closePicker} onPick={pick} />
    </>
  );
}
