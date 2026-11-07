import { Fragment, type ReactNode } from "react";
import { safeUrl } from "./url";

/** `code`, [text](https://…) and **bold**: the inline Markdown JUDGES.md's cells and the API's texts use. */
const TOKEN = /(`[^`]+`)|(\[[^\]]+\]\([^)\s]+\))|(\*\*[^*]+\*\*)/g;

/**
 * Inline Markdown as React nodes: code spans, links (external, http or https only) and bold. Nothing is
 * injected as HTML, so text written by anyone renders as text.
 */
export function Md({ text }: { text: string | null | undefined }) {
  if (!text) return null;
  const out: ReactNode[] = [];
  let last = 0;
  let k = 0;
  for (const m of text.matchAll(TOKEN)) {
    const at = m.index ?? 0;
    if (at > last) out.push(<Fragment key={k++}>{text.slice(last, at)}</Fragment>);
    const t = m[0];
    if (t.startsWith("`")) {
      out.push(<code key={k++}>{t.slice(1, -1)}</code>);
    } else if (t.startsWith("[")) {
      const link = /^\[([^\]]+)\]\(([^)\s]+)\)$/.exec(t);
      const href = safeUrl(link?.[2]);
      out.push(
        href ? (
          <a key={k++} href={href} target="_blank" rel="noreferrer noopener">
            {link?.[1]}
          </a>
        ) : (
          <Fragment key={k++}>{link?.[1] ?? t}</Fragment>
        ),
      );
    } else {
      out.push(<strong key={k++}>{t.slice(2, -2)}</strong>);
    }
    last = at + t.length;
  }
  if (last < text.length) out.push(<Fragment key={k++}>{text.slice(last)}</Fragment>);
  return <>{out}</>;
}
