import type { ReactNode } from "react";
import { Icon } from "./Icon";
import styles from "./ui.module.css";

export function Empty({ children }: { children: ReactNode }) {
  return (
    <div className={styles.state} role="status">
      <Icon name="info" size={16} />
      <div>{children}</div>
    </div>
  );
}

/** A failed read, said plainly, with what to do. */
export function ErrorState({ what, error, children }: { what: string; error?: string; children?: ReactNode }) {
  return (
    <div className={`${styles.state} ${styles.stateError}`} role="alert">
      <Icon name="alert" size={16} />
      <div className="stack" style={{ ["--gap" as string]: "4px" }}>
        <strong>Could not read {what}.</strong>
        {error ? <span className="mono xsmall muted">{error}</span> : null}
        {children ?? <span className="small">The API may be busy; this page retries on its next load. Every value is also readable from NEAR directly.</span>}
      </div>
    </div>
  );
}

export function Skeleton({ width = "100%", height = 16, className }: { width?: number | string; height?: number; className?: string }) {
  return <span className={`skeleton ${className ?? ""}`} style={{ width, height }} aria-hidden />;
}

export function Loading({ label = "Loading" }: { label?: string }) {
  return (
    <div className="stack" role="status" aria-label={label} style={{ ["--gap" as string]: "10px" }}>
      <Skeleton width="40%" height={14} />
      <Skeleton width="85%" />
      <Skeleton width="70%" />
      <span className="visually-hidden">{label}…</span>
    </div>
  );
}
