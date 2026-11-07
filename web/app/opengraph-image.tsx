import { ImageResponse } from "next/og";
import recorded from "@/data/moments.json";
import type { Moments } from "@/lib/api/types";
import { plainStep } from "@/lib/moment";
import { OG, OgMark, ogFonts } from "@/lib/og";

export const alt = "The one moment: A's Critical verified at 18.5 s, paid at 71.0 s, both chains paused by 90.6 s (recorded on NEAR testnet)";
export const size = OG.size;
export const contentType = "image/png";

/** The chart's frame: lane names in a gutter on the left, then seconds from the commit to the right edge. */
const W = 1040;
const GUTTER = 150;
const H = 290;
const LANES: { lane: string; name: string; y: number; color: string }[] = [
  { lane: "near", name: "NEAR TESTNET", y: 72, color: OG.ink },
  { lane: "evm:84532", name: "BASE SEPOLIA", y: 152, color: OG.base },
  { lane: "evm:11155111", name: "SEPOLIA", y: 220, color: OG.sepolia },
];

/** The landing's card: the one moment, from the recorded run (the same data as /v1/moments). */
export default async function Image() {
  const m = (recorded as unknown as Moments).moments[0];
  const steps = m.steps.filter((s) => !s.detail?.minor).map(plainStep);
  const end = Math.max(...steps.map((s) => s.seconds_exact));
  const span = Math.ceil((end + 4) / 10) * 10;
  const x = (s: number) => GUTTER + (s / span) * (W - GUTTER);
  const laneOf = (c: string) => LANES.find((l) => l.lane === c) ?? LANES[0];
  const paid = steps.find((s) => s.key === "paid");
  const signed = steps.find((s) => s.key === "permits_signed");
  const evm = steps.filter((s) => s.chain.startsWith("evm:"));
  const nearY = LANES[0].y;
  const nodes = [{ key: "commit", label: "Committed", seconds_exact: 0, chain: "near" }, ...steps];
  // Where each label goes, clear of the others and of the permit wires: NEAR labels alternate above and below their line,
  // the permits and the payment both sit above it (one ending at its node, one starting there), and on the chains the
  // later pause's label goes below its node.
  const lastPause = evm.length ? evm.reduce((a, b) => (b.seconds_exact > a.seconds_exact ? b : a)) : null;
  const place = (s: (typeof nodes)[number], i: number): { above: boolean; align: "center" | "start" | "end" } => {
    if (s.chain.startsWith("evm:")) return { above: s !== lastPause, align: "center" };
    if (s.key === "permits_signed") return { above: true, align: "end" };
    if (s.key === "paid") return { above: true, align: "start" };
    return { above: i % 2 === 0, align: "center" };
  };
  return new ImageResponse(
    (
      <div style={{ width: "100%", height: "100%", display: "flex", flexDirection: "column", background: OG.bg, color: OG.ink, padding: "54px 80px", fontFamily: "Sans" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
          <OgMark size={40} />
          <span style={{ fontSize: 30, fontWeight: 600, letterSpacing: -0.5 }}>Antibody</span>
          <span style={{ marginLeft: "auto", fontFamily: "Mono", fontSize: 18, color: OG.ink3, letterSpacing: 1.5 }}>RECORDED ON NEAR TESTNET</span>
        </div>
        <div style={{ display: "flex", flexDirection: "column", marginTop: 24 }}>
          <span style={{ fontSize: 52, fontWeight: 600, letterSpacing: -1.4, lineHeight: 1.05 }}>We don&apos;t read reports.</span>
          <span style={{ fontSize: 52, fontWeight: 600, letterSpacing: -1.4, lineHeight: 1.05, color: OG.accent }}>We run them.</span>
        </div>
        <div style={{ display: "flex", position: "relative", width: W, height: H, marginTop: 18 }}>
          {LANES.map((l) => (
            <div key={l.lane} style={{ position: "absolute", left: GUTTER, top: l.y - 1, width: W - GUTTER, height: 2, background: OG.line, display: "flex" }} />
          ))}
          {LANES.map((l) => (
            <span key={l.lane} style={{ position: "absolute", left: 0, top: l.y - 11, fontFamily: "Mono", fontSize: 16, lineHeight: 1.3, letterSpacing: 1, color: l.color }}>
              {l.name}
            </span>
          ))}
          <svg width={W} height={H} viewBox={`0 0 ${W} ${H}`} style={{ position: "absolute", left: 0, top: 0 }}>
            {signed
              ? evm.map((s) => {
                  const x1 = x(signed.seconds_exact);
                  const x2 = x(s.seconds_exact);
                  const y2 = laneOf(s.chain).y;
                  return (
                    <g key={s.key}>
                      <path d={`M${x1},${y2} L${x2},${y2}`} stroke={laneOf(s.chain).color} strokeWidth={3} fill="none" />
                      <path d={`M${x1},${nearY + 12} C${x1},${(nearY + y2) / 2 + 10} ${x1 + (x2 - x1) * 0.3},${y2} ${x2},${y2}`} stroke={OG.accent} strokeWidth={3} fill="none" />
                    </g>
                  );
                })
              : null}
          </svg>
          <div style={{ position: "absolute", left: x(0), top: nearY - 2, width: x(paid?.seconds_exact ?? end) - x(0), height: 4, background: OG.ink, display: "flex" }} />
          {nodes.map((s, i) => {
            const lane = laneOf(s.chain);
            const isEvm = s.chain.startsWith("evm:");
            const { above, align } = place(s, i);
            return (
              <div key={s.key} style={{ position: "absolute", left: x(s.seconds_exact) - 12, top: lane.y - 12, display: "flex", flexDirection: "column", alignItems: "center" }}>
                <div style={{ width: 24, height: 24, borderRadius: 12, background: isEvm ? OG.accent : lane.color, display: "flex" }} />
                <div
                  style={{
                    position: "absolute",
                    top: above ? -60 : 30,
                    width: 200,
                    left: align === "center" ? -88 : align === "end" ? -176 : 0,
                    display: "flex",
                    flexDirection: "column",
                    alignItems: align === "center" ? "center" : align === "end" ? "flex-end" : "flex-start",
                  }}
                >
                  <span style={{ fontFamily: "Mono", fontWeight: 700, fontSize: 22, lineHeight: 1.2 }}>{s.seconds_exact.toFixed(1)} s</span>
                  <span style={{ fontSize: 16, lineHeight: 1.25, color: OG.ink2 }}>{s.label}</span>
                </div>
              </div>
            );
          })}
        </div>
        <div style={{ display: "flex", marginTop: "auto", gap: 26, fontSize: 20, color: OG.ink2 }}>
          <span>Every node is a transaction</span>
          <span style={{ color: OG.ink3 }}>·</span>
          <span>Seconds from A&apos;s commit block</span>
          <span style={{ color: OG.ink3 }}>·</span>
          <span>Testnet: wNEAR has no value</span>
        </div>
      </div>
    ),
    { ...size, fonts: await ogFonts() },
  );
}
