import type { Metadata } from "next";
import Link from "next/link";
import { OneMoment } from "@/components/moment/OneMoment";
import { LogoMark } from "@/components/ui/Logo";
import { utc, wnear } from "@/lib/format";
import { chainsWord, committer, getMoments, latestMoment, pausesOf, possessive } from "@/lib/moment";
import styles from "./present.module.css";

export const metadata: Metadata = {
  title: "The one moment, presentation mode",
  description: "The one moment, full screen: A's Critical verified, T1-confirmed, paid, and both chains paused, from the recorded testnet run.",
  robots: { index: false },
};

/**
 * Presentation mode for the video and the stage: 16:9, no chrome, readable at 1080p, driven by the
 * recorded run. Space or a tap plays or pauses, Home replays, arrows step, Enter goes full screen; ?speed=1 plays in real time.
 */
export default async function Present({ searchParams }: { searchParams: Promise<{ speed?: string; run?: string }> }) {
  const sp = await searchParams;
  const { data } = await getMoments();
  const m = data.moments.find((x) => x.id === sp.run) ?? latestMoment(data);
  const paused = pausesOf(m);
  const who = possessive(committer(m).name);
  return (
    <div className={styles.stage} data-theme-lock="dark">
      <div className={styles.frame}>
        <header className={styles.top}>
          <span className={styles.brand}>
            <LogoMark size={34} /> Antibody
          </span>
          <span className={styles.tagline}>We don&apos;t read reports. We run them.</span>
        </header>
        <div className={styles.title}>
          <p className={styles.kicker}>The one moment · recorded on NEAR testnet · {utc(m.commit.block_ts_ns, "ns")}</p>
          <h1>
            {who} sealed proof of a Critical: verified, paid {wnear(m.paid_raw)}, and the protocol paused on {chainsWord(paused.length)}.
          </h1>
        </div>
        <OneMoment moment={m} variant="present" initialSpeed={sp.speed === "1" ? 1 : 10} />
        <footer className={styles.foot}>
          <span>
            Every node links to its transaction. NEAR steps are timed at their blocks, each pause when the relayer got its chain&apos;s
            receipt. Testnet: wNEAR has no value; Acme Lend is our deliberately vulnerable fixture.
          </span>
          <span className={styles.keys}>
            <kbd>Space</kbd> or tap: play · <kbd>Home</kbd> replay · <kbd>←</kbd> <kbd>→</kbd> step · <kbd>Enter</kbd> full screen ·{" "}
            <Link href="/moment">exit</Link>
          </span>
        </footer>
      </div>
    </div>
  );
}
