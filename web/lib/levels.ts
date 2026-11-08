/**
 * How far a verdict is checked, in plain words. The codes (T0, T1, T2) stay in tooltips and on /trust,
 * where they map to the CLI's output and JUDGES.md: T0, OutLayer's operator delivered the enclave's output;
 * T1, the run's Intel TDX quote was verified on NEAR; T2, a zero-knowledge validity proof was verified on NEAR.
 */
export type Level = "T0" | "T1" | "T2";

export const LEVEL: Record<Level, { name: string; prose: string; title: string }> = {
  T0: { name: "Verified in the enclave", prose: "verified in the enclave, not yet attested", title: "Level T0: OutLayer's operator delivered the enclave's output; not yet attested on NEAR" },
  T1: { name: "Attested on NEAR", prose: "attested on NEAR", title: "Level T1: the run's Intel TDX quote was verified on NEAR" },
  T2: { name: "Zero-knowledge proof", prose: "proven with a zero-knowledge proof", title: "Level T2: a zero-knowledge validity proof (SP1) of this verdict was verified on NEAR" },
};

/** The level a recorded tier names: anything that is not T1 or T2 reads as the enclave's result. */
export const levelOf = (tier: string): Level => (tier === "T2" ? "T2" : tier === "T1" ? "T1" : "T0");

/**
 * A sentence the API writes with the levels' codes ("no T1", "T1-confirmed", "the TEE's answer (T0)",
 * "t1-relayer"), in the site's words. Used where the site shows the API's own text: status alerts and the
 * recorded moment's steps. Anything else passes through unchanged.
 */
export function plainLevels(text: string): string {
  return text
    .replace(/\bthe TEE's answer \(T0\)/g, "the enclave's result")
    .replace(/\bT1-confirmed\b/g, "attested on NEAR")
    .replace(/\bgets no T1\b/g, "cannot be attested on NEAR")
    .replace(/\bno T1\b/g, "no attestation on NEAR")
    .replace(/\bt1-relayer\b/g, "attestation relayer")
    .replace(/\bt2-prover\b/g, "zero-knowledge prover")
    .replace(/\bTEE\b/g, "enclave");
}
