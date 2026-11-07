import type { IconName } from "@/components/ui/Icon";
import { Pill } from "@/components/ui/Pill";
import type { Tone } from "@/lib/format";
import { anchorConfirmed, KIND_TEXT, phaseOf, RECORDED_RUN, runGroup, type ProgramKind } from "./helpers";

const KIND_LOOK: Record<ProgramKind, { tone: Tone; icon: IconName | null }> = {
  fixture: { tone: "warn", icon: "alert" },
  practice: { tone: "info", icon: "play" },
  fork: { tone: "info", icon: "layers" },
  program: { tone: "neutral", icon: null },
};

/** What a program is: our deliberately vulnerable fixture, a practice tutorial, or practice in fork mode. */
export function ProgramKindBadge({ kind, long = false, size }: { kind: ProgramKind; long?: boolean; size?: "sm" | "md" }) {
  const t = KIND_TEXT[kind];
  const look = KIND_LOOK[kind];
  return (
    <Pill tone={look.tone} icon={look.icon} size={size} title={t.long}>
      {long ? t.long : t.short}
    </Pill>
  );
}

export function PhaseBadge({ phase, size }: { phase?: string | null; size?: "sm" | "md" }) {
  const p = phaseOf(phase);
  return (
    <Pill tone={p.tone} size={size} title={phase ? `phase ${phase}` : undefined}>
      {p.text}
    </Pill>
  );
}

/** rules.anchor.check: confirmed by NEAR's Ethereum light client at creation, or attested by the project. */
export function AnchorBadge({ check, size, long = false }: { check?: string | null; size?: "sm" | "md"; long?: boolean }) {
  if (!check) return <Pill size={size}>anchor not read</Pill>;
  if (anchorConfirmed(check))
    return (
      <Pill tone="ok" icon="shield" size={size} title="The anchor block's hash was confirmed by NEAR's Ethereum light client when the program was created">
        {long ? "confirmed by the light client" : "light-client confirmed"}
      </Pill>
    );
  return (
    <Pill tone="warn" size={size} title="The light client could not confirm this anchor: the project attests to it">
      project-attested
    </Pill>
  );
}

/** Which run a program belongs to: this run (recorded), an earlier run on the same deployment, or later. */
export function RunBadge({ id, size }: { id: number; size?: "sm" | "md" }) {
  const g = runGroup(id);
  if (g === "this-run")
    return (
      <Pill icon="clock" size={size} title={`From the run recorded in ${RECORDED_RUN.results_file}`}>
        recorded run, {RECORDED_RUN.date}
      </Pill>
    );
  if (g === "earlier")
    return (
      <Pill icon="clock" size={size} title="An earlier run on the same locked deployment">
        earlier run
      </Pill>
    );
  return null;
}
