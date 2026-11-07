import { shortHash } from "@/lib/format";
import { CopyButton } from "./CopyButton";
import { Icon } from "./Icon";
import styles from "./ui.module.css";

/**
 * A hash, account or address: truncated, the full value on hover and focus (title), a copy button, and
 * a link when one is given.
 */
export function Hash({
  value,
  href,
  head = 8,
  tail = 0,
  copy = true,
  label,
  full = false,
  wrap,
}: {
  value: string | null | undefined;
  href?: string | null;
  head?: number;
  tail?: number;
  copy?: boolean;
  label?: string;
  full?: boolean;
  /** Let the value wrap (default: when shown in full or longer than 24 characters). */
  wrap?: boolean;
}) {
  if (!value) return <span className="muted">—</span>;
  const text = full ? value : shortHash(value, head, tail);
  const wraps = wrap ?? (full || text.length > 24);
  // The accessible name starts with the visible text (WCAG 2.5.3), then the full value and what the link does.
  const inner = (
    <>
      <span className={`mono ${styles.hashText}`} title={value}>
        {text}
      </span>
      {text !== value || label || href ? (
        <span className="visually-hidden">
          {`, ${label ?? (href ? "link" : "value")}: ${value}${href ? ", opens an explorer in a new tab" : ""}`}
        </span>
      ) : null}
    </>
  );
  return (
    <span className={`${styles.hash} ${wraps ? styles.wrap : ""}`}>
      {href ? (
        <a href={href} target="_blank" rel="noreferrer noopener" className={styles.hashLink}>
          {inner}
          <Icon name="external" size={12} />
        </a>
      ) : (
        <span>{inner}</span>
      )}
      {copy ? <CopyButton value={value} label={label ? `Copy ${label}` : "Copy"} /> : null}
    </span>
  );
}
