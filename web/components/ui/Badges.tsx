import { cap, severityTone, stateLabel } from "@/lib/format";
import { Pill } from "./Pill";

/**
 * Who vouches for a verdict. T0: OutLayer's operator delivered the TEE's output. T1: its TDX quote was
 * verified on NEAR (hardware). T2: a validity proof of the disclosed verdict, verified on NEAR (math).
 */
export function TierBadge({ tier, size }: { tier: string | null | undefined; size?: "sm" | "md" }) {
  if (!tier) return <Pill size={size}>no verdict yet</Pill>;
  if (tier === "T2")
    return (
      <Pill tone="t2" size={size} title="T2: a validity proof (SP1) of the disclosed verdict, verified on NEAR">
        T2 · math
      </Pill>
    );
  if (tier === "T1")
    return (
      <Pill tone="t1" size={size} title="T1: the run's Intel TDX quote, verified on NEAR">
        T1 · hardware
      </Pill>
    );
  return (
    <Pill tone="neutral" icon="clock" size={size} title="T0: OutLayer's operator delivered the TEE's output; T1 not yet">
      T0 · operator
    </Pill>
  );
}

export function SeverityBadge({ severity, size }: { severity: string; size?: "sm" | "md" }) {
  return (
    <Pill tone={severityTone[severity] ?? "neutral"} icon={severity === "critical" ? "alert" : null} size={size}>
      {cap(severity)}
    </Pill>
  );
}

export function StateBadge({ state, size }: { state: string | null | undefined; size?: "sm" | "md" }) {
  const s = stateLabel(state);
  return (
    <Pill tone={s.tone} size={size} title={state ?? undefined}>
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
