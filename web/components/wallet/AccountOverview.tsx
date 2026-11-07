"use client";

import Link from "next/link";
import { useCallback, useEffect, useRef, useState } from "react";
import { Icon } from "@/components/ui/Icon";
import { Section } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { ErrorState, Loading } from "@/components/ui/States";
import { AccountLink } from "@/components/ui/TxLink";
import { Time } from "@/components/ui/Time";
import { fixtureAgentLetter, provenance } from "@/lib/format";
import { loadAccount, loadActivity, loadHistory, loadNearBalance } from "@/lib/wallet/account";
import type { Session } from "@/lib/wallet/connector";
import { ANTIBODY, WNEAR } from "@/lib/wallet/contracts";
import type { AccountData, Activity, History } from "@/lib/wallet/types";
import { Actions } from "./Actions";
import { ActivityTable } from "./ActivityTable";
import type { ApiContracts } from "./AccountApp";
import { AgentStatementView } from "./AgentStatementView";
import { Agents } from "./Agents";
import { Amount, type Load } from "./shared";
import styles from "./wallet.module.css";

export type Blocked = "network" | "api" | null;

/** How the account is labelled when it is one of ours. */
function Provenance({ account }: { account: string }) {
  const p = provenance(account);
  if (p === "fixture-agent") return <Pill tone="info" icon="cpu">our fixture agent {fixtureAgentLetter(account)}</Pill>;
  if (p === "practice") return <Pill tone="neutral">practice</Pill>;
  if (p === "ours") return <Pill tone="neutral">an Antibody account</Pill>;
  return null;
}

type NearBalance = { amount: string; locked: string } | null;

interface Snapshot {
  account: Load<AccountData>;
  activity: Load<Activity>;
  history: Load<History>;
  balance: Load<NearBalance>;
}

const asLoad = <T,>(r: { ok: true; data: T } | { ok: false; error: string }): Load<T> => (r.ok ? { status: "ok", data: r.data } : { status: "error", error: r.error });

/** Reads everything; `fresh` reads the account's views from NEAR rather than the API's short cache. */
async function readAll(id: string, fresh: boolean): Promise<Snapshot> {
  const [a, act, h, b] = await Promise.all([loadAccount(id, fresh), loadActivity(id), loadHistory(id), loadNearBalance(id)]);
  return { account: asLoad(a), activity: asLoad(act), history: asLoad(h), balance: asLoad(b) };
}

