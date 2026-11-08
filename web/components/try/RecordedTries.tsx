import Link from "next/link";
import { Command } from "@/components/ui/Command";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { Pill } from "@/components/ui/Pill";
import { TxLink } from "@/components/ui/TxLink";
import { sourceUrl } from "@/lib/config";
import { provenance, wnear } from "@/lib/format";
import { RECORDED_FILE, RECORDED_TRIES } from "./recorded";
import styles from "./try.module.css";

/**
 * The fallback when a try cannot run now (the hourly vend limit, a low faucet, busy RPCs): the two
 * recorded tries, each transaction linked, a check the browser runs for each, and the CLI command.
 */
export function RecordedTries({ id = "recorded" }: { id?: string }) {
  return (
    <section id={id} className={`block ${styles.recorded}`} aria-labelledby={`${id}-title`}>
      <div className="block-head">
        <div className="stack" style={{ ["--gap" as string]: "6px" }}>
          <div className="row" style={{ ["--gap" as string]: "8px" }}>
            <p className="eyebrow">If a try cannot run now</p>
            <Pill tone="neutral" icon="clock" size="sm">
              recorded
            </Pill>
          </div>
          <h2 id={`${id}-title`}>Recorded tries, and the CLI</h2>
          <p className="muted small block-lede">
            Two practice tries from the 2026-11-04 testnet run, each account vended by the public vend key for someone with no NEAR account: recorded, not live. Every
            hash below is a NEAR testnet transaction.{" "}
            <a href={sourceUrl(RECORDED_FILE)} target="_blank" rel="noreferrer noopener">
              The results file <Icon name="external" size={11} style={{ display: "inline" }} />
            </a>
          </p>
        </div>
      </div>
      <div className={styles.recordedGrid}>
        {RECORDED_TRIES.map((t) => (
          <article key={t.account} className={`card ${styles.recordedCard}`} aria-labelledby={`rec-${t.commitId}`}>
            <div className="row" style={{ ["--gap" as string]: "8px", justifyContent: "space-between" }}>
              <h3 id={`rec-${t.commitId}`} className={`mono small ${styles.wrapAny}`}>
                {t.account}
              </h3>
              {provenance(t.account) === "practice" ? (
                <Pill tone="neutral" size="sm">
                  practice try
                </Pill>
              ) : null}
            </div>
            <p className="small">
              <strong>{t.state}</strong>, {wnear(t.paidRaw)} to the vending contract, <span className="num">{t.seconds.toFixed(1)} s</span> from the vend to the payout
              (vault {t.vault}, run {t.runId}). <Link href={`/commits/${t.commitId}`}>Commit {t.commitId}</Link> times each of its steps from the commit&apos;s block.
            </p>
            <ul className={styles.txList}>
              {t.txs.map((x) => (
                <li key={x.hash}>
                  <span className={styles.txStep}>{x.step}</span>
                  <TxLink hash={x.hash} />
                </li>
              ))}
            </ul>
            <LiveCheck
              check={{ kind: "near-view", contract: "bounty.antibody.testnet", method: "get_commit", args: { commit_id: t.commitId } }}
              expect={{ path: "state.Done.Paid.amount", equals: t.paidRaw, says: `commit ${t.commitId} is Done:Paid, ${wnear(t.paidRaw)} (${t.paidRaw} raw)` }}
              cli={`antibody status --commit ${t.commitId}`}
              label={`Check commit ${t.commitId} on NEAR`}
            />
          </article>
        ))}
      </div>
      <div className="stack" style={{ ["--gap" as string]: "8px" }}>
        <p className="small">
          The same try from a terminal signs the same transactions. The release binary (the README shows how to download it and check its
          sha256) has the testnet deployment built in; it needs no account and no key:
        </p>
        <Command label="Try from the CLI">antibody try --name &lt;you&gt;</Command>
        <p className="xsmall muted">Once the public vend key&apos;s allowance is spent, the CLI can vend with your own testnet account instead:</p>
        <Command label="Try with your own testnet account">antibody --account &lt;your-account.testnet&gt; try --name &lt;you&gt;</Command>
      </div>
    </section>
  );
}
