import Link from "next/link";
import type { Moment } from "@/lib/api/types";
import { sourceUrl } from "@/lib/config";
import { utc, wnear } from "@/lib/format";
import { chainsWord, committer, pausesOf, possessive } from "@/lib/moment";
import { Icon } from "../ui/Icon";
import { Pill } from "../ui/Pill";

/** What the panel shows, and where it comes from: recorded (a results file) or a live drill. */
export function MomentHeader({ moment, live, titleId, level = 2 }: { moment: Moment; live: boolean; titleId?: string; level?: 2 | 1 }) {
  const paused = pausesOf(moment);
  const lastPause = paused.length ? Math.max(...paused.map((s) => s.seconds)) : null;
  const paid = moment.steps.find((s) => s.key === "paid");
  const { name, over } = committer(moment);
  const H = level === 1 ? "h1" : "h2";
  return (
    <div className="stack" style={{ ["--gap" as string]: "10px" }}>
      <div className="row" style={{ ["--gap" as string]: "8px" }}>
        <p className="eyebrow">The one moment</p>
        <Pill tone={moment.kind === "drill" ? "ok" : "neutral"} icon={moment.kind === "drill" ? "bolt" : "clock"} size="sm">
          {moment.kind === "drill" ? "automated drill" : "recorded"}
        </Pill>
        {!live ? (
          <Pill tone="warn" size="sm" title="The API could not be reached; showing the bundled copy of the same results">
            offline copy
          </Pill>
        ) : null}
      </div>
      <H id={titleId}>
        {possessive(name)} Critical: paid in {paid?.seconds.toFixed(1)} s
        {lastPause !== null ? `, ${chainsWord(paused.length)} paused by ${lastPause.toFixed(1)} s` : ""}
      </H>
      <p className="small muted">
        Program {moment.program_id} (Acme Lend, our fixture), commit {moment.commit_id} by {moment.hunter ?? `agent ${name}`}
        {over ? ` over ${over}` : ""}, {utc(moment.commit.block_ts_ns, "ns")}. Paid {wnear(moment.paid_raw)}
        {moment.instant ? " instantly (attested on NEAR, under the cap)" : ""}.{" "}
        <span style={{ whiteSpace: "nowrap" }}>
          <a href={sourceUrl(moment.results_file)} target="_blank" rel="noreferrer noopener">
            Results file <Icon name="external" size={11} style={{ display: "inline" }} />
          </a>{" "}
          · <Link href={`/commits/${moment.commit_id}`}>Commit {moment.commit_id}</Link>
        </span>
      </p>
    </div>
  );
}
