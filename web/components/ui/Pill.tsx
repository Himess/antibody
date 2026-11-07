import type { ReactNode } from "react";
import type { Tone } from "@/lib/format";
import { Icon, type IconName } from "./Icon";
import styles from "./ui.module.css";

const toneIcon: Partial<Record<Tone, IconName>> = {
  ok: "check",
  bad: "alert",
  warn: "alert",
  pending: "clock",
  info: "info",
  t1: "cpu",
  t2: "sigma",
};

/** A status pill: colour plus an icon plus words, never colour alone. */
export function Pill({
  tone = "neutral",
  icon,
  children,
  title,
  size = "md",
}: {
  tone?: Tone;
  icon?: IconName | null;
  children: ReactNode;
  title?: string;
  size?: "sm" | "md";
}) {
  const ic = icon === null ? undefined : (icon ?? toneIcon[tone]);
  return (
    <span className={`${styles.pill} ${styles[`tone-${tone}`]} ${size === "sm" ? styles.pillSm : ""}`} title={title}>
      {ic ? <Icon name={ic} size={size === "sm" ? 12 : 13} strokeWidth={2} /> : null}
      <span>{children}</span>
    </span>
  );
}
