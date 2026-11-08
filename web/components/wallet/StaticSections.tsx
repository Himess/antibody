import { Cell } from "@/components/programs/Cell";
import cards from "@/components/programs/programs.module.css";
import { Command } from "@/components/ui/Command";
import { Icon } from "@/components/ui/Icon";
import { Section } from "@/components/ui/PageHead";
import { config, sourceUrl } from "@/lib/config";
import { ANTIBODY, DEPOSIT_MSG, SOURCES, WNEAR } from "@/lib/wallet/contracts";
import { WALLET_CONNECT_SRC } from "@/lib/wallet/csp";
import { WALLET_INFO } from "@/lib/wallet/manifest";
import { ContractCheck } from "./ContractCheck";
import styles from "./wallet.module.css";

function Src({ path, children }: { path: string; children: React.ReactNode }) {
  return (
    <a href={sourceUrl(path)} target="_blank" rel="noreferrer noopener">
      {children} <Icon name="external" size={11} style={{ display: "inline" }} />
    </a>
  );
}

const CALLS: { action: string; to: string; method: string; args: string; attached: string; gas: string }[] = [
  { action: "Add NEAR credit", to: ANTIBODY, method: "storage_deposit", args: "{}", attached: "the NEAR you choose", gas: "30 Tgas" },
  {
    action: "Wrap NEAR (in a deposit, when your wallet is short)",
    to: WNEAR,
    method: "storage_deposit, then near_deposit",
    args: '{"account_id": <you>, "registration_only": true}, then {}',
    attached: "wrap.testnet's registration once (read before you sign), then the NEAR you wrap",
    gas: "30 Tgas each",
  },
  {
    action: "Deposit wNEAR into your credit",
    to: WNEAR,
    method: "ft_transfer_call",
    args: `{"receiver_id": "${ANTIBODY}", "amount": <raw>, "msg": ${JSON.stringify(DEPOSIT_MSG)}}`,
    attached: "1 yoctoNEAR",
    gas: "300 Tgas",
  },
  { action: "Withdraw wNEAR credit", to: ANTIBODY, method: "withdraw", args: '{"amount": <raw>}', attached: "1 yoctoNEAR", gas: "60 Tgas" },
  { action: "Withdraw NEAR credit", to: ANTIBODY, method: "storage_withdraw", args: '{"amount": <yoctoNEAR>}', attached: "1 yoctoNEAR", gas: "30 Tgas" },
  {
    action: "Register an agent, or change one",
    to: ANTIBODY,
    method: "register_agent",
    args: '{"agent": <name>.<you>, "bond_budget": <raw>, "per_commit_cap": <raw>, "programs": null | [ids]}',
    attached: "1 yoctoNEAR",
    gas: "30 Tgas",
  },
];

