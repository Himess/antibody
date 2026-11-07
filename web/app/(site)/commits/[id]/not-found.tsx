import Link from "next/link";
import { PageHead } from "@/components/ui/PageHead";

export default function CommitNotFound() {
  return (
    <>
      <PageHead
        crumbs={[{ href: "/programs", label: "Programs" }]}
        eyebrow="Commit"
        title="No such commit"
        lede="near-antibody has no commit with this id. Commit ids are numbers, counted across every program from 0; a commit appears here as soon as its transaction is final."
      />
      <div className="page row">
        <Link href="/programs" className="btn btn-primary">
          Programs and their commits
        </Link>
        <Link href="/try" className="btn">
          Make one yourself
        </Link>
      </div>
    </>
  );
}
