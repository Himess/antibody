import type { ReactNode } from "react";
import { Icon, type IconName } from "@/components/ui/Icon";
import styles from "./build.module.css";

/** One thing a page states, with the evidence that checks it underneath. */
export function Claim({
  icon,
  title,
  status,
  children,
  evidence,
  evidenceTitle = "Check it",
}: {
  icon: IconName;
  title: string;
  status?: ReactNode;
  children: ReactNode;
  evidence?: ReactNode;
  evidenceTitle?: string;
}) {
  return (
    <div className={`card ${styles.claim}`}>
      <div className={styles.claimHead}>
        <span className={styles.claimIcon} aria-hidden>
          <Icon name={icon} size={17} />
        </span>
        <h3>{title}</h3>
        {status}
      </div>
      <div className={styles.claimText}>{children}</div>
      {evidence ? (
        <div className={styles.evidence}>
          <p className={styles.evidenceTitle}>{evidenceTitle}</p>
          {evidence}
        </div>
      ) : null}
    </div>
  );
}
