import Link from "next/link";
import { PageHead } from "@/components/ui/PageHead";

export default function RunNotFound() {
  return (
    <>
      <PageHead
        crumbs={[{ href: "/programs", label: "Programs" }]}
        eyebrow="Verification run"
        title="No such run"
        lede="near-antibody has no run with this id. Run ids are numbers, counted from 0 across every program: activations, verifications, re-runs and release wraps alike."
      />
      <div className="page row">
        <Link href="/programs" className="btn btn-primary">
          Programs and their commits
        </Link>
      </div>
    </>
  );
}
