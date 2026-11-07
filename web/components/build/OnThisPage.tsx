import styles from "./build.module.css";

/** In-page links to a page's sections (their ids). */
export function OnThisPage({ items }: { items: { href: string; label: string }[] }) {
  return (
    <nav aria-label="On this page" className={`page ${styles.tocWrap}`}>
      <ul className={styles.toc}>
        {items.map((i) => (
          <li key={i.href}>
            <a href={i.href}>{i.label}</a>
          </li>
        ))}
      </ul>
    </nav>
  );
}
