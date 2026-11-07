import Link from "next/link";
import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { chainName, sourceUrl } from "@/lib/config";
import { ClaimResult, SharedCheck } from "./ClaimChecks";
import { Md } from "./Md";
import { evmCheck, limitsFor, primaryChecks, safeUrl, specOf, type ClaimCheck, type JudgeClaim, type JudgeEvmCheck, type JudgeLink } from "./judges";
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

/** A command as text, for comparing JUDGES.md's check cell with the command boxes. */
const plain = (s: string | null | undefined) => (s ?? "").replace(/`/g, "").replace(/\s+/g, " ").trim();

/** What an EVM read reads, over its check: "Sepolia · AcmeLend" (a contract name in mono). */
function EvmSubject({ chainId, subject }: { chainId: number; subject?: string }) {
  return (
    <p className={styles.evmSubject}>
      {chainName(chainId)}
      {subject ? <> · {/^[A-Z][A-Za-z0-9]*$/.test(subject) ? <code>{subject}</code> : subject}</> : null}
    </p>
  );
}

function ClaimCheckView({ c, evm }: { c: ClaimCheck; evm?: JudgeEvmCheck }) {
  return (
    <div className="stack" style={{ ["--gap" as string]: "6px" }}>
      {evm ? <EvmSubject chainId={evm.chain_id} subject={evm.subject} /> : null}
      <SharedCheck c={c} />
    </div>
  );
}

/**
 * One claim of JUDGES.md, as its row states it (the same cells), with the transactions behind it, the
 * limits that bound it, and the checks a browser can run on it (its EVM reads included), each showing the
 * raw answer. Its first check shares its answer with the claims index; the card's header shows the result.
 */
export function ClaimCard({
  claim,
  acme,
  evm,
  limits,
}: {
  claim: JudgeClaim;
  acme?: number;
  /** The run's EVM checks; the card shows the ones that back this claim. */
  evm: JudgeEvmCheck[];
  /** README's limitations, as the Limits section lists them. */
  limits: { title: string; text: string }[];
}) {
  const id = `claim-${claim.n}`;
  const live = claim.live ?? [];
  const mine = evm.filter((e) => e.claim === claim.n);
  const primary = primaryChecks(claim, evm);
  const primaryEvm = live.length ? [] : mine.filter((e) => e.live);
  const [first, ...rest] = primary;
  // Beyond the claim's own checks: its further live checks and, when it has a live check, its EVM reads.
  const moreLive = live.slice(1);
  const moreEvm = live.length ? mine.filter((e) => e.live) : [];
  const curlOnly = mine.filter((e) => !e.live);
  const moreCount = rest.length + moreLive.length + moreEvm.length;
  const onChainOnly = moreLive.every((m) => m.kind !== "api");

  const shown = new Set([...live.map((l) => l.cli), ...mine.map((e) => e.curl)].filter(Boolean));
  const commands = (claim.commands ?? []).filter((c) => !shown.has(c));
  // JUDGES.md's check cell, only when it says more than the command boxes already do.
  const boxes = new Set([...shown, ...(claim.commands ?? [])].map(plain));
  const checkSaysMore = Boolean(claim.check) && !boxes.has(plain(claim.check));
  const links = claim.links ?? [];
  const textHasLinks = /\]\(/.test(claim.evidence ?? "");
  const onSite = siteLinks(claim, acme);
  const bounds = limitsFor(claim, limits);

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
          <ClaimResult checks={primary} />
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
            {checkSaysMore ? (
              <div>
                <dt>Check, as JUDGES.md names it</dt>
                <dd>
                  <Md text={claim.check} />
                </dd>
              </div>
            ) : null}
            {claim.does_not_show ? (
              <div>
                <dt>Does not show</dt>
                <dd>
                  <Md text={claim.does_not_show} />
                </dd>
              </div>
            ) : null}
            {bounds.length ? (
              <div>
                <dt>Limits that apply</dt>
                <dd>
                  <ul className={styles.limitLinks}>
                    {bounds.map((b) => (
                      <li key={b.index}>
                        <a href={`#limit-${b.index}`} title={plain(b.text)}>
                          <Md text={b.label} />
                        </a>
                      </li>
                    ))}
                  </ul>
                </dd>
              </div>
            ) : null}
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
              <ClaimCheckView c={first} evm={primaryEvm[0]} />
            ) : (
              <p className="small muted">This check runs in a terminal: its command is below.</p>
            )}
            {moreCount ? (
              <details className={styles.more}>
                <summary>
                  {moreCount === 1 ? "One more check" : `${moreCount} more checks`}
                  {onChainOnly ? ", read straight from the chain" : ""}
                </summary>
                <div className={styles.moreBody}>
                  {rest.map((c, i) => (
                    <ClaimCheckView key={c.id} c={c} evm={primaryEvm[i + 1]} />
                  ))}
                  {moreLive.map((m, i) => (
                    <LiveCheck key={`${m.kind}-${i}`} check={specOf(m)} expect={m.expect} cli={m.cli} label={m.label} />
                  ))}
                  {moreEvm.map((e, i) => {
                    const c = evmCheck(e, `${claim.n}.more.${i}`);
                    return (
                      <div key={c.id} className="stack" style={{ ["--gap" as string]: "6px" }}>
                        <EvmSubject chainId={e.chain_id} subject={e.subject} />
                        <LiveCheck check={c.check} expect={c.expect} cli={c.cli} label={c.label} />
                      </div>
                    );
                  })}
                </div>
              </details>
            ) : null}
            {curlOnly.map((e) => (
              <div key={`${e.chain_id}-${e.address}`} className="stack" style={{ ["--gap" as string]: "6px" }}>
                <EvmSubject chainId={e.chain_id} subject={e.subject} />
                <Command label="curl command">{e.curl}</Command>
              </div>
            ))}
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
