import type { ReactNode } from "react";
import { Icon } from "@/components/ui/Icon";
import { sourceUrl } from "@/lib/config";
import styles from "./build.module.css";

/** A file in the repository at the deployed commit, opened in a new tab. */
export function DocLink({ path, children, hash }: { path: string; children?: ReactNode; hash?: string }) {
  return (
    <a href={`${sourceUrl(path)}${hash ? `#${hash}` : ""}`} target="_blank" rel="noreferrer noopener" className={styles.docLink}>
      {children ?? <code>{path}</code>}
      <Icon name="external" size={11} className={styles.inlineIcon} />
      <span className="visually-hidden"> (opens in a new tab)</span>
    </a>
  );
}
