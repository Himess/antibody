import { cap, severityTone, stateLabel } from "@/lib/format";
import { LEVEL, levelOf, type Level } from "@/lib/levels";
import { Pill } from "./Pill";
import styles from "./ui.module.css";

export { LEVEL, levelOf, type Level } from "@/lib/levels";

/**
 * The level's badge, in plain words (lib/levels.ts): "Enclave verdict", "Attested on NEAR", "Zero-knowledge proof", the
 * code in its tooltip. `code` adds the code beside it as small text (the trust page only).
 */
export function TierBadge({ tier, size, code = false }: { tier: string | null | undefined; size?: "sm" | "md"; code?: boolean }) {
  if (!tier) return <Pill size={size}>no verdict yet</Pill>;
  const level = levelOf(tier);
  const pill = (
    <Pill tone={level === "T2" ? "t2" : level === "T1" ? "t1" : "neutral"} icon={level === "T0" ? "clock" : undefined} size={size} title={LEVEL[level].title}>
      {LEVEL[level].name}
    </Pill>
  );
  if (!code) return pill;
  return (
    <span className={styles.level}>
      {pill}
      <LevelCode level={level} />
    </span>
  );
}

/** A level's code as small secondary text ("T1"), where the trust page maps names to the CLI's codes. */
export function LevelCode({ level }: { level: Level }) {
  return (
    <span className={styles.levelCode} title={LEVEL[level].title}>
      {level}
    </span>
  );
}

export function SeverityBadge({ severity, size }: { severity: string; size?: "sm" | "md" }) {
  return (
    <Pill tone={severityTone[severity] ?? "neutral"} icon={severity === "critical" ? "alert" : null} size={size}>
      {cap(severity)}
    </Pill>
  );
}

/** The state in plain words; the contract's own name for it ("Done:Paid") is in the tooltip. */
export function StateBadge({ state, size }: { state: string | null | undefined; size?: "sm" | "md" }) {
  const s = stateLabel(state);
  return (
    <Pill tone={s.tone} size={size} title={state ? `Contract state: ${state}` : undefined}>
      {s.text}
    </Pill>
  );
}

export function KindBadge({ kind, size }: { kind: string | null | undefined; size?: "sm" | "md" }) {
  if (!kind) return <Pill tone="pending" size={size}>no verdict yet</Pill>;
  const tone = kind === "Violation" ? "accent" : kind === "NoViolation" ? "neutral" : "warn";
  return (
    <Pill tone={tone} icon={kind === "Violation" ? "bolt" : null} size={size}>
      {kind === "NoViolation" ? "No violation" : kind}
    </Pill>
  );
}
