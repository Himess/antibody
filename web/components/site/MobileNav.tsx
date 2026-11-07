"use client";

import { useEffect, useRef, useState } from "react";
import { usePathname } from "next/navigation";
import Link from "next/link";
import { Icon } from "../ui/Icon";
import { NavLinks } from "./NavLinks";

/** The menu on small screens: a disclosure that closes on Escape or navigation. */
export function MobileNav() {
  const path = usePathname();
  // Open for the page it was opened on: navigating closes it with no effect.
  const [openOn, setOpenOn] = useState<string | null>(null);
  const open = openOn === path;
  const btn = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpenOn(null);
        btn.current?.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);
  return (
    <div className="nav-mobile">
      <button
        ref={btn}
        type="button"
        className="btn btn-ghost btn-sm icon-btn"
        aria-expanded={open}
        aria-controls="mobile-menu"
        onClick={() => setOpenOn(open ? null : path)}
      >
        <Icon name={open ? "x" : "menu"} size={18} />
        <span className="visually-hidden">{open ? "Close menu" : "Open menu"}</span>
      </button>
      {open ? (
        <div id="mobile-menu" className="mobile-menu">
          <nav aria-label="Main (mobile)">
            <NavLinks onNavigate={() => setOpenOn(null)} />
            <ul className="nav-list">
              <li>
                <Link href="/status" onClick={() => setOpenOn(null)}>
                  Status
                </Link>
              </li>
              <li>
                <Link href="/account" onClick={() => setOpenOn(null)}>
                  Account
                </Link>
              </li>
            </ul>
          </nav>
        </div>
      ) : null}
    </div>
  );
}
