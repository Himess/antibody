import type { Metadata } from "next";
import { StatusLive } from "@/components/checks/StatusLive";
import { judges, runRole } from "@/components/checks/judges";
import { PageHead } from "@/components/ui/PageHead";
import { api } from "@/lib/api";

export async function generateMetadata(): Promise<Metadata> {
  return {
    title: "Status",
    description: "Antibody's own health on testnet: the services' state files and view calls, refreshed in your browser every 30 seconds.",
  };
}

export default async function StatusPage() {
  const first = await api.status();
  // This run's programs by id ("Acme Lend", "practice", "fork mode"); the others are earlier runs'.
  const runRoles: Record<string, string> = {};
  for (const [role, id] of Object.entries(judges.programs ?? {})) {
    const name = runRole(role);
    if (name && typeof id === "number") runRoles[String(id)] = name;
  }
  return (
    <>
      <PageHead
        crumbs={[{ href: "/status", label: "Status" }]}
        eyebrow="Status"
        title="Our own health"
        lede={<p>We show our own health: these are the services&apos; own state files and view calls.</p>}
      />
      <StatusLive initial={first} runRoles={runRoles} />
    </>
  );
}