/** The connected account: its numbers, its commits, the actions it can take, its agents. */
export function AccountOverview({
  session,
  onDisconnect,
  blocked,
  apiContracts,
  focusOnMount,
}: {
  session: Session;
  onDisconnect: () => void;
  blocked: Blocked;
  apiContracts: ApiContracts;
  /** After a connect the person started (not a restored session), focus moves to the account. */
  focusOnMount: boolean;
}) {
  const id = session.accountId;
  const [account, setAccount] = useState<Load<AccountData>>({ status: "loading" });
  const [activity, setActivity] = useState<Load<Activity>>({ status: "loading" });
  const [history, setHistory] = useState<Load<History>>({ status: "loading" });
  const [balance, setBalance] = useState<Load<NearBalance>>({ status: "loading" });
  const [busy, setBusy] = useState(false);
  const heading = useRef<HTMLSpanElement>(null);

  const apply = useCallback((s: Snapshot) => {
    setAccount(s.account);
    setActivity(s.activity);
    setHistory(s.history);
    setBalance(s.balance);
  }, []);

  useEffect(() => {
    if (focusOnMount) heading.current?.focus();
  }, [focusOnMount]);

  useEffect(() => {
    let live = true;
    readAll(id, false)
      .then((s) => {
        if (live) apply(s);
      })
      .catch((e: unknown) => {
        if (live) apply({ account: { status: "error", error: String(e) }, activity: { status: "error", error: String(e) }, history: { status: "error", error: String(e) }, balance: { status: "error", error: String(e) } });
      });
    return () => {
      live = false;
    };
  }, [id, apply]);

  const refresh = useCallback(
    async (fresh: boolean) => {
      setBusy(true);
      try {
        apply(await readAll(id, fresh));
      } finally {
        setBusy(false);
      }
    },
    [id, apply],
  );

  const data = account.status === "ok" ? account.data : null;
  const credit = data?.credit ?? null;
  const statement = data?.statement ?? null;
  const isAgent = Boolean(statement?.registered || credit?.agent);

  return (
    <>
      <Section
        id="account"
        eyebrow={`Connected with ${session.walletName}`}
        title="Account"
        lede="Credit held for you at near-antibody, and what your wallet holds. Amounts show up to 6 decimals; the exact raw values are in the checks below."
        actions={
          <>
            <button type="button" className="btn btn-sm" onClick={() => refresh(true)} disabled={busy} aria-busy={busy}>
              <Icon name="refresh" size={13} /> {busy ? "Reading…" : "Refresh from NEAR"}
            </button>
            <button type="button" className="btn btn-sm btn-ghost" onClick={onDisconnect}>
              <Icon name="x" size={13} /> Disconnect
            </button>
          </>
        }
      >
        <div className="stack">
          <div className={styles.who}>
            <div className="stack" style={{ ["--gap" as string]: "6px" }}>
              {/* Focused once connected: the connect button it replaces is gone. */}
              <span ref={heading} tabIndex={-1} className={styles.accountId}>
                <span className="visually-hidden">Connected account: </span>
                {id}
              </span>
              <div className="row" style={{ ["--gap" as string]: "8px" }}>
                <Provenance account={id} />
                {isAgent ? <Pill tone="info" icon="cpu">agent</Pill> : null}
                {credit?.committed ? (
                  <Pill tone="neutral" title="This account committed as its own beneficiary, so it can never become an agent.">
                    commits as itself
                  </Pill>
                ) : null}
                <AccountLink account={id} />
              </div>
            </div>
          </div>

          {account.status === "loading" ? (
            <Loading label="Reading your account" />
          ) : account.status === "error" ? (
            <ErrorState what="your account" error={account.error} />
          ) : (
            <>
              <dl className={styles.tiles} aria-live="polite">
                <div>
                  <dt>Token credit at near-antibody</dt>
                  <dd>
                    <Amount raw={credit ? (credit.tokens ?? "0") : null} />
                  </dd>
                  <dd className={styles.tileNote}>Withdrawable; bonds are taken from it</dd>
                </div>
                <div>
                  <dt>NEAR credit at near-antibody</dt>
                  <dd>
                    <Amount raw={credit ? (credit.near ?? "0") : null} unit="NEAR" />
                  </dd>
                  <dd className={styles.tileNote}>Pays storage and verification runs</dd>
                </div>
                <div>
                  <dt>wNEAR in your wallet</dt>
                  <dd>
                    <Amount raw={data?.tokenBalance ?? null} />
                  </dd>
                  <dd className={styles.tileNote}>On {WNEAR}</dd>
                </div>
                <div>
                  <dt>NEAR in your wallet</dt>
                  <dd>
                    {balance.status === "ok" && balance.data ? (
                      <Amount raw={balance.data.amount} unit="NEAR" />
                    ) : balance.status === "loading" ? (
                      <span className="muted">…</span>
                    ) : (
                      <span className="muted">—</span>
                    )}
                  </dd>
                  <dd className={styles.tileNote}>{balance.status === "error" ? "The RPC did not answer" : "Read by your browser from NEAR"}</dd>
                </div>
              </dl>
              <p className="xsmall muted">
                {data?.source === "near" ? "Read by your browser from NEAR just now" : "From the API (cached for a few seconds)"}
                {data ? (
                  <>
                    , <Time t={data.at} unit="ms" />
                  </>
                ) : null}
                .{data?.apiError ? ` The API did not answer (${data.apiError}); these are the same views, read from NEAR.` : ""} wNEAR on testnet has no
                value.
              </p>
              <details className={styles.details}>
                <summary>Check these numbers on NEAR yourself</summary>
                <div className={styles.checks}>
                  <LiveCheck
                    check={{ kind: "near-view", contract: ANTIBODY, method: "get_account", args: { account_id: id } }}
                    expect={{ path: "tokens", equals: credit?.tokens ?? "0", says: `tokens is your token credit (${credit?.tokens ?? "0"} raw)` }}
                    cli={`antibody status --of ${id}`}
                  />
                  <LiveCheck check={{ kind: "near-view", contract: ANTIBODY, method: "agent_statement", args: { agent: id } }} expect={{ path: "registered", equals: isAgent, says: isAgent ? "registered: true" : "registered: false" }} />
                  <LiveCheck check={{ kind: "near-view", contract: WNEAR, method: "ft_balance_of", args: { account_id: id } }} expect={{ equals: data?.tokenBalance ?? "0", says: `your wNEAR balance (${data?.tokenBalance ?? "0"} raw)` }} />
                </div>
              </details>
            </>
          )}
          {activity.status === "ok" && activity.data.programs.some((p) => p.project === id) ? (
            <div className="callout">
              <Icon name="info" size={16} />
              <span>
                You opened{" "}
                {activity.data.programs
                  .filter((p) => p.project === id)
                  .map((p, i, all) => (
                    <span key={p.id}>
                      <Link href={`/programs/${p.id}`}>program {p.id}</Link>
                      {i < all.length - 1 ? ", " : ""}
                    </span>
                  ))}
                . To read a proof you paid for, use the CLI: <a href="#read-proofs">why it stays there</a>.
              </span>
            </div>
          ) : null}
        </div>
      </Section>

      {statement?.registered || credit?.agent ? <AgentStatementView statement={statement} record={credit?.agent ?? null} /> : null}

      <ActivityTable account={id} activity={activity} />

      <Actions session={session} account={account} history={history} blocked={blocked} apiContracts={apiContracts} onSettled={() => refresh(true)} />

      <Agents
        session={session}
        account={account}
        history={history}
        programs={activity.status === "ok" ? activity.data.programs : null}
        isAgent={isAgent}
        blocked={blocked}
        apiContracts={apiContracts}
        onSettled={() => refresh(true)}
      />
    </>
  );
}
