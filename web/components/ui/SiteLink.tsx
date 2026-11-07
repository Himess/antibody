"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import type { ReactNode } from "react";

/**
 * A link to a page on this site, not prefetched (a table can hold dozens). To the page already shown, with
 * another query (an account's view, from /account), it loads the page again: the account page reads
 * `?view=` when it loads, and a client-side navigation would keep it as it was.
 */
export function SiteLink({ href, className, children }: { href: string; className?: string; children: ReactNode }) {
  const path = usePathname() ?? "";
  if (href.split(/[?#]/, 1)[0] === path) {
    return (
      <a href={href} className={className}>
        {children}
      </a>
    );
  }
  return (
    <Link href={href} prefetch={false} className={className}>
      {children}
    </Link>
  );
}
