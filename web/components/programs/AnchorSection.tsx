import { Command } from "@/components/ui/Command";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { Section } from "@/components/ui/PageHead";
import { TxLink } from "@/components/ui/TxLink";
import type { ProgramDetail } from "@/lib/api";
import { chainName } from "@/lib/config";
import { AnchorCheck } from "./AnchorCheck";
import { anchorConfirmed, evmBlockUrl, findEvent } from "./helpers";
import { LocationView } from "./Location";
import { AnchorBadge } from "./ProgramBadges";
import styles from "./programs.module.css";

/** NEAR testnet's Ethereum light client follows Sepolia only. */
const LIGHT_CLIENT_CHAIN = 11155111;

/** 4. The anchor block every proof runs at, and how its hash was checked. */
export function AnchorSection({ d, id, lightClient }: { d: ProgramDetail; id: number; lightClient: string }) {
  const a = d.rules?.anchor;
  const chainId = a?.chain_id ?? d.meta?.chain_id;
  const block = a?.block ?? d.meta?.anchor_block;
  const hash = a?.hash ?? d.meta?.anchor_hash;
  const checked = findEvent(d, "anchor_checked");
  const confirmed = anchorConfirmed(a?.check);
  const base = d.meta?.base ?? null;
  const blockUrl = chainId && block ? evmBlockUrl(chainId, block) : undefined;

  return (
    <Section
      id="anchor"
      title="The anchor"
      lede="Every proof runs on the state proven at one block. A pause ticket, and the verdict guard, act only on an anchor the light client confirmed: on a project-attested anchor a verdict is about state the project chose."
    >
      <div className="grid grid-2">
        <div className="card">
          <dl className={`kv ${styles.kvStack}`}>
            <dt>Chain</dt>
            <dd>{chainId ? `${chainName(chainId)} (${chainId})` : "—"}</dd>
            <dt>Block</dt>
            <dd>
              {block ? (
                blockUrl ? (
                  <a href={blockUrl} target="_blank" rel="noreferrer noopener" className="num">
                    {block} <Icon name="external" size={11} className={styles.inlineIcon} />
                    <span className="visually-hidden"> (opens an explorer)</span>
                  </a>
                ) : (
                  <span className="num">{block}</span>
                )
              ) : (
                "—"
              )}
            </dd>
            <dt>Hash</dt>
            <dd className="stack" style={{ ["--gap" as string]: "4px" }}>
              <span className={styles.full}>{hash ? `0x${hash.replace(/^0x/, "")}` : "—"}</span>
            </dd>
            <dt>Its check</dt>
            <dd className="stack" style={{ ["--gap" as string]: "4px" }}>
              <span>
                <AnchorBadge check={a?.check} size="sm" long />
              </span>
              <span className="xsmall muted">
                {confirmed
                  ? `near-antibody compared the hash with ${lightClient} when the program was created.`
                  : "The light client could not confirm this hash when the program was created: the project attests to it."}
              </span>
              {checked ? (
                <span className="xsmall">
                  anchor_checked ({String(checked.data?.anchor ?? "—")}): <TxLink hash={checked.tx} href={checked.explorer} />
                </span>
              ) : null}
            </dd>
            {base ? (
              <>
                <dt>Base witness</dt>
                <dd className="stack" style={{ ["--gap" as string]: "4px" }}>
                  <span className="xsmall muted">The anchor state the rules and the round-0 checks read, proven against the anchor and pinned at activation.</span>
                  <Hash value={base.sha256} head={16} label="base witness sha256" />
                  <LocationView location={base.location} what="The base witness" />
                </dd>
              </>
            ) : null}
          </dl>
        </div>

        <div className="card stack">
          <h3 className={styles.cardTitle}>Check the hash yourself</h3>
          {chainId === LIGHT_CLIENT_CHAIN && block && hash ? (
            <AnchorCheck lightClient={lightClient} block={block} hash={hash} />
          ) : (
            <p className="small muted">
              NEAR testnet&apos;s Ethereum light client follows Sepolia only; this anchor{chainId ? ` is on ${chainName(chainId)}` : " has no chain"}, so there is
              nothing to ask it.
            </p>
          )}
          <p className="xsmall muted">The same check from a terminal, and the one that also hashes the block header from a Sepolia RPC:</p>
          <Command label="CLI command">{`antibody judge --program ${id}`}</Command>
          {block ? <Command label="CLI command">{`antibody anchor-check --block ${block}`}</Command> : null}
        </div>
      </div>
    </Section>
  );
}
