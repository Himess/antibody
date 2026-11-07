import type { ReactNode } from "react";
import styles from "./programs.module.css";

/**
 * A cell of a table whose rows become cards on phones: the column's name rides along as `data-label`
 * (shown before the value there), and the content sits in one box so it lays out as one value.
 */
export function Cell({ label, lead = false, className, children }: { label: string; lead?: boolean; className?: string; children: ReactNode }) {
  const cls = [lead ? styles.lead : "", className ?? ""].filter(Boolean).join(" ");
  return (
    <td data-label={lead ? undefined : label} className={cls || undefined}>
      <div className={styles.cell}>{children}</div>
    </td>
  );
}
