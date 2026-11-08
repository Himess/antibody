import Link from "next/link";
import { config } from "@/lib/config";
import { LogoMark } from "../ui/Logo";
import { FOOTER } from "./nav";

export function SiteFooter() {
  return (
    <footer className="site-footer">
      <div className="page">
        <div className="footer-grid">
          <div className="stack footer-about">
            <span className="row footer-brand">
              <LogoMark size={20} />
              <strong>Antibody</strong>
            </span>
            <p className="small muted">Pay on proof. Pause on proof. Ship only what survived.</p>
            <p className="xsmall muted">
              Everything here runs on NEAR testnet, Sepolia and Base Sepolia. wNEAR on testnet has no value. Acme Lend and the
              practice vaults are deliberately vulnerable test fixtures, with textbook bugs labelled in their code.
            </p>
          </div>
          {FOOTER.map((col) => (
            <div key={col.title}>
              <p className="eyebrow footer-title">{col.title}</p>
              <ul className="footer-links">
                {col.links.map((l) => (
                  <li key={l.href}>
                    <Link href={l.href}>{l.label}</Link>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
        <div className="footer-base small muted">
          <span>MIT licensed</span>
          <a href={config.repo} target="_blank" rel="noreferrer noopener">
            Source on GitHub
          </a>
          <a href={`${config.repo}/blob/main/JUDGES.md`} target="_blank" rel="noreferrer noopener">
            JUDGES.md
          </a>
          <span className="footer-api">
            API{" "}
            <a href={`${config.apiShown}/v1/status`} target="_blank" rel="noreferrer noopener" className="mono xsmall">
              /v1/status
            </a>
          </span>
        </div>
      </div>
    </footer>
  );
}
