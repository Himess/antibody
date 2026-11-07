"use client";

import { useEffect, useRef, useState } from "react";
import styles from "./programs.module.css";

/**
 * Links to the page's sections, so a judge can jump to (or share) any one of them. The row stays under
 * the site header while the page scrolls (one scrolling row on phones) and marks the section in view.
 */
export function OnThisPage({ items }: { items: { id: string; label: string }[] }) {
  const nav = useRef<HTMLElement>(null);
  const list = useRef<HTMLOListElement>(null);
  const [active, setActive] = useState<string | null>(null);
  const [stuck, setStuck] = useState(false);
  const ids = items.map((i) => i.id).join(" ");

  // Anchors land below the header and this row, not under them.
  useEffect(() => {
    const el = nav.current;
    if (!el) return;
    const html = document.documentElement;
    const before = html.style.scrollPaddingTop;
    const top = parseFloat(getComputedStyle(el).top) || 0;
    html.style.scrollPaddingTop = `${Math.round(top + el.offsetHeight + 12)}px`;
    return () => {
      html.style.scrollPaddingTop = before;
    };
  }, []);

  useEffect(() => {
    const el = nav.current;
    if (!el || typeof IntersectionObserver === "undefined") return;
    const top = parseFloat(getComputedStyle(el).top) || 0;
    // Below where an anchor lands (see the scroll padding above), so a jump marks the section it jumped to.
    const band = Math.round(top + el.offsetHeight + 24);

    // Stuck: the row is pinned at its sticky top, just under the header.
    const pin = new IntersectionObserver(([e]) => setStuck(e.boundingClientRect.top <= top + 1), {
      rootMargin: `-${Math.round(top) + 1}px 0px 0px 0px`,
      threshold: [1],
    });
    pin.observe(el);

    // In view: the first section crossing a band just under the row.
    const order = ids.split(" ");
    const sections = order.map((id) => document.getElementById(id)).filter((s): s is HTMLElement => s !== null);
    const seen = new Set<string>();
    const spy = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) seen.add(e.target.id);
          else seen.delete(e.target.id);
        }
        const current = order.find((id) => seen.has(id));
        if (current) setActive(current);
        else if (sections[0] && sections[0].getBoundingClientRect().top > band) setActive(null);
      },
      { rootMargin: `-${band}px 0px -60% 0px` },
    );
    sections.forEach((s) => spy.observe(s));
    return () => {
      pin.disconnect();
      spy.disconnect();
    };
  }, [ids]);

  // Keep the current link in view when the row scrolls sideways (phones).
  useEffect(() => {
    const ol = list.current;
    if (!ol || !active) return;
    const a = ol.querySelector<HTMLElement>(`a[href="#${CSS.escape(active)}"]`);
    if (!a) return;
    const left = a.offsetLeft;
    const right = left + a.offsetWidth;
    if (left < ol.scrollLeft || right > ol.scrollLeft + ol.clientWidth) {
      const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      ol.scrollTo({ left: Math.max(0, left - 24), behavior: reduce ? "auto" : "smooth" });
    }
  }, [active]);

  return (
    <nav ref={nav} aria-label="On this page" className={styles.toc} data-stuck={stuck ? "true" : undefined}>
      <ol ref={list}>
        {items.map((i) => (
          <li key={i.id}>
            <a href={`#${i.id}`} aria-current={active === i.id ? "location" : undefined}>
              {i.label}
            </a>
          </li>
        ))}
      </ol>
    </nav>
  );
}
