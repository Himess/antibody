import Link from "next/link";

export default function NotFound() {
  return (
    <div className="page page-head">
      <p className="eyebrow">404</p>
      <h1>Nothing here</h1>
      <p className="lede">This page does not exist, or the API does not know this id. Programs, commits and runs are numbered from 0 on near-antibody.</p>
      <div className="row">
        <Link href="/" className="btn btn-sm">
          Home
        </Link>
        <Link href="/programs" className="btn btn-sm btn-ghost">
          Programs
        </Link>
      </div>
    </div>
  );
}
