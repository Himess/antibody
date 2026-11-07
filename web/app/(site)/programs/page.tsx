import type { Metadata } from "next";
import Link from "next/link";
import { Suspense } from "react";
import { Icon } from "@/components/ui/Icon";
import { PageHead } from "@/components/ui/PageHead";
import { DirectorySkeleton, ProgramsDirectory } from "@/components/programs/ProgramsDirectory";

export const metadata: Metadata = {
  title: "Programs",
  description:
    "Every program on near-antibody, NEAR testnet: its kind, phase, round, anchor check, escrow coverage, largest payout, commits and payouts. Acme Lend is our deliberately vulnerable fixture; the practice programs are tutorials.",
};

export default function ProgramsPage() {
  return (
    <>
      <PageHead
        crumbs={[{ href: "/programs", label: "Programs" }]}
        eyebrow="Rules as code, escrow on NEAR"
        title="Programs"
        lede={
          <>
            A program is a project&apos;s rules as code, an escrow on NEAR and an anchor block. A proof that breaks a rule at the
            anchor is paid by its terms. Everything here is on testnet: Acme Lend is our deliberately vulnerable fixture, the
            practice programs are tutorials, and wNEAR on testnet has no value.
          </>
        }
      >
        <Link href="/try" className="btn btn-sm">
          Try the practice program <Icon name="arrow" size={12} />
        </Link>
        <Link href="/projects" className="btn btn-sm btn-ghost">
          Open a program
        </Link>
      </PageHead>
      <Suspense fallback={<DirectorySkeleton />}>
        <ProgramsDirectory />
      </Suspense>
    </>
  );
}
