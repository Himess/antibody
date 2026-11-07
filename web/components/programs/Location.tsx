import { Icon } from "@/components/ui/Icon";
import { TxLink } from "@/components/ui/TxLink";
import { parseLocation } from "./helpers";
import styles from "./programs.module.css";

/**
 * Where a manifest or a witness lives: an https link, or an upload carried by a NEAR transaction
 * (`near-tx:<hash>:<signer>`), shown as that transaction. Its sha256 sits next to it wherever it is used.
 */
export function LocationView({ location, what }: { location?: string | null; what: string }) {
  const l = parseLocation(location);
  if (!l) return <span className="muted small">no location published</span>;
  if (l.kind === "https")
    return (
      <a href={l.url} target="_blank" rel="noreferrer noopener" className="small">
        {what}, as uploaded <Icon name="external" size={11} className={styles.inlineIcon} />
      </a>
    );
  if (l.kind === "near-tx")
    return (
      <span className="small">
        Uploaded in NEAR transaction <TxLink hash={l.tx} head={10} />
        {l.signer ? (
          <>
            {" "}
            by <span className="mono">{l.signer}</span>
          </>
        ) : null}
      </span>
    );
  return <span className={styles.full}>{l.text}</span>;
}