/** Every transaction the page can ask a wallet to sign, and what it never asks for. */
export function SignableCalls() {
  return (
    <Section
      id="calls"
      eyebrow="Transparency"
      title="Every transaction this page can ask you to sign"
      lede={
        <>
          The same methods, arguments, deposits and gas as the CLI (<Src path={SOURCES.cli}>crates/cli/src/cmd.rs</Src>), on the contracts in{" "}
          <Src path={SOURCES.deployment}>deployments/near-testnet.json</Src>. Raw amounts have 24 decimals.
        </>
      }
    >
      <div className="stack">
        <div className={`table-wrap ${cards.cardsWrap}`}>
          <table className={`table ${cards.rtable} ${styles.callsTable}`}>
            <caption className="visually-hidden">Transactions the account page can ask a wallet to sign</caption>
            <thead>
              <tr>
                <th scope="col">Action</th>
                <th scope="col">Contract and method</th>
                <th scope="col">Arguments</th>
                <th scope="col">Attached</th>
                <th scope="col">Gas</th>
              </tr>
            </thead>
            <tbody>
              {CALLS.map((c) => (
                <tr key={c.action}>
                  <Cell label="Action" lead>
                    {c.action}
                  </Cell>
                  <Cell label="Call">
                    <span className={styles.cellStack}>
                      <span className="mono">{c.to}</span>
                      <span className="mono xsmall">{c.method}</span>
                    </span>
                  </Cell>
                  <Cell label="Arguments" className={styles.argsCell}>
                    <span className={styles.codeText}>{c.args}</span>
                  </Cell>
                  <Cell label="Attached" className="small">
                    {c.attached}
                  </Cell>
                  <Cell label="Gas" className="small num nowrap">
                    {c.gas}
                  </Cell>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="grid grid-2">
          <p className="callout">
            <Icon name="key" size={16} />
            <span>
              One yoctoNEAR is attached wherever credit leaves near-antibody or an agent gets a budget. A function-call key cannot attach a deposit, so only
              a full-access key signs these: an agent&apos;s key never can (<Src path={SOURCES.contract}>near-antibody</Src>).
            </span>
          </p>
          <p className="callout callout-ok">
            <Icon name="shield" size={16} />
            <span>
              Never asked for: a message signature, a new access key, a plain transfer, or a call to any other contract. The NEAR and wNEAR you deposit are
              credited to your own account, at near-antibody or at wrap.testnet. Signing in only tells the page your account id.
            </span>
          </p>
        </div>
        <div className={styles.subBlock}>
          <h3>Before you sign: the code at these contracts</h3>
          <ContractCheck />
        </div>
      </div>
    </Section>
  );
}

/** Why a project reads paid proofs in the CLI, never through a web page (threat model P13). */
export function ReadProofs() {
  return (
    <Section
      id="read-proofs"
      eyebrow="For projects"
      title="Reading paid proofs stays in the CLI"
      lede="A web page that asks a project's wallet for one particular signature can read every finding ever paid to that project. This page never asks, and no page should."
    >
      <div className="grid grid-2">
        <div className="prose small">
          <p>
            A finding paid to a project is wrapped to the project&apos;s key. With <span className="mono">keygen x25519 --nep413</span> that key is derived from
            the project wallet&apos;s NEP-413 signature over one fixed message naming near-antibody and the project (
            <Src path={`${SOURCES.cryptoSpec}#33-project-key`}>crypto spec §3.3</Src>). The signature is deterministic, so whoever obtains it holds the key.
          </p>
          <p>
            That is a phishing-shaped request: a page that got the project&apos;s wallet to sign the message would read every paid finding (
            <Src path={SOURCES.threatModel}>threat model, P13</Src>). So the key is derived, and paid proofs are read, only on the project&apos;s own machine.
          </p>
          <p>
            <strong>If any website asks your wallet to sign a message that begins “Antibody project key v2”, reject it.</strong> This site never shows that
            request.
          </p>
          <p>
            A random key (<span className="mono">keygen x25519</span> without <span className="mono">--nep413</span>) has no message to phish; keep its file
            safe instead.
          </p>
        </div>
        <div className="stack">
          <Command label="Derive the project key, once">{"antibody --account <project>.testnet keygen x25519 --nep413 --out project-key.json"}</Command>
          <Command label="Read a paid finding">
            {`antibody --account <project>.testnet program read --commit <id> --project-key project-key.json --out ./finding --api ${config.apiShown}`}
          </Command>
          <p className="small muted">
            <span className="mono">program read</span> asks for the finding&apos;s release wrap the first time (the requester pays), unwraps the content key
            with the project key, opens the sealed proof and re-runs it locally; the local receipt must equal the enclave&apos;s. In the recorded run the project
            read A&apos;s paid proof this way, and near-antibody refused to wrap D&apos;s duplicate (
            <a href={`${config.repo}/blob/main/JUDGES.md`} target="_blank" rel="noreferrer noopener">
              JUDGES.md, row 16
            </a>
            ).
          </p>
        </div>
      </div>
    </Section>
  );
}

const hostPath = (u: string) => {
  try {
    const x = new URL(u);
    return { host: x.host, path: x.pathname };
  } catch {
    return { host: u, path: "" };
  }
};

/** What the wallet connection loads, from where, and what it may reach (with the CSP that enforces it). */
export function HowItConnects() {
  return (
    <Section
      id="how-it-connects"
      eyebrow="What loads, and from where"
      title="How the wallet connects"
      lede="Nothing below loads until you pick a wallet. The page keeps no key: your wallet signs, after you review each transaction here."
    >
      <div className="stack">
        <div className="grid grid-3">
          <div className="card-flat stack" style={{ ["--gap" as string]: "6px" }}>
            <h3>The connector</h3>
            <p className="small muted">
              A NEAR wallet connector library, version 0.12.0, pinned in <Src path="web/package.json">package.json</Src>. It is part of this site&apos;s
              code and loads on this page only, when you connect or a saved session is restored. This page draws its own wallet list instead of the
              connector&apos;s.
            </p>
          </div>
          <div className="card-flat stack" style={{ ["--gap" as string]: "6px" }}>
            <h3>A sandbox per wallet</h3>
            <p className="small muted">
              The wallet you pick runs its connector script in a sandboxed iframe: an opaque origin with no access to this page, its cookies or its storage.
              It reaches the page only through the calls its manifest permits: open its own window, keep its session here under its name, talk to its
              extension.
            </p>
          </div>
          <div className="card-flat stack" style={{ ["--gap" as string]: "6px" }}>
            <h3>Pinned, and fenced by the CSP</h3>
            <p className="small muted">
              Three of the four scripts are fixed to a commit; one wallet serves its own, marked in the table. The Content-Security-Policy allows each script
              by its exact path, and the iframe inherits that policy, so its requests are held to the same list.
            </p>
          </div>
        </div>
        <div className={`table-wrap ${cards.cardsWrap}`}>
          <table className={`table ${cards.rtable}`}>
            <caption className="visually-hidden">The wallets offered, their connector scripts and what they contact</caption>
            <thead>
              <tr>
                <th scope="col">Wallet</th>
                <th scope="col">Connector script</th>
                <th scope="col">Also contacts</th>
                <th scope="col">May open</th>
              </tr>
            </thead>
            <tbody>
              {WALLET_INFO.map((w) => {
                const s = hostPath(w.script.url);
                return (
                  <tr key={w.id}>
                    <Cell label="Wallet" lead>
                      <span className={styles.cellStack}>
                        <strong>{w.name}</strong>
                        <span className="xsmall muted">{w.kind}</span>
                      </span>
                    </Cell>
                    <Cell label="Script">
                      <span className={styles.cellStack}>
                        <span className="mono xsmall" style={{ overflowWrap: "anywhere" }}>
                          {s.host}
                          {s.path}
                        </span>
                        <span className="xsmall muted">
                          {w.script.pinned ? `fixed at commit ${w.script.commit}` : "served by the wallet; not pinned"} · {w.script.bytes}
                        </span>
                      </span>
                    </Cell>
                    <Cell label="Contacts" className="small">
                      {w.contacts.map((c) => (
                        <span key={c.origin} className={styles.cellStack}>
                          <span className="mono xsmall">{c.origin.replace(/^https?:\/\//, "")}</span>
                          <span className="xsmall muted">{c.why}</span>
                        </span>
                      ))}
                    </Cell>
                    <Cell label="May open">
                      <span className="mono xsmall">{w.opens.length ? w.opens.map((o) => o.replace(/^https:\/\//, "")).join(", ") : "—"}</span>
                    </Cell>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        <p className="small muted" style={{ maxWidth: "72ch" }}>
          Not offered here: wallets that support only mainnet, wallets that need a relay service, EVM wallets, and USB hardware wallets (this site&apos;s
          Permissions-Policy turns USB off). Wallet windows open outside this page. A browser extension that announces a testnet wallet is listed as well: its
          code already runs in your browser.
        </p>
        <details className={styles.details}>
          <summary>The connect-src entries the wallet connection adds</summary>
          <Command label="connect-src entries" prompt={false}>
            {WALLET_CONNECT_SRC.join("\n")}
          </Command>
        </details>
        <p className="xsmall muted">
          Source: <Src path="web/lib/wallet/manifest.ts">the pinned wallet list</Src>, <Src path="web/lib/wallet/csp.ts">the CSP entries</Src>,{" "}
          <Src path="web/lib/wallet/connector.ts">the connection</Src>, <Src path="web/lib/wallet/tx.ts">the transactions</Src>.
        </p>
      </div>
    </Section>
  );
}
