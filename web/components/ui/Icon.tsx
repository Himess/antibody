/** Inline stroke icons (no icon font, no third-party request). Decorative unless `label` is given. */
import type { SVGProps } from "react";

const paths: Record<string, string> = {
  check: "M4 12.5l5 5L20 6.5",
  copy: "M9 9h10v10H9zM5 15V5h10",
  external: "M14 5h5v5M19 5l-8 8M18 14v5H5V6h5",
  lock: "M6 11h12v9H6zM8.5 11V8a3.5 3.5 0 017 0v3",
  seal: "M12 3l2.4 2.1 3.2-.3.7 3.1 2.7 1.7-1.3 2.9 1.3 2.9-2.7 1.7-.7 3.1-3.2-.3L12 21l-2.4-2.1-3.2.3-.7-3.1-2.7-1.7L4.3 12 3 9.1l2.7-1.7.7-3.1 3.2.3z",
  pause: "M9 6v12M15 6v12",
  play: "M8 5.5v13l11-6.5z",
  clock: "M12 7v5l3 2M21 12a9 9 0 11-18 0 9 9 0 0118 0z",
  alert: "M12 8v5M12 16.5v.5M10.3 4.2L2.8 17.5A2 2 0 004.5 20.5h15a2 2 0 001.7-3L13.7 4.2a2 2 0 00-3.4 0z",
  info: "M12 11v6M12 7.5v.5M21 12a9 9 0 11-18 0 9 9 0 0118 0z",
  arrow: "M5 12h14M13 6l6 6-6 6",
  arrowDown: "M12 5v14M6 13l6 6 6-6",
  chain: "M10 14a4 4 0 005.7 0l3-3a4 4 0 00-5.7-5.7l-1 1M14 10a4 4 0 00-5.7 0l-3 3a4 4 0 005.7 5.7l1-1",
  shield: "M12 3l7 3v6c0 4.5-3 7.6-7 9-4-1.4-7-4.5-7-9V6z",
  cpu: "M8 8h8v8H8zM10 3v3M14 3v3M10 18v3M14 18v3M3 10h3M3 14h3M18 10h3M18 14h3",
  sigma: "M17 5H7l6 7-6 7h10",
  x: "M6 6l12 12M18 6L6 18",
  dot: "M12 12h.01",
  sun: "M12 4V2M12 22v-2M4 12H2M22 12h-2M5.6 5.6L4.2 4.2M19.8 19.8l-1.4-1.4M5.6 18.4l-1.4 1.4M19.8 4.2l-1.4 1.4M16 12a4 4 0 11-8 0 4 4 0 018 0z",
  moon: "M20 14.5A8 8 0 019.5 4a8 8 0 1010.5 10.5z",
  menu: "M4 7h16M4 12h16M4 17h16",
  wallet: "M4 7h14a2 2 0 012 2v9a2 2 0 01-2 2H4zM4 7l11-3v3M16 13.5h.01",
  terminal: "M5 8l4 4-4 4M11 16h8",
  refresh: "M20 11a8 8 0 00-14.9-3M4 5v4h4M4 13a8 8 0 0014.9 3M20 19v-4h-4",
  eye: "M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12zM12 15a3 3 0 100-6 3 3 0 000 6z",
  eyeOff: "M3 3l18 18M10.6 6.1A9 9 0 0112 6c6 0 9.5 6 9.5 6a16 16 0 01-3 3.6M6.6 7.6C4 9.4 2.5 12 2.5 12S6 18 12 18c1.6 0 3-.4 4.2-1",
  key: "M15 9a3 3 0 11-6 0 3 3 0 016 0zM12 12v9M12 16h3M12 19h2",
  bolt: "M13 3L5 14h6l-1 7 8-11h-6z",
  layers: "M12 3l9 5-9 5-9-5zM3 13l9 5 9-5",
};

export type IconName = keyof typeof paths;

export function Icon({
  name,
  size = 16,
  label,
  strokeWidth = 1.7,
  ...rest
}: { name: IconName; size?: number; label?: string; strokeWidth?: number } & SVGProps<SVGSVGElement>) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      focusable="false"
      {...rest}
    >
      <path d={paths[name]} />
    </svg>
  );
}
