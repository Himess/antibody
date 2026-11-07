"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { NAV } from "./nav";

export function NavLinks({ onNavigate }: { onNavigate?: () => void }) {
  const path = usePathname() ?? "/";
  return (
    <ul className="nav-list">
      {NAV.map((n) => {
        const active = path === n.href || path.startsWith(n.href + "/");
        return (
          <li key={n.href}>
            <Link href={n.href} aria-current={active ? "page" : undefined} onClick={onNavigate}>
              {n.label}
            </Link>
          </li>
        );
      })}
    </ul>
  );
}
