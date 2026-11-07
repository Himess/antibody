import { CopyButton } from "./CopyButton";
import styles from "./ui.module.css";

/** A shell command (or JSON) to copy: no leading prompt in the copied text. */
export function Command({ children, prompt = true, label }: { children: string; prompt?: boolean; label?: string }) {
  return (
    <div className={styles.command} role="group" aria-label={label ?? "Command"}>
      {prompt ? (
        <span className={styles.prompt} aria-hidden>
          $
        </span>
      ) : null}
      <pre tabIndex={0}>
        <code>{children}</code>
      </pre>
      <CopyButton value={children} label={label ? `Copy ${label}` : "Copy command"} />
    </div>
  );
}
