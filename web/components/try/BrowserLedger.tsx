import type { TrySnapshot } from "@/lib/practice/flow";
import type { HostRole, HostUse } from "./store";
import styles from "./try.module.css";

const ROLE: Record<HostRole, string> = {
  rpc: "NEAR RPC: the views, and the transactions this tab signed",
  archival: "NEAR archival RPC: the program's pinned base witness, an old transaction",
  api: "our API: the live event stream (the verdict and its attestation, as they are indexed), and the run's zero-knowledge proof record once near-t2 is on testnet",
};

/** Before a try, what each host will carry: nothing is signed yet. */
const ROLE_IDLE: Partial<Record<HostRole, string>> = {
  rpc: "NEAR RPC: the vending contract and the program, read before a try; the try's transactions go here too",
};

const ORDER: HostRole[] = ["rpc", "archival", "api"];

/**
 * "Your browser did everything": what was made here, what stayed here, and every host this tab talked to.
 * Before a try (`idle`), the same in the future tense: nothing is signed yet.
 */
export function BrowserLedger({ s, hosts, streamEvents, idle = false }: { s: TrySnapshot | null; hosts: HostUse[]; streamEvents: number; idle?: boolean }) {
  const sorted = [...hosts].sort((a, b) => ORDER.indexOf(a.role) - ORDER.indexOf(b.role) || b.requests - a.requests);
  const signed = s?.signed ?? 0;
  return (
    <div className={`card ${styles.panel}`}>
      <h3>{idle ? "What your browser will do" : "Your browser did everything"}</h3>
      <ul className={styles.bullets}>
        <li>
          <strong>Made here:</strong> the throwaway ed25519 key, the salt, the seal (X25519, HKDF-SHA256, ChaCha20-Poly1305) and{" "}
          {idle ? "every transaction signature of the try" : signed === 1 ? "1 transaction signature" : `${signed} transaction signatures`}.
        </li>
        <li>
          <strong>Kept here:</strong> the practice key, in this tab&apos;s memory and sessionStorage, so a reload can resume the try or close the account. It is never sent;
          only signatures are.
        </li>
        <li>
          <strong>Done elsewhere, as for every run:</strong> OutLayer&apos;s enclave answers the reveal, and our attestation relayer submits the run&apos;s TDX quote,
          which near-antibody verifies on chain.
        </li>
      </ul>
      <div className="stack" style={{ ["--gap" as string]: "8px" }}>
        <h4>This tab talked to</h4>
        {sorted.length ? (
          <ul className={styles.hosts} aria-live="off">
            {sorted.map((h) => (
              <li key={h.host}>
                <span className={styles.hostName}>{h.host}</span>
                <span className="num small">
                  {[
                    h.streams ? `${h.streams === 1 ? "1 stream" : `${h.streams} streams`}, ${streamEvents} ${streamEvents === 1 ? "event" : "events"}` : "",
                    h.requests ? `${h.requests} ${h.requests === 1 ? "request" : "requests"}` : "",
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
                <span className={styles.hostRole}>{(idle ? ROLE_IDLE[h.role] : undefined) ?? ROLE[h.role]}</span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="small muted">Nothing yet: the page reads the vending contract from NEAR when it opens, and the try does the rest.</p>
        )}
        <p className="xsmall muted">
          Counted in this tab since the page opened (checks you run with a button are separate). Besides this site itself, the page&apos;s Content-Security-Policy lets it
          reach only our API and public NEAR and EVM RPCs.
        </p>
      </div>
    </div>
  );
}
