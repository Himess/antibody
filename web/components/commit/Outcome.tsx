import Link from "next/link";
import type { ReactNode } from "react";
import { StateBadge } from "@/components/ui/Badges";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { Pill } from "@/components/ui/Pill";
import { Empty, ErrorState } from "@/components/ui/States";
import { Hash } from "@/components/ui/Hash";
import { Time } from "@/components/ui/Time";
import { TxLink } from "@/components/ui/TxLink";
import { Sealed } from "@/components/ui/Verdict";
import type { CommitDetail, FeedRow, IndexedEvent, ProgramDetail, Result, VerdictKey } from "@/lib/api";
import { sourceUrl } from "@/lib/config";
import { seconds, wnear } from "@/lib/format";
import {
  bondFate,
  cleanKeys,
  earlierClaim,
  evData,
  isObj,
  keyFile,
  knownMatch,
  modeOfEvent,
  numOf,
  publishedFixture,
  recordedNote,
  runOfEvent,
  slotInfo,
  slotName,
  stateData,
  stateExpect,
  strOf,
  usd,
  who,
  type KeyState,
} from "./model";
import { TagMatch } from "./TagMatch";
import styles from "./commit.module.css";

export interface OutcomeInput {
  c: CommitDetail;
  label: string;
  keys: VerdictKey[];
  kind: string | null;
  program: ProgramDetail | null;
  programEvents: IndexedEvent[];
  /** The commit's events, in seq order. */
  events: IndexedEvent[];
  rows: FeedRow[] | null;
  feedError: string | null;
  /** The earlier claim's own record, for a duplicate. */
  earlier: Result<CommitDetail> | null;
  contract: string;
  /** Who can open this commit's proof besides the TEE (the proof card's state). */
  keyState: KeyState;
}

const ev = (events: IndexedEvent[], name: string) => events.find((e) => e.event === name);
const byWho = (account: string | null | undefined, program: number) => {
  const w = who(account, program);
  return w ? `${w} (${account})` : (account ?? "—");
};

function Evidence({ eyebrow, title, lede, children }: { eyebrow: string; title: ReactNode; lede?: ReactNode; children?: ReactNode }) {
  return (
    <section className={`card ${styles.evidence}`}>
      <header className={styles.evidenceHead}>
        <p className="eyebrow">{eyebrow}</p>
        <h3>{title}</h3>
        {lede ? <p className="small muted">{lede}</p> : null}
      </header>
      {children}
    </section>
  );
}

// ------------------------------------------------------------------ bond and payout

function Bond({ x }: { x: OutcomeInput }) {
  const fate = bondFate(x.label);
  const bond = x.c.commit.bond;
  const id = x.c.commit.program;
  const at =
    ev(x.events, "forfeited") ??
    ev(x.events, "expired") ??
    ev(x.events, "holder") ??
    ev(x.events, "duplicate") ??
    ev(x.events, "known_match") ??
    ev(x.events, "gap_claim");
  const words: Record<string, string> = {
    "Done:Forfeited": `The proof broke no rule, so the bond is forfeited to program ${id}'s escrow, where it pays future findings. The protocol never earns from forfeits.`,
    "Done:Expired": `The proof was never revealed in time, so the bond is forfeited to program ${id}'s escrow.`,
    Duplicate: "A duplicate is never paid, and its bond is returned.",
    "Done:DuplicateConfirmed": "A duplicate is never paid, and its bond is returned.",
    "Done:Known": "A known match is never paid, and its bond is returned.",
    GapClaim: "Returned when the gap claim was recorded.",
    "Done:GapOpen": "Returned when the gap claim was recorded.",
  };
  const refunded = x.label === "Done:Refunded" ? strOf(stateData(x.c.commit.state)?.reason) : null;
  const forfeits = fate === "forfeited" ? strOf(x.program?.program?.ledger?.forfeits) : null;
  const text =
    words[x.label] ??
    (fate === "held"
      ? "Locked until the verdict is final: refunded if the proof holds a key, is a duplicate or a known issue; forfeited to the escrow if it breaks nothing."
      : fate === "returned"
        ? x.events.some((e) => e.event === "gap_claim")
          ? "Returned when the gap claim was recorded."
          : refunded
            ? `Refunded: ${refunded}.`
            : "Returned at allocation: a proof that holds a key gets its bond back before it is paid."
        : "—");
  return (
    <div className={`card ${styles.outcomeCard}`}>
      <p className="eyebrow">Bond</p>
      <p className={`${styles.figure} num`}>{bond ? wnear(bond) : "—"}</p>
      <div className="row" style={{ ["--gap" as string]: "8px" }}>
        {fate === "returned" ? (
          <Pill tone="ok">returned</Pill>
        ) : fate === "forfeited" ? (
          <Pill tone="warn">forfeited to the escrow</Pill>
        ) : fate === "held" ? (
          <Pill tone="pending">locked</Pill>
        ) : (
          <Pill>unknown</Pill>
        )}
      </div>
      <p className="small muted">{text}</p>
      {forfeits ? (
        <p className="small muted">
          Program {id} has received <span className="num">{wnear(forfeits)}</span> in forfeited bonds, in all.
        </p>
      ) : null}
      {at && fate !== "held" ? (
        <p className="small">
          In <TxLink hash={at.tx} href={at.explorer} />
        </p>
      ) : null}
    </div>
  );
}

