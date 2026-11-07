import type { Metadata } from "next";
import { PageHead } from "@/components/ui/PageHead";
import { Pill } from "@/components/ui/Pill";
import { AccountApp, type ApiContracts } from "@/components/wallet/AccountApp";
import { HowItConnects, ReadProofs, SignableCalls } from "@/components/wallet/StaticSections";
import { api } from "@/lib/api";

export const metadata: Metadata = {
  title: "Your account",
  description:
    "Connect a NEAR testnet wallet: your credit at near-antibody, your commits and payouts, your agents, and the transactions to change them, each reviewed before your wallet signs. With no wallet, read any account. The page keeps no key and never asks for a message signature.",
};

/**
 * The account page. The server part renders what needs no wallet (what the page can sign, how the wallet
 * connects, why reading paid proofs stays in the CLI); the client part connects a wallet and reads the
 * account, or reads any account read-only (`?view=<id>`). The contracts the page signs for are fixed in its
 * code; /health is read only to compare.
 */
export default async function AccountPage() {
  const health = await api.health();
  const apiContracts: ApiContracts = health.ok ? { antibody: health.data?.contracts?.antibody ?? null, token: health.data?.contracts?.token ?? null } : null;
  return (
    <>
      <PageHead
        crumbs={[{ href: "/account", label: "Account" }]}
        eyebrow="Account · NEAR testnet"
        title="Your account"
        lede="Your credit at near-antibody, your commits and payouts, your agents, and the transactions that change them, signed by your own wallet after you review each one."
      >
        {health.ok ? null : (
          <Pill tone="warn" size="sm" title={health.error}>
            API unavailable: the page reads NEAR directly where it can
          </Pill>
        )}
      </PageHead>
      <AccountApp apiContracts={apiContracts} />
      <SignableCalls />
      <HowItConnects />
      <ReadProofs />
    </>
  );
}
