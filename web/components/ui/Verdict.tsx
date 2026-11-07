import { shortHash } from "@/lib/format";
import { Icon } from "./Icon";
import styles from "./ui.module.css";

/**
 * The verdict as a deterministic stamp: the kind, the broken slots, and the receipt hash that any re-run
 * of the same input reproduces byte for byte.
 */
export function VerdictStamp({ kind, slots, severity, receipt }: { kind: string | null; slots?: number[]; severity?: string; receipt?: string | null }) {
  const tone = kind === "Violation" ? "var(--accent-ink)" : "var(--ink-2)";
  return (
    <div className={styles.stamp} style={{ color: tone }} aria-label={`Verdict: ${kind ?? "pending"}`}>
      <span className={styles.stampKind}>{kind === "NoViolation" ? "No violation" : (kind ?? "Pending")}</span>
      <span className={styles.stampMeta}>
        {!kind ? "awaiting the verifier" : slots && slots.length ? `slot ${slots.join(", ")}` : "no rule broken"}
        {kind && severity ? ` · ${severity}` : ""}
      </span>
      {receipt ? (
        <span className={styles.stampMeta} title={receipt}>
          receipt <span className={styles.stampHash}>{shortHash(receipt, 12)}</span>
        </span>
      ) : null}
    </div>
  );
}

/**
 * A sealed proof: visible, unreadable. Its ciphertext is public once revealed, but only the TEE (and,
 * after payment, the project) can open it until disclosure.
 */
export function Sealed({ seed, label = "Sealed until disclosure", lines = 4 }: { seed: string; label?: string; lines?: number }) {
  // Deterministic filler from the seed (a hash), so the block looks like ciphertext without being any.
  const chars = (seed.replace(/[^0-9a-f]/gi, "") || "a1b2c3d4e5f6").repeat(40).slice(0, lines * 64);
  return (
    <div className={`sealed ${styles.sealedCard}`} role="img" aria-label={label}>
      <div className="sealed-text" aria-hidden>
        {chars}
      </div>
      <div className={styles.sealedBadge} aria-hidden>
        <span>
          <Icon name="lock" size={13} strokeWidth={2} /> {label}
        </span>
      </div>
    </div>
  );
}
