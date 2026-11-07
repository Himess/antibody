"use client";

import { useState } from "react";
import { Icon } from "./Icon";
import styles from "./ui.module.css";

/** Copies `value` to the clipboard; says so for screen readers. */
export function CopyButton({ value, label = "Copy", className }: { value: string; label?: string; className?: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      type="button"
      className={`${styles.copy} ${className ?? ""}`}
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(value);
          setDone(true);
          setTimeout(() => setDone(false), 1400);
        } catch {
          setDone(false);
        }
      }}
      aria-label={done ? "Copied" : `${label}: ${value.length > 80 ? value.slice(0, 80) + "…" : value}`}
      title={done ? "Copied" : label}
    >
      <Icon name={done ? "check" : "copy"} size={14} />
      <span aria-live="polite" className="visually-hidden">
        {done ? "Copied" : ""}
      </span>
    </button>
  );
}
