import type { ReactNode } from "react";
import styles from "./ui.module.css";

/**
 * Commands, hashes and live checks, collapsed under one line ("Check it yourself"): every check stays on the
 * page, one click away, and the claim above it reads without them.
 */
export function Disclosure({ summary = "Check it yourself", open = false, className, children }: { summary?: ReactNode; open?: boolean; className?: string; children: ReactNode }) {
  return (
    <details className={`${styles.disclosure} ${className ?? ""}`} open={open || undefined}>
      <summary>{summary}</summary>
      <div className={styles.disclosureBody}>{children}</div>
    </details>
  );
}
