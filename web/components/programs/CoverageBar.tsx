import { Pill } from "@/components/ui/Pill";
import { amount, TOKEN } from "@/lib/format";
import { coverageText } from "./helpers";
import styles from "./programs.module.css";

/** The bar runs from 0 to 3× the largest payout; the tick marks 1×, one largest payout covered. */
const SCALE = 3;

/**
 * Escrow coverage: the available escrow over the largest payout (rules.escrow_coverage), as a small bar
 * with both numbers. Below 1× the status alert fires (`/v1/status`), and the words say so too.
 */
export function CoverageBar({
  coverage,
  available,
  largest,
  scale = false,
}: {
  coverage: number | null;
  available?: string | null;
  largest?: string | null;
  /** Show the bar's 0 / 1× / 3× scale under it. */
  scale?: boolean;
}) {
  if (coverage === null) return <span className="muted">—</span>;
  const pct = Math.max(0, Math.min(coverage / SCALE, 1)) * 100;
  const low = coverage < 1;
  return (
    <div className={styles.coverage}>
      <div className="row" style={{ ["--gap" as string]: "6px" }}>
        <strong className="num">{coverageText(coverage)}</strong>
        {low ? (
          <Pill tone="warn" size="sm">
            below the largest payout
          </Pill>
        ) : (
          <span className="xsmall muted">the largest payout</span>
        )}
      </div>
      <span className={styles.bar} aria-hidden>
        <span className={`${styles.fill} ${low ? styles.fillLow : ""}`} style={{ width: `${pct}%` }} />
        <span className={styles.tick} style={{ left: `${100 / SCALE}%` }} />
      </span>
      {scale ? (
        <span className={styles.scale} aria-hidden>
          <span>0</span>
          <span>1×</span>
          <span>2×</span>
          <span>3×+</span>
        </span>
      ) : null}
      <span className="xsmall muted num">
        {amount(available)} available / {amount(largest)} {TOKEN.symbol} largest
      </span>
    </div>
  );
}
