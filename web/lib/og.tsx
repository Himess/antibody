import { readFile } from "node:fs/promises";
import { join } from "node:path";

/** OG images (next/og): the site's own fonts (OFL, static WOFF), read from the repository at render time. */
export async function ogFonts() {
  const dir = join(process.cwd(), "assets/fonts");
  const [sans5, sans6, mono5, mono7] = await Promise.all([
    readFile(join(dir, "instrument-sans-latin-500-normal.woff")),
    readFile(join(dir, "instrument-sans-latin-600-normal.woff")),
    readFile(join(dir, "jetbrains-mono-latin-500-normal.woff")),
    readFile(join(dir, "jetbrains-mono-latin-700-normal.woff")),
  ]);
  return [
    { name: "Sans", data: sans5, weight: 500 as const, style: "normal" as const },
    { name: "Sans", data: sans6, weight: 600 as const, style: "normal" as const },
    { name: "Mono", data: mono5, weight: 500 as const, style: "normal" as const },
    { name: "Mono", data: mono7, weight: 700 as const, style: "normal" as const },
  ];
}

export const OG = {
  size: { width: 1200, height: 630 },
  bg: "#0a0b0c",
  ink: "#edeeeb",
  ink2: "#b4b9b5",
  ink3: "#8e948f",
  line: "#2a2e31",
  accent: "#ff6b2c",
  base: "#74a7ff",
  sepolia: "#4fd1c5",
};

/** The mark, for OG images: the stem in ink, the two binding sites in the accent. */
export function OgMark({ size = 44 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path d="M12 21.5V13.2M12 13.2L6.2 6.4M12 13.2l5.8-6.8" stroke={OG.ink} strokeWidth="2.4" strokeLinecap="round" />
      <circle cx="5" cy="5" r="2.6" fill={OG.accent} />
      <circle cx="19" cy="5" r="2.6" fill={OG.accent} />
    </svg>
  );
}
