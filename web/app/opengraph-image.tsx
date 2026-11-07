import { ImageResponse } from "next/og";
import recorded from "@/data/moments.json";
import type { Moments } from "@/lib/api/types";
import { OG, OgMark, ogFonts } from "@/lib/og";

export const alt = "The one moment: A's Critical verified at 18.5 s, paid at 71.0 s, both chains paused by 90.6 s (recorded on NEAR testnet)";
export const size = OG.size;
export const contentType = "image/png";

/** The landing's card: the one moment, from the recorded run (the same data as /v1/moments). */
export default async function Image() {
  const m = (recorded as unknown as Moments).moments[0];
  const steps = m.steps.filter((s) => !s.detail?.minor);
  const end = Math.max(...steps.map((s) => s.seconds_exact));
  const span = Math.ceil((end + 4) / 10) * 10;
  const W = 1040;
  const x = (s: number) => (s / span) * W;
  const laneY: Record<string, number> = { near: 40, "evm:84532": 150, "evm:11155111": 230 };
  const color = (c: string) => (c === "near" ? OG.ink : c === "evm:84532" ? OG.base : OG.sepolia);
  const paid = steps.find((s) => s.key === "paid");
  return new ImageResponse(
    (
      <div style={{ width: "100%", height: "100%", display: "flex", flexDirection: "column", background: OG.bg, color: OG.ink, padding: "54px 80px", fontFamily: "Sans" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
          <OgMark size={40} />
          <span style={{ fontSize: 30, fontWeight: 600, letterSpacing: -0.5 }}>Antibody</span>
          <span style={{ marginLeft: "auto", fontFamily: "Mono", fontSize: 18, color: OG.ink3, letterSpacing: 1.5 }}>RECORDED ON NEAR TESTNET</span>
        </div>
        <div style={{ display: "flex", flexDirection: "column", marginTop: 34 }}>
          <span style={{ fontSize: 58, fontWeight: 600, letterSpacing: -1.6, lineHeight: 1.05 }}>We don&apos;t read reports.</span>
          <span style={{ fontSize: 58, fontWeight: 600, letterSpacing: -1.6, lineHeight: 1.05, color: OG.accent }}>We run them.</span>
        </div>
        <div style={{ display: "flex", position: "relative", width: W, height: 270, marginTop: 30 }}>
          {Object.entries(laneY).map(([lane, y]) => (
            <div key={lane} style={{ position: "absolute", left: 0, top: y, width: W, height: 2, background: OG.line, display: "flex" }} />
          ))}
          <span style={{ position: "absolute", left: 0, top: 158, fontFamily: "Mono", fontSize: 16, color: OG.base }}>BASE SEPOLIA</span>
          <span style={{ position: "absolute", left: 0, top: 238, fontFamily: "Mono", fontSize: 16, color: OG.sepolia }}>SEPOLIA</span>
          <div style={{ position: "absolute", left: 0, top: 40 - 1, width: x(paid?.seconds_exact ?? end), height: 4, background: OG.ink, display: "flex" }} />
          {[0, ...steps.map((s) => s.seconds_exact)].map((s, i) => {
            const step = i === 0 ? null : steps[i - 1];
            const lane = step?.chain ?? "near";
            const y = laneY[lane] ?? 40;
            const evm = lane.startsWith("evm:");
            const label = step ? step.label : "Committed";
            const above = lane === "near" ? i % 2 === 0 : true;
            return (
              <div key={i} style={{ position: "absolute", left: x(s) - 12, top: y - 12, display: "flex", flexDirection: "column", alignItems: "center" }}>
                <div style={{ width: 24, height: 24, borderRadius: 12, background: evm ? OG.accent : color(lane), display: "flex" }} />
                <div
                  style={{
                    position: "absolute",
                    top: above ? -64 : 30,
                    width: 200,
                    left: -88,
                    display: "flex",
                    flexDirection: "column",
                    alignItems: "center",
                  }}
                >
                  <span style={{ fontFamily: "Mono", fontWeight: 700, fontSize: 24 }}>{s === 0 ? "0" : s.toFixed(1)} s</span>
                  <span style={{ fontSize: 17, color: OG.ink2 }}>{label}</span>
                </div>
              </div>
            );
          })}
        </div>
        <div style={{ display: "flex", marginTop: "auto", gap: 26, fontSize: 20, color: OG.ink2 }}>
          <span>Every node is a transaction</span>
          <span style={{ color: OG.ink3 }}>·</span>
          <span>NEAR testnet, Base Sepolia, Sepolia</span>
          <span style={{ color: OG.ink3 }}>·</span>
          <span>Testnet: wNEAR has no value</span>
        </div>
      </div>
    ),
    { ...size, fonts: await ogFonts() },
  );
}