function Payout({ x }: { x: OutcomeInput }) {
  const d = stateData(x.c.commit.state);
  const paidEv = ev(x.events, "paid");
  const promoted = ev(x.events, "gap_promoted") ?? ev(x.events, "promoted");
  const challenge = numOf(x.program?.program?.params?.windows?.challenge);
  const cap = x.program?.rules?.instant?.cap_raw;
  let figure = "0 wNEAR";
  let pill: ReactNode = <Pill>not paid</Pill>;
  let text = "";
  switch (x.label) {
    case "Done:Paid": {
      const amount = strOf(d?.amount) ?? strOf(evData(paidEv).amount);
      const instant = d?.instant === true || evData(paidEv).instant === true;
      figure = amount ? wnear(amount) : "—";
      pill = <Pill tone="ok">{instant ? "paid · instant" : "paid · after the window"}</Pill>;
      text = instant
        ? `Paid in seconds: the verdict is T1, at most the instant cap${cap ? ` (${wnear(cap)})` : ""}, within its window's instant budget.`
        : promoted
          ? `Promoted at round ${numOf(evData(promoted).round) ?? "—"} and paid at its original priority.`
          : `Paid once its challenge window${challenge ? ` (${seconds(challenge, 0)})` : ""} passed with no challenge open.`;
      break;
    }
    case "Holder":
      figure = wnear(strOf(d?.amount) ?? "0");
      pill = <Pill tone="info">reserved · awaiting payment</Pill>;
      text = "Reserved from the escrow at allocation; the keeper pays funded holders on its next pass, and an unfunded remainder is queued as debt in commit order.";
      break;
    case "Promoted":
      figure = wnear(strOf(d?.amount) ?? "0");
      pill = <Pill tone="info">promoted · awaiting payment</Pill>;
      text = `Promoted at round ${numOf(d?.round) ?? "—"}: paid at its original priority once funded.`;
      break;
    case "Duplicate":
      pill = <Pill>never paid</Pill>;
      text = `${strOf(d?.reserved) ? `${wnear(String(d?.reserved))} stays reserved: ` : ""}if a later round's fix stops the earlier claim but not this proof, it proves a distinct bug and is promoted, paid at its original priority.`;
      break;
    case "GapClaim":
      figure = wnear(strOf(d?.reserved) ?? strOf(d?.amount) ?? "0");
      pill = <Pill tone="info">reserved</Pill>;
      text = "Reserved for the gap claim: paid only if a later round adds a rule this proof breaks.";
      break;
    case "Done:DuplicateConfirmed":
      text = "Never paid: the next round confirmed it a duplicate and released its reservation.";
      break;
    case "Done:Known":
      text = "Never paid: the project declared this issue, sealed, before the program opened.";
      break;
    case "Done:Forfeited":
      text = "Nothing is owed for a proof that breaks no rule.";
      break;
    case "Done:Expired":
      text = "Never revealed, never judged.";
      break;
    case "Done:Refunded":
      text = "Refunded and recorded; nothing paid.";
      break;
    case "Done:GapOpen":
      text = "A gap claim no round promoted: nothing paid.";
      break;
    case "Done:Exhausted":
      pill = <Pill>budget spent</Pill>;
      text = "It holds its keys, but the rule's budget for the round was already spent: paid nothing.";
      break;
    case "Done:Defaulted":
      pill = <Pill tone="warn">defaulted</Pill>;
      text = `A valid claim the escrow never covered: ${wnear(strOf(d?.owed) ?? "0")} owed at the program's exit.`;
      break;
    default:
      figure = "—";
      pill = <Pill tone="pending">no verdict yet</Pill>;
      text = "A payout follows only a final Violation that holds a key.";
  }
  return (
    <div className={`card ${styles.outcomeCard}`}>
      <p className="eyebrow">Payout</p>
      <p className={`${styles.figure} num`}>{figure}</p>
      <div className="row" style={{ ["--gap" as string]: "8px" }}>
        {pill}
      </div>
      <p className="small muted">{text}</p>
      {paidEv ? (
        <p className="small">
          To {byWho(strOf(evData(paidEv).beneficiary) ?? x.c.commit.beneficiary, x.c.commit.program)}, in <TxLink hash={paidEv.tx} href={paidEv.explorer} />{" "}
          <span className={`xsmall muted ${styles.nowrap}`}>
            <Time t={paidEv.block_ts_ns} unit="ns" showRelative={false} />
          </span>
        </p>
      ) : null}
    </div>
  );
}

