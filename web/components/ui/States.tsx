import type { ReactNode } from "react";
import { cap } from "@/lib/format";
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

/** A transport failure (a timeout, no connection) in plain words; other errors are already words. */
function plainError(error?: string): { late: boolean; words: string | null } {
  if (!error) return { late: false, words: null };
  if (/timed? ?out|timeout|aborted/i.test(error)) return { late: true, words: null };
  if (/fetch failed|failed to fetch|load failed|networkerror|network error|econn|enotfound|eai_again|socket/i.test(error))
    return { late: false, words: "The connection failed." };
  return { late: false, words: null };
}

/**
 * A failed read, said plainly, with what to do. A whole page that could not load is an alert; a part of a
 * page (`partial`), or a read that only timed out, is a note: the rest of the page stands. The raw error
 * stays one click away.
 */
export function ErrorState({ what, error, children, partial = false }: { what: string; error?: string; children?: ReactNode; partial?: boolean }) {
  const p = plainError(error);
  const technical = p.late || p.words !== null;
  return (
    <div className={`${styles.state} ${partial || p.late ? styles.stateWarn : styles.stateError}`} role={partial ? "status" : "alert"}>
      <Icon name="alert" size={16} />
      <div className="stack" style={{ ["--gap" as string]: "4px" }}>
        <strong>{p.late ? `${cap(what)} did not load in time.` : `Could not read ${what}.`}</strong>
        {p.words ? <span className="small">{p.words}</span> : null}
        {error && !technical ? <span className="mono xsmall muted">{error}</span> : null}
        {children ?? (
          <span className="small">
            The API may be busy; this page tries again on its next load. Every on-chain value here can also be read from NEAR and the EVM chains
            directly.
          </span>
        )}
        {error && technical ? (
          <details className={styles.stateDetails}>
            <summary>Error details</summary>
            <code>{error}</code>
          </details>
        ) : null}
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
