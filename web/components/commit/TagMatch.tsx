import type { ReactNode } from "react";
import { CopyButton } from "@/components/ui/CopyButton";
import { Pill } from "@/components/ui/Pill";
import styles from "./commit.module.css";

export interface TagSide {
  title: ReactNode;
  caption?: ReactNode;
  slot: number;
  slotLabel: string;
  tag: string;
}

/**
 * Two keys side by side, and whether they are equal. A tag is a MAC of the rule and the entry point under
 * the program's key, which only the TEE holds: equal tags say "same rule, same entry point" and nothing
 * more (two bugs behind one entry point share a tag until a fix tells them apart, README: Limitations).
 * The hex is laid out in the same grid of eight-character groups on both sides, so the eye can compare it.
 */
export function TagMatch({ left, right, label }: { left: TagSide; right: TagSide; label: string }) {
  const equal = left.slot === right.slot && left.tag.toLowerCase() === right.tag.toLowerCase();
  return (
    <figure className={styles.tagMatch} aria-label={label}>
      <div className={styles.tagSides}>
        <Side side={left} />
        <div className={`${styles.tagEq} ${equal ? styles.tagEqOk : styles.tagEqBad}`} aria-hidden="true">
          <span>{equal ? "=" : "≠"}</span>
        </div>
        <Side side={right} />
      </div>
      <figcaption className={styles.tagVerdict}>
        {equal ? <Pill tone="ok">equal: same slot, same tag</Pill> : <Pill tone="bad">not equal</Pill>}
        <span>
          {equal
            ? "Equal tags: the same rule broken through the same entry point. That is all a tag says; a different bug behind the same entry point shows up at the fix (rounds)."
            : "Different keys: these proofs are about different rules or entry points."}
        </span>
      </figcaption>
    </figure>
  );
}

function Side({ side }: { side: TagSide }) {
  const groups = side.tag.toLowerCase().match(/.{1,8}/g) ?? [side.tag];
  return (
    <div className={styles.tagCard}>
      <div className={styles.tagHead}>{side.title}</div>
      {side.caption ? <div className="xsmall muted">{side.caption}</div> : null}
      <dl className={styles.tagKv}>
        <dt>Rule</dt>
        <dd>{side.slotLabel}</dd>
        <dt>Tag</dt>
        <dd>
          <code className={styles.tagHex}>
            {groups.map((g, i) => (
              <span key={i}>{g}</span>
            ))}
          </code>
          <CopyButton value={side.tag} label="Copy the tag" />
        </dd>
      </dl>
    </div>
  );
}
