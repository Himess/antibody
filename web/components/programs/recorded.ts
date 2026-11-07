/**
 * What the recorded run (docs/results/testnet-2026-11-04.json) shows and the API does not serve: the
 * post-release checks, copied exactly into data/release-checks.json, and the verdict guard's transactions
 * (JUDGES.md row 15; the results file's `verdict_consumer`). Shown as recorded, next to a live check.
 */
import extract from "@/data/release-checks.json";

export interface ReleaseCheck {
  chain_id: number;
  proxy?: string;
  address?: string;
  expected?: string;
  implementation?: string;
  ok?: boolean;
  rpc?: string;
  b_exploit_reverts?: boolean;
}

export interface RecordedPermit {
  permit_id: number;
  chain_id: number;
  round: number;
  kind: string;
  status: string;
  tx_hash?: string;
  explorer?: string;
}

interface ReleaseExtract {
  results_file: string;
  program_id: number;
  program_key: string;
  eip1967_implementation_slot: string;
  release: { ok: boolean; checks: ReleaseCheck[]; permits: RecordedPermit[] };
}

export const RELEASE = extract as unknown as ReleaseExtract;

/** guard.antibody.testnet, the verdict consumer: a DAO treasury the Acme project owns, watching program 5. */
export const GUARD = {
  results_file: "docs/results/testnet-2026-11-04.json",
  /** verdict_consumer.note: "added on 2026-11-06 … on the same run"; docs/verdict-guard.md: "after the story ran". */
  added: "2026-11-06",
  watched_program: 5,
  owner: "acme.antibody.testnet",
  code_hash: "7Wotb7nnbSnZnC8Dbx6iSCB785Dojai2HoVq7AqCbsdh",
  transfer_before_tx: "DpX3C6ajzA6JMxXzqG5Ew13aNDSppMDrrxKVoUtFPC1b",
  d_report_tx: "C63y1vnEr2ri8nsbYjg8sjoNUU1F3e2wj6PsWjtggPac",
  a_report_tx: "EgbrSvLD7Rfx8kxHEvsJHNDWx4U6usg7YznT29cu4psX",
  transfer_refused_tx: "79wqNDoQ7iLjVPkNKFCgTGGjkxDMSEu8j8BZwsC4MqsG",
  lock_tx: "EAHLthsR4SasEsxnUSG48PsVzfTH2zBzrQj1bx25RB4p",
} as const;