// ------------------------------------------------------------------ the outcomes that need evidence

function DuplicateEvidence({ x }: { x: OutcomeInput }) {
  const dup = ev(x.events, "duplicate");
  const of = numOf(evData(dup).of) ?? numOf(stateData(x.c.commit.state)?.of);
  const claim = x.rows ? earlierClaim(x.rows, x.c.commit.id, x.keys, of) : null;
  const title = "The earlier claim";
  if (!claim) {
    return (
      <Evidence eyebrow="A duplicate is never paid" title={title}>
        {x.feedError ? (
          <ErrorState what="the program's feed (where the earlier claim is found)" error={x.feedError} partial />
        ) : (
          <Empty>
            {of !== null ? `near-antibody names commit ${of} as the earlier claim, but it is not in the program's feed yet.` : "No earlier claim with an equal key is in the program's feed yet."}
          </Empty>
        )}
      </Evidence>
    );
  }
  const r = claim.row;
  const e = x.earlier?.ok ? x.earlier.data : null;
  const before = typeof r.committed_at === "number" && typeof x.c.commit.committed_at === "number" ? x.c.commit.committed_at - r.committed_at : null;
  const theirSealed = e ? e.proof_card?.sealed_until_disclosure !== false : true;
  const theirKey = publishedFixture({ commitId: r.commit_id, runId: r.run_id, programId: x.c.commit.program, receipt: e?.proof_card?.receipt_sha256 });
  const myKey = x.keyState.kind === "published" ? x.keyState.fixture : null;
  const m = claim.matches[0];
  return (
    <Evidence
      eyebrow="A duplicate is never paid"
      title={
        <>
          {title}: <Link href={`/commits/${r.commit_id}`}>commit {r.commit_id}</Link>
        </>
      }
      lede={
        <>
          By {byWho(r.hunter, x.c.commit.program)}, committed <Time t={r.committed_at} showRelative={false} />
          {before !== null && before > 0 ? ` (${seconds(before)} before this one)` : ""}. Commit order is priority: the first proof of a key is the one paid.
        </>
      }
    >
      <div className="row" style={{ ["--gap" as string]: "8px" }}>
        {r.paid_raw && r.paid_raw !== "0" ? <Pill tone="ok">paid {wnear(r.paid_raw)}</Pill> : <StateBadge state={r.state} />}
        {dup ? (
          <span className="small">
            near-antibody recorded it as the earlier claim in <TxLink hash={dup.tx} href={dup.explorer} />
          </span>
        ) : null}
      </div>
      {claim.matches.length ? (
        claim.matches.map((mm) => (
          <TagMatch
            key={`${mm.mine.slot}-${mm.mine.tag}`}
            label={`Commit ${x.c.commit.id}'s key against commit ${r.commit_id}'s key`}
            left={{ title: `This commit (${x.c.commit.id})`, slot: mm.mine.slot, slotLabel: slotLabel(x.program, mm.mine.slot), tag: mm.mine.tag }}
            right={{ title: `The earlier claim (commit ${r.commit_id})`, slot: mm.theirs.slot, slotLabel: slotLabel(x.program, mm.theirs.slot), tag: mm.theirs.tag }}
          />
        ))
      ) : (
        <Empty>The feed shows no key in common between these two commits.</Empty>
      )}
      <p className="small muted">A tag is a PRF of the rule and the entry point under the program&apos;s key, which only the TEE holds.</p>
      <div className={styles.pair}>
        <div className={styles.sealedWrap}>
          <p className="small">
            <strong>The earlier claim&apos;s proof</strong>
          </p>
          {theirSealed ? (
            <Sealed
              seed={e?.commit.blob_sha256 ?? e?.commit.commitment ?? m?.theirs.tag ?? "0"}
              label={theirKey ? "Sealed · key published" : "Sealed until disclosure"}
              lines={3}
            />
          ) : (
            <div className="callout callout-ok">Disclosed: anyone can open it and re-run it.</div>
          )}
          {e ? (
            <dl className={`kv small ${styles.facts}`}>
              <dt>Receipt</dt>
              <dd>
                <Hash value={e.proof_card?.receipt_sha256} head={12} label="the earlier claim's receipt sha256" />
              </dd>
              <dt>Blob sha256</dt>
              <dd>
                <Hash value={e.commit.blob_sha256} head={12} label="the earlier claim's blob sha256" />
              </dd>
            </dl>
          ) : x.earlier && !x.earlier.ok ? (
            <ErrorState what={`commit ${r.commit_id}`} error={x.earlier.error} partial />
          ) : null}
          {theirSealed && theirKey ? <KeyNote fixture={theirKey} /> : null}
        </div>
        <div className={styles.sealedWrap}>
          <p className="small">
            <strong>This proof</strong>
          </p>
          <Sealed seed={x.c.commit.blob_sha256 ?? x.c.commit.commitment ?? "0"} label={myKey ? "Sealed · key published" : "Sealed until disclosure"} lines={3} />
          <dl className={`kv small ${styles.facts}`}>
            <dt>Receipt</dt>
            <dd>
              <Hash value={x.c.proof_card?.receipt_sha256} head={12} label="this proof's receipt sha256" />
            </dd>
            <dt>Blob sha256</dt>
            <dd>
              <Hash value={x.c.commit.blob_sha256} head={12} label="this proof's blob sha256" />
            </dd>
          </dl>
          <p className="xsmall muted">Kept as a proof record: its keys, its receipt and its transactions. At disclosure it opens with the claim it duplicates.</p>
          {myKey ? <KeyNote fixture={myKey} /> : null}
        </div>
      </div>
      {m && x.c.commit.run !== null && x.c.commit.run !== undefined && r.run_id !== null ? (
        <div className={styles.checks}>
          <p className={styles.checksTitle}>Check both tags on NEAR</p>
          <LiveCheck
            check={{ kind: "near-view", contract: x.contract, method: "get_run", args: { run_id: x.c.commit.run } }}
            expect={{ path: `outcome.Answered.facts.keys.${m.mineIndex}.tag`, equals: m.mine.tag, says: `this commit's run ${x.c.commit.run} records tag ${m.mine.tag.slice(0, 12)}…` }}
            label={`Read run ${x.c.commit.run}`}
          />
          <LiveCheck
            check={{ kind: "near-view", contract: x.contract, method: "get_run", args: { run_id: r.run_id } }}
            expect={{ path: `outcome.Answered.facts.keys.${m.theirsIndex}.tag`, equals: m.theirs.tag, says: `the earlier claim's run ${r.run_id} records the same tag` }}
            label={`Read run ${r.run_id}`}
          />
        </div>
      ) : null}
    </Evidence>
  );
}

