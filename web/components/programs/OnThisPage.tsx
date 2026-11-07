import styles from "./programs.module.css";

/** Links to the page's sections, so a judge can jump to (or share) any one of them. */
export function OnThisPage({ items }: { items: { id: string; label: string }[] }) {
  return (
    <nav aria-label="On this page" className={`page ${styles.toc}`}>
      <ol>
        {items.map((i) => (
          <li key={i.id}>
            <a href={`#${i.id}`}>{i.label}</a>
          </li>
        ))}
      </ol>
    </nav>
  );
}
