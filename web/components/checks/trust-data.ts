/**
 * Pinned values the trust page shows, each with the repository file it comes from. They are fixed by
 * construction (a docker build's identity, recorded runs' receipts, a recorded T1 check), so they do not
 * change with the deployment; the live values (the verifier pin, code hashes, near-t2) are read at request
 * time from the API and checked from the browser against NEAR.
 */

/** The ZK program (T2): SP1's docker build of crates/zk/guest, which near-t2 pins (docs/zk.md). */
export const ZK = {
  sp1: "6.10.0",
  programVkey: "0x00e87aaad0ca36c285eec6bccab0cace523784873ad65e45b3b65e2759288203", // gitleaks:allow (a public program vkey, docs/zk.md)
  elfSha256: "7fea93b80cc71db7ccd2315acc9b85c583fd3164bc95c6e6b68b6119d1c39702",
  elfPath: "fixtures/zk/guest/antibody-zk-guest.elf",
  constants: "crates/cli/src/zk.rs",
  doc: "docs/zk.md",
  build: "cd crates/zk/guest && cargo prove build --docker --tag v6.10.0 --workspace-directory ../../.. --locked",
  hash: "sha256sum target/elf-compilation/docker/*/release/antibody-zk-guest",
} as const;

/** The verifier wasm's reproducible build (docs/trust.md: "The verifier build"). */
export const VERIFIER_BUILD = {
  make: "make verifier-wasm",
  docker: "docker build -f Dockerfile.verifier -t antibody-verifier-build . && docker run --rm antibody-verifier-build",
  script: "scripts/build/verifier-wasm.sh",
  dockerfile: "Dockerfile.verifier",
} as const;

/**
 * Agent A's run (33): what its TDX quote binds, as `antibody tee check` read it on 2026-11-06, against
 * OutLayer's on-chain collateral and against fresh Intel PCS collateral (both Verified, UpToDate).
 */
export const RUN_BINDING = {
  source: "docs/results/t1-collateral-2026-11-06.json",
  checkedAt: "2026-11-06T12:50:44Z",
  run: 33,
  commit: 13,
  program: 5,
  requestId: 8839,
  revealTx: "8yEJyYXgz34BJWCTPLVnvq13PLW15oBx8uzkMkopc4DY",
  wasmHash: "ccc27f0ca60c503c46f6f1b2024b54481709fdd0c8849fe8d9a8912c637b28c1",
  inputHash: "c348b67ec9a15c29e066fa0d3933944be77336a394dce17b70d95554480afd86",
  outputHash: "c0c46c1444c520cd9ce18c915309cbd070868f6386c3643b1bf8b42d43ee2c7f",
  caller: "agent-a.antibody.testnet",
  dcapVerifier: "tee-verifier-2026-07-22.testnet",
  dcapStatus: "UpToDate",
  imageIndex: 109,
  attestation: "https://testnet-api.outlayer.ai/attestations/by-request/8839",
} as const;

/**
 * The four disclosed testnet runs: the TEE's receipt (on NEAR, in each run's answer), which the native
 * re-run and SP1's executor reproduce byte for byte (fixtures/zk/<case>/expected.json).
 */
export const RECEIPTS = [
  {
    fixture: "fixtures/zk/a",
    run: 33,
    commit: 13,
    program: 5,
    who: "our fixture agent A",
    kind: "Violation",
    slots: [0],
    receipt: "0ea67c6a56b964c4a8ef66861c89d56e99e0428836316d727f9d3dcea3dca2ee",
  },
  {
    fixture: "fixtures/zk/c",
    run: 35,
    commit: 15,
    program: 5,
    who: "our fixture agent C",
    kind: "NoViolation",
    slots: [],
    receipt: "0326cfc9b4f8f727dec761c64a9f9ec0c0db46f9bf6c376526dbd3d8a744ba8d",
  },
  {
    fixture: "fixtures/zk/d",
    run: 36,
    commit: 16,
    program: 5,
    who: "our fixture agent D (a duplicate)",
    kind: "Violation",
    slots: [0],
    receipt: "fa62fbea3e9ea80fc38a8c5411b7041948e26add0c98efe501eafeec318d7c98",
  },
  {
    fixture: "fixtures/zk/practice",
    run: 53,
    commit: 20,
    program: 6,
    who: "a practice try",
    kind: "Violation",
    slots: [0],
    receipt: "e51759c61d379da5947f1d38291b0efa4bd0d90634a72d9411bd9eda4fd119df",
  },
] as const;

/**
 * The commands that reproduce run 33's receipt (docs/zk.md: Proving, Reproduce): natively from the chain
 * with its disclosed content key (no proof), and in SP1's executor from the recorded case.
 */
export const REPRODUCE = {
  native: 'antibody zk prove --run 33 --key "$(cat fixtures/zk/a/content_key.hex)" --tx 8yEJyYXgz34BJWCTPLVnvq13PLW15oBx8uzkMkopc4DY --mode none',
  zkvm: "crates/zk/prove/target/release/antibody-zk-prove --mode execute fixtures/zk/guest/antibody-zk-guest.elf fixtures/zk/a",
} as const;
