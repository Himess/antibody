import Link from "next/link";
import type { Moment } from "@/lib/api/types";
import { sourceUrl } from "@/lib/config";
import { utc, wnear } from "@/lib/format";
import { Icon } from "../ui/Icon";
import { Pill } from "../ui/Pill";

/** What the panel shows, and where it comes from: recorded (a results file) or a live drill. */
export function MomentHeader({ moment, live, titleId, level = 2 }: { moment: Moment; live: boolean; titleId?: string; level?: 2 | 1 }) {
  const paused = moment.steps.filter((s) => s.chain.startsWith("evm:"));
  const lastPause = paused.length ? Math.max(...paused.map((s) => s.seconds)) : null;
  const H = level === 1 ? "h1" : "h2";
  return (
    <div className="stack" style={{ ["--gap" as string]: "10px" }}>
      <div className="row" style={{ ["--gap" as string]: "8px" }}>
        <p className="eyebrow">The one moment</p>
        <Pill tone={moment.kind === "drill" ? "ok" : "neutral"} icon={moment.kind === "drill" ? "bolt" : "clock"} size="sm">
          {moment.kind === "drill" ? "scheduled drill" : "recorded"}
        </Pill>
        {!live ? (
          <Pill tone="warn" size="sm" title="The API could not be reached; showing the bundled copy of the same results">
            offline copy
          </Pill>
        ) : null}
      </div>
      <H id={titleId}>
        A&apos;s Critical: paid in {moment.steps.find((s) => s.key === "paid")?.seconds.toFixed(1)} s, both chains paused by{" "}
        {lastPause?.toFixed(1)} s
      </H>
      <p className="small muted">
        Program {moment.program_id} (Acme Lend, our fixture), commit {moment.commit_id} by {moment.hunter ?? "agent A"}
        {moment.via ? ` over ${moment.via.replace(/^A \((.*)\)$/, "$1")}` : ""}, {utc(moment.commit.block_ts_ns, "ns")}. Paid{" "}
        {wnear(moment.paid_raw)}
        {moment.instant ? " instantly, under the cap" : ""}.{" "}
        <a href={sourceUrl(moment.results_file)} target="_blank" rel="noreferrer noopener">
          Results file <Icon name="external" size={11} style={{ display: "inline" }} />
        </a>{" "}
        · <Link href={`/commits/${moment.commit_id}`}>Commit {moment.commit_id}</Link>
      </p>
    </div>
  );
}
