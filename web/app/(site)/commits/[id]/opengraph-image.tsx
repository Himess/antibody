import { ImageResponse } from "next/og";
import { api } from "@/lib/api";
import { stateLabel, wnear } from "@/lib/format";
import { OG, OgMark, ogFonts } from "@/lib/og";

export const alt = "A commit's verdict card on Antibody (NEAR testnet)";
export const size = OG.size;
export const contentType = "image/png";

/** A commit's card: its verdict as a stamp, its tier, its state and payout. */
export default async function Image({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const n = Number(id);
  const r = Number.isInteger(n) && n >= 0 ? await api.commit(n) : null;
  const c = r?.ok ? r.data : null;
  const kind = c?.proof_card.facts.kind ?? null;
  const tier = c?.proof_card.facts.tier ?? null;
  const state = c ? stateLabel(c.row.state).text : "Unknown commit";
  const slots = (c?.proof_card.facts.keys ?? []).map((k) => k.slot);
  const paid = c && c.row.paid_raw !== "0" ? wnear(c.row.paid_raw) : null;
  const stampColor = kind === "Violation" ? OG.accent : OG.ink2;
  return new ImageResponse(
    (
      <div style={{ width: "100%", height: "100%", display: "flex", flexDirection: "column", background: OG.bg, color: OG.ink, padding: "60px 80px", fontFamily: "Sans" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
          <OgMark size={40} />
          <span style={{ fontSize: 30, fontWeight: 600 }}>Antibody</span>
          <span style={{ marginLeft: "auto", fontFamily: "Mono", fontSize: 18, color: OG.ink3, letterSpacing: 1.5 }}>NEAR TESTNET</span>
        </div>
        <span style={{ marginTop: 44, fontFamily: "Mono", fontSize: 26, color: OG.ink2 }}>
          {c ? `Program ${c.commit.program} · commit ${c.commit.id} · round ${c.commit.round}` : `Commit ${id}`}
        </span>
        <div style={{ display: "flex", alignItems: "center", gap: 40, marginTop: 26 }}>
          <div style={{ display: "flex", flexDirection: "column", border: `4px solid ${stampColor}`, borderRadius: 8, padding: "18px 28px", color: stampColor }}>
            <span style={{ fontFamily: "Mono", fontWeight: 700, fontSize: 56, letterSpacing: 2 }}>{(kind === "NoViolation" ? "NO VIOLATION" : kind ?? "PENDING").toUpperCase()}</span>
            <span style={{ fontFamily: "Mono", fontSize: 22, letterSpacing: 2 }}>{slots.length ? `SLOT ${slots.join(", ")}` : "NO RULE BROKEN"}</span>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <span style={{ fontSize: 40, fontWeight: 600 }}>{state}</span>
            {paid ? <span style={{ fontSize: 30, color: OG.ink2 }}>{paid}</span> : null}
            <span style={{ fontFamily: "Mono", fontSize: 22, color: tier === "T1" ? "#5bbdea" : tier === "T2" ? "#b39bff" : OG.ink3 }}>
              {tier === "T1" ? "T1 · TDX quote verified on NEAR" : tier === "T2" ? "T2 · validity proof verified on NEAR" : tier ? "T0 · operator" : ""}
            </span>
          </div>
        </div>
        <div style={{ display: "flex", flexDirection: "column", marginTop: "auto", gap: 8 }}>
          {c?.proof_card.receipt_sha256 ? (
            <span style={{ fontFamily: "Mono", fontSize: 20, color: OG.ink3 }}>receipt {c.proof_card.receipt_sha256}</span>
          ) : null}
          <span style={{ fontSize: 20, color: OG.ink2 }}>The proof stays sealed until disclosure · Testnet: wNEAR has no value</span>
        </div>
      </div>
    ),
    { ...size, fonts: await ogFonts() },
  );
}
