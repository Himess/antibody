/**
 * The mark: an antibody is a Y-shaped protein whose two arms bind one exact target. Two binding sites in
 * the accent, the stem in ink.
 */
export function LogoMark({ size = 22, title }: { size?: number; title?: string }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
    >
      <path d="M12 21.5V13.2M12 13.2L6.2 6.4M12 13.2l5.8-6.8" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" />
      <circle cx="5" cy="5" r="2.6" fill="var(--accent)" />
      <circle cx="19" cy="5" r="2.6" fill="var(--accent)" />
    </svg>
  );
}

export function Wordmark() {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
      <LogoMark />
      <span style={{ fontWeight: 650, letterSpacing: "-0.02em", fontSize: "1.0625rem" }}>Antibody</span>
    </span>
  );
}