function slotLabel(p: ProgramDetail | null, slot: number) {
  const i = slotInfo(p, slot);
  return `${slotName(p, slot)}${i.severity ? ` · ${i.severity}` : ""}`;
}

/** A proof whose content key our fixture agent published for the T2 study (docs/zk.md). */
function KeyNote({ fixture }: { fixture: string }) {
  return (
    <p className="xsmall muted">
      Not disclosed on chain; its content key is published for the T2 study (
      <a href={sourceUrl(keyFile(fixture))} target="_blank" rel="noreferrer noopener">
        <code>{keyFile(fixture)}</code>
      </a>
      ).
    </p>
  );
}

function KnownEvidence({ x }: { x: OutcomeInput }) {
  const km = knownMatch(x.program, x.keys);
  const title = "The known issue";
  if (!km) {
    return (
      <Evidence eyebrow="A known issue is never paid again" title={title}>
        {x.program ? <Empty>No known issue of this program carries this commit&apos;s tag in the program&apos;s record.</Empty> : <ErrorState what="the program's known issues" partial />}
      </Evidence>
    );
  }
  const declared = x.programEvents.find((e) => e.event === "known_issue" && runOfEvent(e) === km.issue.run);
  const opened = x.programEvents.find((e) => e.event === "program_opened");
  const declaredList = x.program?.program?.known;
  const total = Array.isArray(declaredList) ? declaredList.length : 1;
  return (
    <Evidence
      eyebrow="A known issue is never paid again"
      title={`${title}: ${km.index + 1} of ${total}, declared sealed before the program opened`}
      lede={`The project proved it itself, sealed (run ${km.issue.run}${km.issue.severity ? `, ${km.issue.severity}` : ""}), before opening; only the count, the slots, the severities and the impact bounds are public. Its content stays sealed for good: a known issue's run carries no disclosure wrap.`}
    >
      <TagMatch
        label={`Commit ${x.c.commit.id}'s key against known issue ${km.index + 1}`}
        left={{ title: `This commit (${x.c.commit.id})`, slot: km.key.slot, slotLabel: slotLabel(x.program, km.key.slot), tag: km.key.tag }}
        right={{
          title: `Known issue ${km.index + 1} (run ${km.issue.run})`,
          slot: Array.isArray(km.issue.slots) && km.issue.slots.length === 1 ? km.issue.slots[0] : km.key.slot,
          slotLabel: Array.isArray(km.issue.slots) ? km.issue.slots.map((s) => slotLabel(x.program, s)).join(", ") : slotLabel(x.program, km.key.slot),
          tag: String(km.issue.tags[km.tagIndex]),
        }}
      />
      <dl className={`kv ${styles.facts}`}>
        <dt>Declared</dt>
        <dd>
          {declared ? (
            <span className={styles.inline}>
              <Time t={declared.block_ts_ns} unit="ns" showRelative={false} /> <TxLink hash={declared.tx} href={declared.explorer} />
            </span>
          ) : (
            <span className="muted">—</span>
          )}
        </dd>
        <dt>Program opened</dt>
        <dd>
          {opened ? (
            <span className={styles.inline}>
              <Time t={opened.block_ts_ns} unit="ns" showRelative={false} /> <TxLink hash={opened.tx} href={opened.explorer} />
            </span>
          ) : (
            <span className="muted">—</span>
          )}
        </dd>
      </dl>
      <LiveCheck
        check={{ kind: "near-view", contract: x.contract, method: "get_program", args: { program_id: x.c.commit.program } }}
        expect={{ path: `known.${km.index}.tags.${km.tagIndex}`, equals: km.key.tag, says: `program ${x.c.commit.program}'s known issue ${km.index + 1} carries tag ${km.key.tag.slice(0, 12)}…` }}
        label="Read the program from NEAR"
      />
    </Evidence>
  );
}

