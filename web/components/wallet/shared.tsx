import { Icon } from "@/components/ui/Icon";
import { amount } from "@/lib/format";
import { formatUnits, rawOf } from "@/lib/wallet/amounts";
import type { FailureKind } from "@/lib/wallet/connector";
import styles from "./wallet.module.css";

/** A read in progress, done or failed. */
export type Load<T> = { status: "loading" } | { status: "ok"; data: T } | { status: "error"; error: string };

/** A raw 24-decimal amount, rounded for reading, the exact value on hover. */
export function Amount({ raw, unit = "wNEAR" }: { raw: string | bigint | null | undefined; unit?: string }) {
  const v = rawOf(raw);
  if (v === null) return <span className="muted">—</span>;
  return (
    <span className="num" title={`${formatUnits(v)} ${unit} (${v.toString()} raw, 24 decimals)`}>
      {amount(v)} <span className={styles.unit}>{unit}</span>
    </span>
  );
}

const advice: Record<FailureKind, string | null> = {
  rejected: null,
  "not-installed": "Wallets that work here: Meteor Wallet (extension or web), Intear Wallet (web), Nightly (extension) and the NEAR CLI.",
  "popup-blocked": "The wallet opens its own window to ask you. Your browser's address bar shows a blocked pop-up icon.",
  load: "The connector script comes from the address listed under “How the wallet connects”. A blocker or a strict network can stop it.",
  network: "This page works on NEAR testnet only. Testnet accounts end in .testnet (or are implicit accounts).",
  "no-account": "A testnet account needs a little testnet NEAR before it exists on chain.",
  wallet: null,
};

/** A wallet step that did not complete: what happened, what to do, and the wallet's own words. */
export function FailureNote({ kind, message, detail, title }: { kind: FailureKind; message: string; detail?: string; title?: string }) {
  const tip = advice[kind];
  return (
    <div className="callout callout-warn" role="alert">
      <Icon name="alert" size={16} />
      <div className="stack" style={{ ["--gap" as string]: "4px" }}>
        <strong>{title ?? message}</strong>
        {title ? <span>{message}</span> : null}
        {tip ? <span className="small">{tip}</span> : null}
        {detail ? <span className="mono xsmall muted">Wallet said: {detail}</span> : null}
      </div>
    </div>
  );
}

/** A small spinner beside a status line (still when motion is reduced). */
export function Busy({ children }: { children: React.ReactNode }) {
  return (
    <p className={styles.status} role="status">
      <span className={styles.spin} aria-hidden>
        <Icon name="refresh" size={14} />
      </span>
      <span>{children}</span>
    </p>
  );
}
