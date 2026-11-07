import type { Metadata } from "next";
import { buildInitial } from "@/components/try/initial";
import { TryApp } from "@/components/try/TryApp";
import { PageHead } from "@/components/ui/PageHead";
import { api } from "@/lib/api";

const description =
  "Run Antibody's loop on NEAR testnet from your browser, with no account: a sealed tutorial proof is committed, verified in OutLayer's TEE and paid, and your browser makes the key, seals the proof and signs every transaction.";

export async function generateMetadata(): Promise<Metadata> {
  return {
    title: "Try it, no account",
    description,
    openGraph: { title: "Try Antibody, no account", description },
    twitter: { title: "Try Antibody, no account", description },
  };
}

export default async function TryPage() {
  // What the page says before a try: the faucet, worker images inside their delay, T2, the stream's cursor.
  const [status, practice, stats, t2] = await Promise.all([api.status(), api.practice(), api.stats(), api.t2()]);
  const initial = buildInitial(status, practice, stats, t2);
  return (
    <>
      <PageHead
        crumbs={[{ href: "/try", label: "Try it" }]}
        eyebrow="Practice program · NEAR testnet"
        title="Try it, no account"
        lede={
          // How long it takes depends on T1 today: the Start card right under this says it, from live data.
          <>
            Commit a sealed proof, have it verified in a TEE and see it paid, from this page. Your browser makes the key, seals the proof and signs every transaction of the
            try.
          </>
        }
      />
      <TryApp initial={initial} />
    </>
  );
}
