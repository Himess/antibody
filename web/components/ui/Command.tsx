import type { ReactNode } from "react";
import { CopyButton } from "./CopyButton";
import styles from "./ui.module.css";

/** Tokens up to this length stay whole: they fit a phone's line, and a flag must never split at its hyphens. */
const WHOLE = 24;

/** One line, its short tokens unbreakable; it wraps only at its spaces (or inside a token too long for a line). */
function tokens(line: string): ReactNode[] {
  return line.split(/( +)/).map((part, i) =>
    part && part.trim() && part.length <= WHOLE ? (
      <span key={i} className={styles.tok}>
        {part}
      </span>
    ) : (
      part
    ),
  );
}

/**
 * A shell command (or JSON) to copy: no leading prompt in the copied text. On screen each line wraps with its
 * continuation hanging under it, so nothing is cut off; the copy button copies the exact text.
 */
export function Command({ children, prompt = true, label }: { children: string; prompt?: boolean; label?: string }) {
  return (
    <div className={styles.command} role="group" aria-label={label ?? "Command"}>
      {prompt ? (
        <span className={styles.prompt} aria-hidden>
          $
        </span>
      ) : null}
      <pre tabIndex={0}>
        <code>
          {children.split("\n").map((line, i) => (
            <span key={i} className={styles.line}>
              {tokens(line)}
            </span>
          ))}
        </code>
      </pre>
      <CopyButton value={children} label={label ? `Copy ${label}` : "Copy command"} />
    </div>
  );
}
