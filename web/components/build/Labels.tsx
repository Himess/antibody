import { Pill } from "@/components/ui/Pill";

/** Status words used down the build pages: each pairs an icon with words, never colour alone. */
export function NotYet({ children = "not yet" }: { children?: string }) {
  return <Pill tone="pending" size="sm">{children}</Pill>;
}

export function Done({ children = "done" }: { children?: string }) {
  return (
    <Pill tone="ok" size="sm">
      {children}
    </Pill>
  );
}

export function Recorded({ children = "recorded" }: { children?: string }) {
  return (
    <Pill tone="neutral" icon="clock" size="sm" title="From the recorded run's results file, not live">
      {children}
    </Pill>
  );
}

export function OfflineCopy() {
  return (
    <Pill tone="warn" size="sm" title="The API could not be reached; showing the bundled copy of the same results">
      offline copy
    </Pill>
  );
}
