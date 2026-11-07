import Link from "next/link";
import { Command } from "@/components/ui/Command";
import { Icon } from "@/components/ui/Icon";
import { LiveCheck } from "@/components/ui/LiveCheck";
import { Section } from "@/components/ui/PageHead";
import type { ProgramDetail } from "@/lib/api";
import { config, sourceUrl } from "@/lib/config";
import { tryLabel } from "./helpers";
import styles from "./programs.module.css";

const SEPOLIA = 11155111;

/** A vault the fork-mode manifest deploys: vault 3 (the one JUDGES.md checks), else the first. */
function aVault(d: ProgramDetail): string | null {
  const targets = Array.isArray(d.meta?.targets) ? d.meta.targets : [];
  const v = targets.find(([n]) => n === "vault-3") ?? targets.find(([n]) => /^vault-\d+$/.test(n));
  return v ? v[1] : null;
}

/** For the practice programs: their one rule in plain words, a try in the browser, and fork mode's note. */
export function PracticeSection({ d, id, fork, practiceProgram }: { d: ProgramDetail; id: number; fork: boolean; practiceProgram: number | null }) {
  const vault = fork ? aVault(d) : null;
  const rpc = config.evmRpcs[SEPOLIA]?.[0];
  return (
    <Section
      id="try"
      title="Try it"
      lede="A tutorial: its bug and its proof are public. Anyone can run the whole loop on testnet in a few minutes, with no account and no key; nothing here is a real vulnerability."
    >
      <div className="grid grid-2">
        <div className="card stack">
          <h3 className={styles.cardTitle}>One rule, one textbook bug</h3>
          <p className="small">
            Each of 24 small vaults holds a little ether, and its <code>sweep(to)</code> has no access check: anyone can send a
            vault&apos;s ether anywhere. The proof is one call to your vault; at the anchor the vault loses its ether, so{" "}
            <span className="mono">vault_keeps_its_ether</span> breaks. Each vault is its own target, so each try is its own
            finding.
          </p>
          <div className="row">
            <Link href="/try" className="btn btn-primary btn-sm">
              {tryLabel(id, practiceProgram)} <Icon name="arrow" size={12} />
            </Link>
            <a href={sourceUrl("docs/practice.md")} target="_blank" rel="noreferrer noopener" className="btn btn-sm btn-ghost">
              docs/practice.md <Icon name="external" size={12} />
            </a>
          </div>
          <Command label="CLI command">{"antibody try --name <you>"}</Command>
          <p className="xsmall muted">
            {practiceProgram === id ? (
              <>
                The browser try and the CLI both use this program; each try lands in <a href="#feed">The tries</a>, below. In the
                recorded run a try took about 44 s from vend to payout.
              </>
            ) : practiceProgram !== null ? (
              <>
                The browser try and the CLI use the practice vending contract&apos;s program,{" "}
                <Link href={`/programs/${practiceProgram}`}>program {practiceProgram}</Link>.
              </>
            ) : (
              "The browser try and the CLI use the program the practice vending contract names."
            )}
          </p>
        </div>

        {fork ? (
          <div className="card stack">
            <h3 className={styles.cardTitle}>Fork mode: the vaults exist only inside the verifier</h3>
            <p className="small">
              The manifest commits the transactions that deploy the factory, its 24 vaults and the rule. The verifier runs them in
              its own copy of the chain, on the anchor&apos;s state, before any proof: no staging deployment. So a vault&apos;s
              address has no code on Sepolia, and the proof was still verified, attested on NEAR and paid.
            </p>
            {vault ? (
              <LiveCheck
                check={{ kind: "evm", chainId: SEPOLIA, method: "eth_getCode", params: [vault, "latest"] }}
                expect={{ equals: "0x", says: `"0x": no code at ${vault.slice(0, 10)}… on Sepolia` }}
                cli={
                  rpc
                    ? `curl -s ${rpc} -H 'content-type: application/json' -d '${JSON.stringify({ jsonrpc: "2.0", id: 1, method: "eth_getCode", params: [vault, "latest"] })}'`
                    : undefined
                }
                label="Ask Sepolia for the vault's code"
              />
            ) : null}
            <a href={sourceUrl("docs/fork-mode.md")} target="_blank" rel="noreferrer noopener" className="small">
              How fork mode works: docs/fork-mode.md <Icon name="external" size={11} className={styles.inlineIcon} />
            </a>
          </div>
        ) : (
          <div className="card stack">
            <h3 className={styles.cardTitle}>What a try does</h3>
            <ol className={`small ${styles.list}`}>
              <li>Vends you a practice account for a key your browser (or the CLI) generates; the key cannot withdraw or transfer.</li>
              <li>Builds the tutorial proof against the vault assigned to you, seals it to the program&apos;s key, commits and reveals it.</li>
              <li>Waits for the verdict and its attestation on NEAR.</li>
              <li>Allocates and pays (both permissionless), then closes the practice account: its NEAR goes back to the vending contract.</li>
            </ol>
            <p className="xsmall muted">The payout goes to the vending contract&apos;s credit, which pays the next try&apos;s bond and run.</p>
          </div>
        )}
      </div>
    </Section>
  );
}