function GapEvidence({ x }: { x: OutcomeInput }) {
  const claim = ev(x.events, "gap_claim");
  const promoted = ev(x.events, "gap_promoted");
  const round = numOf(evData(promoted).round);
  const caught = x.events.find((e) => e.event === "run_answered" && modeOfEvent(e) === "rerun" && strOf(evData(e).kind) === "Violation");
  const caughtKeys = cleanKeys(evData(caught).keys);
  const paid = ev(x.events, "paid");
  const gap = x.program?.program?.params?.gap;
  const threshold = isObj(gap) ? gap.threshold_usd_micro : undefined;
  const reserve = isObj(gap) ? strOf(gap.claim_amount) : null;
  return (
    <Evidence
      eyebrow="The gap lane"
      title={promoted ? "Harm no rule caught: refunded, recorded, then paid" : "Harm no rule caught: refunded and recorded"}
      lede="A proof that breaks no rule but harms a protected account above the program's threshold is a gap claim: its bond is returned and the claim waits, sealed, at its priority. A later round that adds a rule it breaks pays it at that priority."
    >
      <dl className={`kv ${styles.facts}`}>
        <dt>Protected harm</dt>
        <dd>
          <span className={styles.inline}>
            <strong className="num">{usd(evData(claim).harm_usd_micro)}</strong>
            <span className={styles.unit}>fixture USD{threshold !== undefined ? `, above the program's threshold of ${usd(threshold)}` : ""}</span>
            {claim ? <TxLink hash={claim.tx} href={claim.explorer} /> : null}
          </span>
        </dd>
        {reserve ? (
          <>
            <dt>Reserved</dt>
            <dd className="num">{wnear(reserve)}</dd>
          </>
        ) : null}
        {caught ? (
          <>
            <dt>Caught at the fix</dt>
            <dd>
              <span className={styles.inline}>
                Re-run {runOfEvent(caught) ?? "—"}: Violation of {caughtKeys.map((k) => slotName(x.program, k.slot)).join(", ") || "a rule"}
                {caughtKeys.some((k) => slotInfo(x.program, k.slot).added) ? ", a rule a round added" : ""} <TxLink hash={caught.tx} href={caught.explorer} />
              </span>
            </dd>
          </>
        ) : null}
        {promoted ? (
          <>
            <dt>Promoted</dt>
            <dd>
              <span className={styles.inline}>
                at round {round ?? "—"} <TxLink hash={promoted.tx} href={promoted.explorer} />
              </span>
            </dd>
          </>
        ) : null}
        {paid ? (
          <>
            <dt>Paid</dt>
            <dd>
              <span className={styles.inline}>
                {wnear(strOf(evData(paid).amount))} at its original priority <TxLink hash={paid.tx} href={paid.explorer} />
              </span>
            </dd>
          </>
        ) : null}
      </dl>
    </Evidence>
  );
}

