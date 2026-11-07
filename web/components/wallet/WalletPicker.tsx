"use client";

import { useEffect, useRef } from "react";
import { Icon } from "@/components/ui/Icon";
import { ErrorState, Loading } from "@/components/ui/States";
import type { WalletChoice } from "@/lib/wallet/connector";
import { walletInfo } from "@/lib/wallet/manifest";
import type { Load } from "./shared";
import styles from "./wallet.module.css";

const host = (u: string) => {
  try {
    return new URL(u).host;
  } catch {
    return u;
  }
};

/**
 * The wallet picker: a modal dialog (the browser's own, so focus stays inside, Escape closes it and the page
 * behind is inert). Each wallet says how it is used and where its connector script comes from.
 */
export function WalletPicker({ open, wallets, onClose, onPick }: { open: boolean; wallets: Load<WalletChoice[]>; onClose: () => void; onPick: (w: WalletChoice) => void }) {
  const ref = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    if (!open && d.open) d.close();
  }, [open]);

  return (
    <dialog
      ref={ref}
      className={styles.dialog}
      aria-labelledby="wallet-picker-title"
      aria-describedby="wallet-picker-desc"
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      onClick={(e) => {
        // A click on the backdrop (the dialog element itself, outside its content) closes it.
        if (e.target === ref.current) onClose();
      }}
    >
      <div className={styles.dialogBody}>
        <div className={styles.dialogHead}>
          <div className="stack" style={{ ["--gap" as string]: "6px" }}>
            <h2 id="wallet-picker-title">Choose a NEAR wallet</h2>
            <p id="wallet-picker-desc" className="small muted">
              NEAR testnet. The wallet you pick loads its connector script in a sandboxed frame; your keys stay in the wallet.
            </p>
          </div>
          <button type="button" className="btn btn-ghost btn-sm icon-btn" onClick={onClose} aria-label="Close the wallet list">
            <Icon name="x" size={16} />
          </button>
        </div>
        {wallets.status === "loading" ? (
          <Loading label="Loading the wallet list" />
        ) : wallets.status === "error" ? (
          <ErrorState what="the wallet list" error={wallets.error}>
            <span className="small">The connector is part of this site; reload the page to try again.</span>
          </ErrorState>
        ) : wallets.data.length === 0 ? (
          <p className="small muted" role="status">
            No wallet that supports NEAR testnet is available here.
          </p>
        ) : (
          <ul className={styles.walletList}>
            {wallets.data.map((w) => {
              // An extension's own wallet object loads no script from the pinned list.
              const info = w.injected ? undefined : walletInfo(w.id);
              return (
                <li key={w.id}>
                  <button type="button" className={styles.walletBtn} onClick={() => onPick(w)}>
                    <span className={styles.mono40} aria-hidden>
                      {w.name.slice(0, 1).toUpperCase()}
                    </span>
                    <span>
                      <span className={styles.walletName}>{w.name}</span>
                      <span className={styles.walletMeta}>{w.injected ? "Announced by a browser extension you installed" : (info?.kind ?? "")}</span>
                      {info ? (
                        <span className={styles.walletMeta}>
                          Script from {host(info.script.url)}
                          {info.script.pinned ? `, fixed at commit ${info.script.commit}` : ", served by the wallet (not pinned)"} · {info.script.bytes}
                        </span>
                      ) : null}
                    </span>
                    <Icon name="arrow" size={14} />
                  </button>
                </li>
              );
            })}
          </ul>
        )}
        <p className="xsmall muted">
          Listed: wallets that support NEAR testnet and need neither a relay service nor USB. Signing in asks for no access key: the page learns your
          account id.
        </p>
      </div>
    </dialog>
  );
}
