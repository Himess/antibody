import { shortHash } from "@/lib/format";
import { Icon } from "./Icon";
import styles from "./ui.module.css";

/**
 * The verdict as a deterministic stamp: the kind, the broken rules (by name when the program names them,
 * else by slot), and the receipt hash that any re-run of the same input reproduces byte for byte.
 */
export function VerdictStamp({
  kind,
  slots,
  rules,
  severity,
  receipt,
}: {
  kind: string | null;
  slots?: number[];
  /** The broken rules' names, in the slots' order: shown instead of the slot numbers. */
  rules?: string[];
  severity?: string;
  receipt?: string | null;
}) {
  const tone = kind === "Violation" ? "var(--accent-ink)" : "var(--ink-2)";
  const broke = rules && rules.length ? rules.join(", ") : slots && slots.length ? `slot ${slots.join(", ")}` : null;
  return (
    <div className={styles.stamp} style={{ color: tone }} aria-label={`Verdict: ${kind ?? "pending"}`}>
      <span className={styles.stampKind}>{kind === "NoViolation" ? "No violation" : (kind ?? "Pending")}</span>
      <span className={styles.stampMeta}>
        {!kind ? "awaiting the verifier" : broke ?? "no rule broken"}
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
 * A sealed proof: visible, unreadable. Its ciphertext is public once revealed, but only the enclave (and,
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