// ------------------------------------------------------------------ the section body

export function Outcome(x: OutcomeInput) {
  const isDuplicate = x.label === "Duplicate" || x.label === "Done:DuplicateConfirmed" || x.events.some((e) => e.event === "duplicate");
  const isKnown = x.label === "Done:Known" || x.events.some((e) => e.event === "known_match");
  const isGap = x.events.some((e) => e.event === "gap_claim") || x.label === "GapClaim" || x.label === "Done:GapOpen";
  const note = recordedNote(x.c.commit.id, x.c.commit.program, x.c.commit.hunter);
  const expect = stateExpect(x.label, x.c.commit.state);
  return (
    <>
      <div className={styles.outcomeGrid}>
        <Bond x={x} />
        <Payout x={x} />
      </div>
      {note ? (
        <div className={`callout ${styles.spaced}`}>
          <Icon name="info" size={16} />
          <div className={styles.noteBody}>
            <p>
              <strong>In the recorded run</strong> of 2026-11-04 (
              <a href={sourceUrl("JUDGES.md")} target="_blank" rel="noreferrer noopener">
                JUDGES.md, row{note.rows.length > 1 ? "s" : ""} {note.rows.join(", ")}
              </a>
              ): {note.text}
            </p>
            {note.quote ? (
              <figure className={styles.quote}>
                <blockquote>
                  <code>{note.quote.text}</code>
                </blockquote>
                <figcaption className="xsmall muted">
                  {note.quote.what} (
                  <a href={sourceUrl(note.quote.file)} target="_blank" rel="noreferrer noopener">
                    {note.quote.file.split("/").pop()}
                  </a>
                  ).
                </figcaption>
              </figure>
            ) : null}
          </div>
        </div>
      ) : null}
      {isDuplicate ? <DuplicateEvidence x={x} /> : null}
      {isKnown ? <KnownEvidence x={x} /> : null}
      {isGap ? <GapEvidence x={x} /> : null}
      {expect ? (
        <div className={`${styles.checks} ${styles.spaced}`}>
          <LiveCheck
            check={{ kind: "near-view", contract: x.contract, method: "get_commit", args: { commit_id: x.c.commit.id } }}
            expect={expect}
            cli={`antibody status --commit ${x.c.commit.id}`}
            label={x.label === "Done:Paid" ? "Check the payout on NEAR" : "Check the state on NEAR"}
          />
        </div>
      ) : null}
    </>
  );
}
