import { ImageResponse } from "next/og";
import {
  cleanKeys,
  evData,
  hasWrapD,
  isPaidGap,
  keyState,
  neverRevealed,
  numOf,
  programInfo,
  promotedIn,
  ruleLabel,
  runOutcome,
  sealedLabel,
  sortedEvents,
  stateLabelOf,
  type KeyState,
} from "@/components/commit/model";
import { api } from "@/lib/api";
import { stateLabel, wnear } from "@/lib/format";
import { LEVEL, levelOf } from "@/lib/levels";
import { OG, ogFonts } from "@/lib/og";
import { OgLockup } from "@/lib/og-logo";

export const alt = "A commit's verdict card on Antibody (NEAR testnet)";
export const size = OG.size;
export const contentType = "image/png";

/** Who can open the proof, as the commit page's proof card says it. */
function sealWords(k: KeyState, wrapD: boolean | null, known: boolean, hunter: string, program: number): string {
  if (k.kind === "disclosed") return "Disclosed: anyone can open the proof and re-run it";
  if (k.kind === "published") return `Content key published in ${k.fixture}`;
  if (known && k.kind === "sealed") return "Sealed";
  return sealedLabel(k, wrapD, hunter, program);
}

/** A commit's card: its verdict as a stamp, its level, its state and payout, and who can open its proof, as its page says them. */
export default async function Image({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const n = Number(id);
  const r = Number.isInteger(n) && n >= 0 ? await api.commit(n) : null;
  const c = r?.ok ? r.data : null;
  const runId = numOf(c?.commit.run);
  const [pr, rr] = c ? await Promise.all([api.program(c.commit.program), runId !== null ? api.run(runId) : Promise.resolve(null)]) : [null, null];

  const events = c ? sortedEvents(c.events) : [];
  const label = c ? c.row?.state || stateLabelOf(c.commit.state) : "";
  const kind = c?.proof_card?.facts?.kind ?? c?.row?.verdict?.kind ?? null;
  const tier = c?.proof_card?.facts?.tier ?? c?.row?.verdict?.tier ?? null;
  const state = c ? stateLabel(label).text : "Unknown commit";
  const slots = cleanKeys(c?.proof_card?.facts?.keys ?? c?.row?.verdict?.keys).map((k) => k.slot);
  const paid = c?.row?.paid_raw && c.row.paid_raw !== "0" ? wnear(c.row.paid_raw) : null;
  const stampColor = kind === "Violation" ? OG.accent : OG.ink2;

  // A gap claim a round promoted and paid: named as the page names it ("gap claim · promoted at round 1"),
  // its round from the program's rounds when its events are not indexed.
  const promoted = events.find((e) => e.event === "gap_promoted");
  const gap = label === "Done:Paid" && (promoted !== undefined || isPaidGap(label, kind));
  const gapRound = numOf(evData(promoted).round) ?? (c && pr?.ok ? promotedIn(pr.data, c.commit.id) : null);
  // Expired unrevealed: the enclave never judged it, so there is no verdict to wait for.
  const unrevealed = !kind && c !== null && neverRevealed(label, c.commit.run);
  const sub = gap
    ? gapRound !== null
      ? `GAP CLAIM · PROMOTED AT ROUND ${gapRound}`
      : "GAP CLAIM · PROMOTED"
    : slots.length
      ? slots.map((s) => ruleLabel(pr?.ok ? pr.data : null, s)).join(", ").toUpperCase()
      : kind
        ? "NO RULE BROKEN"
        : unrevealed
          ? "NEVER JUDGED"
          : "NO VERDICT YET";

  let seal: string | null = null;
  if (c) {
    const disclosed = c.proof_card?.sealed_until_disclosure === false || events.some((e) => e.event === "disclosed");
    const info = programInfo(c.commit.program, pr?.ok ? pr.data : null);
    const keys = keyState({ disclosed, commitId: c.commit.id, runId, programId: c.commit.program, receipt: c.proof_card?.receipt_sha256, practice: info.practice });
    const outcome = rr?.ok ? runOutcome(rr.data.raw?.outcome) : null;
    const wrapD = outcome?.status === "answered" ? hasWrapD(outcome.a.extra) : null;
    seal = sealWords(keys, wrapD, label === "Done:Known", c.commit.hunter, c.commit.program);
  }

  return new ImageResponse(
    (
      <div style={{ width: "100%", height: "100%", display: "flex", flexDirection: "column", background: OG.bg, color: OG.ink, padding: "60px 80px", fontFamily: "Sans" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
          <OgLockup height={40} ink={OG.ink} accent={OG.accent} />
          <span style={{ marginLeft: "auto", fontFamily: "Mono", fontSize: 18, color: OG.ink3, letterSpacing: 1.5 }}>NEAR TESTNET</span>
        </div>
        <span style={{ marginTop: 44, fontFamily: "Mono", fontSize: 26, color: OG.ink2 }}>
          {c ? `Program ${c.commit.program} · commit ${c.commit.id} · round ${c.commit.round}` : `Commit ${id}`}
        </span>
        <div style={{ display: "flex", alignItems: "center", gap: 40, marginTop: 26 }}>
          <div style={{ display: "flex", flexDirection: "column", border: `4px solid ${stampColor}`, borderRadius: 8, padding: "18px 28px", color: stampColor }}>
            <span style={{ fontFamily: "Mono", fontWeight: 700, fontSize: 56, letterSpacing: 2 }}>{(kind === "NoViolation" ? "NO VIOLATION" : (kind ?? (unrevealed ? "NOT REVEALED" : "PENDING"))).toUpperCase()}</span>
            <span style={{ fontFamily: "Mono", fontSize: 22, letterSpacing: 2 }}>{sub}</span>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <span style={{ fontSize: 40, fontWeight: 600 }}>{state}</span>
            {paid ? <span style={{ fontSize: 30, color: OG.ink2 }}>{paid}</span> : null}
            <span style={{ fontFamily: "Mono", fontSize: 22, color: tier === "T1" ? "#5bbdea" : tier === "T2" ? "#b39bff" : OG.ink3 }}>
              {tier ? LEVEL[levelOf(tier)].name : ""}
            </span>
          </div>
        </div>
        <div style={{ display: "flex", flexDirection: "column", marginTop: "auto", gap: 8 }}>
          {c?.proof_card?.receipt_sha256 ? (
            <span style={{ fontFamily: "Mono", fontSize: 20, color: OG.ink3 }}>receipt {c.proof_card.receipt_sha256}</span>
          ) : null}
          {seal ? <span style={{ fontSize: 20, color: OG.ink2 }}>{seal}</span> : null}
          <span style={{ fontSize: 18, color: OG.ink3 }}>Testnet: wNEAR has no value</span>
        </div>
      </div>
    ),
    { ...size, fonts: await ogFonts() },
  );
}
