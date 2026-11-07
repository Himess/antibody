import type { Metadata } from "next";
import { PilotAside } from "@/components/pilot/PilotAside";
import { PilotForm } from "@/components/pilot/PilotForm";
import styles from "@/components/pilot/pilot.module.css";
import { isRole } from "@/components/pilot/validate";
import { PageHead } from "@/components/ui/PageHead";

export async function generateMetadata(): Promise<Metadata> {
  return {
    title: "Pilot and early access",
    description:
      "Ask for an Antibody pilot for your project, or early access as a hunter or an AI agent's operator. The request goes from your browser to our API and is stored on our server only, never published.",
  };
}

export default async function PilotPage({ searchParams }: { searchParams: Promise<{ role?: string | string[] }> }) {
  const sp = await searchParams;
  const asked = Array.isArray(sp.role) ? sp.role[0] : sp.role;
  const role = isRole(asked) ? asked : undefined;
  return (
    <>
      <PageHead
        crumbs={[{ href: "/pilot", label: "Pilot and early access" }]}
        eyebrow="Pilot and early access"
        title="Ask for a pilot or early access"
        lede={
          <>
            A pilot for a project that wants pay on proof next to its existing bounty, or early access for a hunter or an AI
            agent&apos;s operator. Tell us who you are and how to reach you; we reply about it.
          </>
        }
      />
      <div className={`page ${styles.layout}`}>
        <div className="card">
          <PilotForm key={role ?? "none"} defaultRole={role} />
        </div>
        <PilotAside />
      </div>
    </>
  );
}
