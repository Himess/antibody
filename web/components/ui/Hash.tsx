import { Fragment, type ReactNode } from "react";
import { shortHash } from "@/lib/format";
import { CopyButton } from "./CopyButton";
import { Icon } from "./Icon";
import { SiteLink } from "./SiteLink";
import styles from "./ui.module.css";

/** A dotted name with a line-break opportunity after each dot: "judge-1-p6.practice." / "antibody.testnet". */
function breakAfterDots(text: string): ReactNode {
  const parts = text.split(".");
  return parts.map((part, i) =>
    i < parts.length - 1 ? (
      <Fragment key={i}>
        {part}.<wbr />
      </Fragment>
    ) : (
      <Fragment key={i}>{part}</Fragment>
    ),
  );
}

/**
 * A hash, account or address: truncated, the full value on hover and focus (title), a copy button, and
 * a link when one is given: to an explorer (a new tab), or to this site's page for it, with the explorer
 * as a small icon beside it.
 */
export function Hash({
  value,
  href,
  to,
  head = 8,
  tail = 0,
  copy = true,
  label,
  full = false,
  wrap,
}: {
  value: string | null | undefined;
  /** An explorer page for the value, in a new tab: the value links there, or (with `to`) an icon beside it. */
  href?: string | null;
  /** This site's page for the value: the value links there, in the same tab. */
  to?: string;
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
  // An account wraps at its dots; a hash (no words to keep whole) wraps anywhere.
  const dotted = wraps && text.includes(".");
  // The value itself opens the explorer only when it has no page on this site.
  const external = to ? null : href;
  // The accessible name starts with the visible text (WCAG 2.5.3), then the full value and what the link does.
  const inner = (
    <>
      <span className={`mono ${styles.hashText} ${dotted ? styles.dotted : ""}`} title={value}>
        {dotted ? breakAfterDots(text) : text}
      </span>
      {text !== value || label || external || to ? (
        <span className="visually-hidden">
          {`, ${label ?? (external || to ? "link" : "value")}: ${value}${external ? ", opens an explorer in a new tab" : ""}`}
        </span>
      ) : null}
    </>
  );
  return (
    <span className={`${styles.hash} ${wraps ? styles.wrap : ""}`}>
      {to ? (
        <SiteLink href={to} className={`${styles.hashLink} ${styles.hashBody}`}>
          {inner}
        </SiteLink>
      ) : external ? (
        <a href={external} target="_blank" rel="noreferrer noopener" className={`${styles.hashLink} ${styles.hashBody}`}>
          {inner}
          <Icon name="external" size={12} />
        </a>
      ) : (
        <span className={styles.hashBody}>{inner}</span>
      )}
      {to && href ? (
        <a
          href={href}
          target="_blank"
          rel="noreferrer noopener"
          className={styles.iconLink}
          aria-label={`Open ${value} in an explorer, new tab`}
          title="Open in an explorer"
        >
          <Icon name="external" size={12} />
        </a>
      ) : null}
      {copy ? <CopyButton value={value} label={label ? `Copy ${label}` : "Copy"} /> : null}
    </span>
  );
}
