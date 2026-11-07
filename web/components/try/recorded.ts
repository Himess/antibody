/**
 * The two recorded practice tries of the 2026-11-04 testnet run (docs/results/testnet-2026-11-04.json,
 * `practice.tries`; JUDGES.md claim 12): what a try looks like when the page cannot run one now. These are
 * recorded, not live; every hash is a NEAR testnet transaction anyone can open.
 */
export const RECORDED_FILE = "docs/results/testnet-2026-11-04.json";

export interface RecordedTry {
  account: string;
  commitId: number;
  runId: number;
  vault: number;
  /** Seconds from the vend to the payout (the CLI's wall clock). */
  seconds: number;
  state: string;
  paidRaw: string;
  txs: { step: string; hash: string; signer: string }[];
}

export const RECORDED_TRIES: RecordedTry[] = [
  {
    account: "judge-1-p6.practice.antibody.testnet",
    commitId: 20,
    runId: 53,
    vault: 4,
    seconds: 43.650731623,
    state: "Done:Paid",
    paidRaw: "1000000000000000000000",
    txs: [
      { step: "vend", hash: "2wUoCN17qhVnTMxFRWdwjfQJvizo4YS81DpkdurFpEC8", signer: "practice.antibody.testnet" },
      { step: "commit", hash: "L9wQ6bJiSore1NN2TzsnPDbLcfevZt7K5xt6AqnWtmc", signer: "judge-1-p6.practice.antibody.testnet" },
      { step: "reveal", hash: "7KduoN6WrLrkPKK6hc1NAWKMmeJqfac9MBt5oi7Ma2DH", signer: "judge-1-p6.practice.antibody.testnet" },
      { step: "allocate", hash: "2qYXUQQVTqDg2zuyhnokZKmjLQr2M3JpAdatBPspkAKi", signer: "judge-1-p6.practice.antibody.testnet" },
      { step: "pay", hash: "rbhQjh9XTWSwPmMQNL8PM6kmWRBoiuHmGSmCMspWGS7", signer: "judge-1-p6.practice.antibody.testnet" },
      { step: "close", hash: "8iVfV6ijpQzW6jRUjZCxFPs844EJaDoqjEtQtf3ugT2X", signer: "judge-1-p6.practice.antibody.testnet" },
      { step: "reclaim", hash: "4C4sh1ahZoToWt1yG92VL48xXdvyX7toYrELYVxER2Xw", signer: "practice.antibody.testnet" },
    ],
  },
  {
    account: "judge-2-p6.practice.antibody.testnet",
    commitId: 21,
    runId: 54,
    vault: 5,
    seconds: 39.860913426,
    state: "Done:Paid",
    paidRaw: "1000000000000000000000",
    txs: [
      { step: "vend", hash: "DsRk2yfeyigpc2RScLzWiWoRBXYkuDiNPtKPMnC8aWtg", signer: "practice.antibody.testnet" },
      { step: "commit", hash: "CvmTX5T5tqbtEbvzsv4MWzyyUvCnFRKWtSY19oewng1V", signer: "judge-2-p6.practice.antibody.testnet" },
      { step: "reveal", hash: "6CqW7dK4Fhzoni3U91BrKrxqR5qBggStQKWYfMKRUovG", signer: "judge-2-p6.practice.antibody.testnet" },
      { step: "allocate", hash: "9qSUr8ovRsaCg7EtnvN2y9pN5pFuBjeBPoVQAVy5Ax8L", signer: "judge-2-p6.practice.antibody.testnet" },
      { step: "pay", hash: "6ienbsLuHNPKmfCuD4aPBLnWqfWFY32nqnwWEpPscn8E", signer: "judge-2-p6.practice.antibody.testnet" },
      { step: "close", hash: "DcL7XZxtZovpPwZNM2PBccKYVtLECRvd8mSyf3HCktqQ", signer: "judge-2-p6.practice.antibody.testnet" },
      { step: "reclaim", hash: "4hFYXmoBofbSZRpZg8rvwqq4pNZhXXj9B41Q88ZD6YMo", signer: "practice.antibody.testnet" },
    ],
  },
];
