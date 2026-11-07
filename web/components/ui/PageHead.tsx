import Link from "next/link";
import type { ReactNode } from "react";

/** A page's title block: breadcrumbs, eyebrow, title, a lede, and optional actions or badges. */
export function PageHead({
  eyebrow,
  title,
  lede,
  crumbs,
  children,
}: {
  eyebrow?: string;
  title: ReactNode;
  lede?: ReactNode;
  crumbs?: { href: string; label: string }[];
  children?: ReactNode;
}) {
  return (
    <header className="page page-head">
      {crumbs?.length ? (
        <nav aria-label="Breadcrumb">
          <ol className="breadcrumbs">
            <li>
              <Link href="/">Antibody</Link>
            </li>
            {crumbs.map((c) => (
              <li key={c.href}>
                <Link href={c.href}>{c.label}</Link>
              </li>
            ))}
          </ol>
        </nav>
      ) : null}
      {eyebrow ? <p className="eyebrow">{eyebrow}</p> : null}
      <h1>{title}</h1>
      {lede ? <div className="lede">{lede}</div> : null}
      {children ? <div className="row">{children}</div> : null}
    </header>
  );
}

/** A titled section with an anchor id, used down every page. */
export function Section({ id, title, eyebrow, lede, children, actions }: { id: string; title: ReactNode; eyebrow?: string; lede?: ReactNode; children: ReactNode; actions?: ReactNode }) {
  return (
    <section id={id} className="page block" aria-labelledby={`${id}-title`}>
      <div className="block-head">
        <div className="stack" style={{ ["--gap" as string]: "6px" }}>
          {eyebrow ? <p className="eyebrow">{eyebrow}</p> : null}
          <h2 id={`${id}-title`}>
            <a href={`#${id}`} className="anchor">
              {title}
            </a>
          </h2>
          {lede ? <div className="block-lede">{lede}</div> : null}
        </div>
        {actions ? <div className="row">{actions}</div> : null}
      </div>
      {children}
    </section>
  );
}
