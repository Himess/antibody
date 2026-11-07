import Link from "next/link";
import { Icon } from "@/components/ui/Icon";
import styles from "./pilot.module.css";

/** What happens to a request, and the promise about the details in it. */
export function PilotAside() {
  return (
    <aside className={styles.aside} aria-label="What happens next, and your details">
      <div className="card stack">
        <h2>What happens next</h2>
        <ol className={styles.next} role="list">
          <li>
            <span className={styles.nextNo} aria-hidden>
              01
            </span>
            <span>Our API checks your request as this form does, and keeps it in a private file on our server.</span>
          </li>
          <li>
            <span className={styles.nextNo} aria-hidden>
              02
            </span>
            <span>We get a notice of it. Nothing else: no mailing list, no account.</span>
          </li>
          <li>
            <span className={styles.nextNo} aria-hidden>
              03
            </span>
            <span>We reply to the contact you gave, about a pilot or early access.</span>
          </li>
        </ol>
      </div>

      <div className="card stack">
        <h2>Your details</h2>
        <ul className={styles.promise}>
          <li>
            <Icon name="lock" size={15} />
            <span>Stored on our server only: never published, and never in the repository.</span>
          </li>
          <li>
            <Icon name="shield" size={15} />
            <span>Sent from your browser straight to our API: no third-party form service.</span>
          </li>
          <li>
            <Icon name="check" size={15} />
            <span>Used only to reply to you about a pilot or early access.</span>
          </li>
          <li>
            <Icon name="clock" size={15} />
            <span>Five requests an hour from one network, so the form stays usable.</span>
          </li>
        </ul>
      </div>

      <div className="card-flat stack" style={{ ["--gap" as string]: "10px" }}>
        <h2>Before you write</h2>
        <ul className={styles.links}>
          <li>
            <Link href="/projects">Open a program: the quickstart</Link>
          </li>
          <li>
            <Link href="/agents">For hunters and agents: the CLI and MCP</Link>
          </li>
          <li>
            <Link href="/try">Try it, no account</Link>
          </li>
        </ul>
      </div>
    </aside>
  );
}
