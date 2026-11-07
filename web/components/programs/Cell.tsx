import type { ReactNode } from "react";
import styles from "./programs.module.css";

/**
 * A cell of a table whose rows become cards on phones: the column's name rides along as `data-label`
 * (shown before the value there), and the content sits in one box so it lays out as one value.
 */
export function Cell({ label, lead = false, children }: { label: string; lead?: boolean; children: ReactNode }) {
  return (
    <td data-label={lead ? undefined : label} className={lead ? styles.lead : undefined}>
      <div className={styles.cell}>{children}</div>
    </td>
  );
}
