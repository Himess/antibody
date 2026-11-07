"use client";

import Link from "next/link";

/** A page that failed to render: said plainly, with a retry. */
export default function Error({ error, reset }: { error: Error & { digest?: string }; reset: () => void }) {
  return (
    <div className="page page-head" role="alert">
      <p className="eyebrow">Something failed</p>
      <h1>This page could not be rendered</h1>
      <p className="lede">The API or an RPC may be busy. Every value on this site is also readable from NEAR directly.</p>
      {error.digest ? <p className="mono xsmall muted">reference {error.digest}</p> : null}
      <div className="row">
        <button type="button" className="btn btn-sm" onClick={reset}>
          Try again
        </button>
        <Link href="/status" className="btn btn-sm btn-ghost">
          Status
        </Link>
      </div>
    </div>
  );
}
