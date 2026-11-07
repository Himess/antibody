"use client";

import { useState } from "react";
import { Cell } from "@/components/programs/Cell";
import cards from "@/components/programs/programs.module.css";
import { Hash } from "@/components/ui/Hash";
import { Icon } from "@/components/ui/Icon";
import { Pill } from "@/components/ui/Pill";
import { checkContracts } from "@/lib/wallet/account";
import { RELEASE } from "@/lib/wallet/contracts";
import type { ContractState } from "@/lib/wallet/types";
import type { Load } from "./shared";
import styles from "./wallet.module.css";

/**
 * The code at the two contracts this page sends transactions to, read by the browser from NEAR and compared
 * with the release record (deployments/near-testnet.json). The same check runs before every review.
 */
export function ContractCheck() {
  const [state, setState] = useState<Load<ContractState[]> | null>(null);
  const run = () => {
    setState({ status: "loading" });
    checkContracts()
      .then((data) => setState({ status: "ok", data }))
      .catch((e: unknown) => setState({ status: "error", error: e instanceof Error ? e.message : String(e) }));
  };
  const shown: ContractState[] =
    state?.status === "ok"
      ? state.data
      : [RELEASE.antibody, RELEASE.token].map((c) => ({ account: c.account, role: c.role, expected: c.codeHash, actual: null, matches: null, fullAccessKeys: null }));
  return (
    <div className="stack">
      <div className={`table-wrap ${cards.cardsWrap}`}>
        <table className={`table ${cards.rtable}`}>
          <caption className="visually-hidden">The contracts this page sends transactions to</caption>
          <thead>
            <tr>
              <th scope="col">Contract</th>
              <th scope="col">Code in the release record</th>
              <th scope="col">On NEAR now</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((c) => (
              <tr key={c.account}>
                <Cell label="Contract" lead>
                  <span className={styles.cellStack}>
                    <span className="mono">{c.account}</span>
                    <span className="xsmall muted">{c.role}</span>
                  </span>
                </Cell>
                <Cell label="Release record">
                  <Hash value={c.expected} head={12} label={`${c.account} code hash`} />
                </Cell>
                <Cell label="On NEAR now">
                  <div aria-live="polite">
                    {state === null ? (
                      <span className="muted small">not checked yet</span>
                    ) : state.status === "loading" ? (
                      <span className="muted small">reading…</span>
                    ) : c.error ? (
                      <Pill tone="warn" size="sm">
                        could not read: {c.error.slice(0, 80)}
                      </Pill>
                    ) : (
                      <span className={styles.cellStack}>
                        <span>
                          {c.matches ? (
                            <Pill tone="ok" size="sm">
                              same code
                            </Pill>
                          ) : (
                            <Pill tone="bad" size="sm">
                              different code
                            </Pill>
                          )}{" "}
                          {c.fullAccessKeys === 0 ? (
                            <Pill tone="ok" size="sm" icon="lock">
                              no full-access key
                            </Pill>
                          ) : c.fullAccessKeys !== null ? (
                            <Pill tone="warn" size="sm">
                              {c.fullAccessKeys} full-access key{c.fullAccessKeys === 1 ? "" : "s"}
                            </Pill>
                          ) : (
                            <span className="xsmall muted">its owner may upgrade it</span>
                          )}
                        </span>
                        {c.actual && c.actual !== c.expected ? <Hash value={c.actual} head={12} label="code hash on NEAR" /> : null}
                      </span>
                    )}
                  </div>
                </Cell>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="row">
        {/* aria-disabled, not disabled, while it works: a disabled button would drop keyboard focus to the page. */}
        <button
          type="button"
          className="btn btn-sm"
          onClick={() => {
            if (state?.status !== "loading") void run();
          }}
          aria-disabled={state?.status === "loading" || undefined}
          aria-busy={state?.status === "loading"}
        >
          <Icon name="play" size={13} /> {state ? "Check again" : "Check the contracts on NEAR"}
        </button>
        <span className="xsmall muted">view_account and view_access_key_list, read by your browser (no key, no transaction)</span>
      </div>
      {state?.status === "error" ? <p className="xsmall mono muted">{state.error}</p> : null}
    </div>
  );
}
