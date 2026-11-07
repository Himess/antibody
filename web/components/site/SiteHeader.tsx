import Link from "next/link";
import { Wordmark } from "../ui/Logo";
import { MobileNav } from "./MobileNav";
import { NavLinks } from "./NavLinks";
import { ThemeToggle } from "./ThemeToggle";

export function SiteHeader() {
  return (
    <header className="site-header">
      <div className="page site-header-inner">
        <Link href="/" className="brand" aria-label="Antibody, home">
          <Wordmark />
        </Link>
        <nav aria-label="Main" className="nav-desktop">
          <NavLinks />
        </nav>
        <div className="row header-actions">
          <Link href="/account" className="btn btn-sm nav-account">
            Account
          </Link>
          <ThemeToggle />
          <MobileNav />
        </div>
      </div>
    </header>
  );
}
