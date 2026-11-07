import { CopyButton } from "@/components/ui/CopyButton";
import styles from "./checks.module.css";

/** A whole hash to compare by eye: it wraps on a narrow screen (never a page scroll), with a copy button. */
export function FullHash({ value, label }: { value: string | null | undefined; label: string }) {
  if (!value) return <span className="muted">—</span>;
  return (
    <span className={styles.fullHash}>
      <code className={styles.fullHashText} aria-label={`${label}: ${value}`}>
        {value}
      </code>
      <CopyButton value={value} label={`Copy ${label}`} />
    </span>
  );
}
