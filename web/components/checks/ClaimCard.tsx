import Link from "next/link";
import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { sourceUrl } from "@/lib/config";
import { Md } from "./Md";
import { safeUrl, specOf, type JudgeClaim, type JudgeLink } from "./judges";
import styles from "./checks.module.css";

/** Where the same facts are on this site, by the claim's kind. */
function siteLinks(c: JudgeClaim, acme: number | undefined): { href: string; label: string }[] {
  const out: { href: string; label: string }[] = [];
  if (c.kind === "one_moment") out.push({ href: "/moment", label: "The one moment, step by step" });
  if (c.kind === "practice") out.push({ href: "/try", label: "Run your own try" });
  if (typeof c.commit === "number") out.push({ href: `/commits/${c.commit}`, label: `Commit ${c.commit}` });
  const program = typeof c.program === "number" ? c.program : ["round", "release", "rule_strength", "verdict_consumer", "audit", "one_moment"].includes(c.kind) ? acme : undefined;
  if (typeof program === "number") out.push({ href: `/programs/${program}`, label: `Program ${program}` });
  return out;
}

function Evidence({ link }: { link: JudgeLink }) {
  if (link.path) {
    return (
      <a href={sourceUrl(link.path)} target="_blank" rel="noreferrer noopener" className="small">
        {link.label} <Icon name="external" size={11} style={{ display: "inline", verticalAlign: "-1px" }} />
      </a>
    );
  }
  const href = safeUrl(link.url);
  const value = link.tx ?? link.url ?? "";
  return (
    <>
      <span className={styles.linkLabel}>{link.label}</span>
      <Hash value={value} href={href} head={8} label={`${link.label} transaction`} />
    </>
  );
}

/**
 * One claim of JUDGES.md, as its row states it (the same cells), with the transactions behind it, the
 * command that checks it, and the checks a browser can run on it, each showing the raw answer.
 */
export function ClaimCard({ claim, acme, evmCount }: { claim: JudgeClaim; acme?: number; evmCount: number }) {
  const id = `claim-${claim.n}`;
  const live = claim.live ?? [];
  const [first, ...more] = live;
  const shown = new Set(live.map((l) => l.cli).filter(Boolean));
  const commands = (claim.commands ?? []).filter((c) => !shown.has(c));
  const links = claim.links ?? [];
  const textHasLinks = /\]\(/.test(claim.evidence ?? "");
  const onSite = siteLinks(claim, acme);
  return (
    <li>
      <article id={id} className={styles.claim} aria-labelledby={`${id}-title`}>
        <header className={styles.claimHead}>
          <a href={`#${id}`} className={styles.claimNo} aria-label={`Claim ${claim.n}: link to this claim`}>
            {claim.n}
          </a>
          <h3 id={`${id}-title`} className={styles.claimTitle}>
            <Md text={claim.claim} />
          </h3>
        </header>
        <div className={styles.claimBody}>
          <dl className={styles.facts}>
            <div>
              <dt>Expected</dt>
              <dd>
                <Md text={claim.expected} />
              </dd>
            </div>
            <div>
              <dt>Evidence</dt>
              {!textHasLinks && claim.evidence ? (
                <dd>
                  <Md text={claim.evidence} />
                </dd>
              ) : null}
              {links.length ? (
                <dd>
                  <ul className={styles.links}>
                    {links.map((l, i) => (
                      <li key={`${l.label}-${i}`}>
                        <Evidence link={l} />
                      </li>
                    ))}
                  </ul>
                </dd>
              ) : null}
              {!links.length && textHasLinks ? (
                <dd>
                  <Md text={claim.evidence} />
                </dd>
              ) : null}
            </div>
            <div>
              <dt>Check, as JUDGES.md names it</dt>
              <dd>
                <Md text={claim.check} />
              </dd>
            </div>
            {onSite.length ? (
              <div>
                <dt>On this site</dt>
                <dd className={styles.siteLinks}>
                  {onSite.map((s) => (
                    <Link key={s.href} href={s.href}>
                      {s.label}
                    </Link>
                  ))}
                </dd>
              </div>
            ) : null}
          </dl>
          <div className={styles.run}>
            {first ? (
              <LiveCheck check={specOf(first)} expect={first.expect} cli={first.cli} label={first.label} />
            ) : (
              <p className="small muted">
                {evmCount ? "This claim is checked on the EVM chains: its reads are below the claims." : "This check runs in a terminal: its command is below."}
              </p>
            )}
            {more.length ? (
              <details className={styles.more}>
                <summary>
                  {more.length === 1 ? "One more check" : `${more.length} more checks`}, read straight from the chain
                </summary>
                <div className={styles.moreBody}>
                  {more.map((m, i) => (
                    <LiveCheck key={`${m.kind}-${i}`} check={specOf(m)} expect={m.expect} cli={m.cli} label={m.label} />
                  ))}
                </div>
              </details>
            ) : null}
            {evmCount ? (
              <p className="small">
                <a href="#evm-checks">
                  {evmCount === 1 ? "Its EVM check" : `Its ${evmCount} EVM checks`}, with the <code>curl</code> commands <Icon name="arrowDown" size={12} style={{ display: "inline", verticalAlign: "-2px" }} />
                </a>
              </p>
            ) : null}
            {commands.map((c, i) => (
              <Command key={c} label={commands.length > 1 ? `CLI command ${i + 1}` : "CLI command"}>
                {c}
              </Command>
            ))}
          </div>
        </div>
      </article>
    </li>
  );
}
