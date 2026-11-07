import Link from "next/link";

/** A program id near-antibody does not hold (or not a program id at all). */
export default function ProgramNotFound() {
  return (
    <div className="page page-head">
      <p className="eyebrow">404 · Programs</p>
      <h1>No such program</h1>
      <p className="lede">
        near-antibody holds no program with this id. Programs are numbered from 0; this run&apos;s are 5 (Acme Lend, our fixture), 6
        (practice) and 7 (practice in fork mode).
      </p>
      <div className="row">
        <Link href="/programs" className="btn btn-sm">
          Every program
        </Link>
        <Link href="/programs/5" className="btn btn-sm btn-ghost">
          Program 5, Acme Lend
        </Link>
        <Link href="/try" className="btn btn-sm btn-ghost">
          Try the practice program
        </Link>
      </div>
    </div>
  );
}
