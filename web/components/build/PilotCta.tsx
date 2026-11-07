import Link from "next/link";
import type { ReactNode } from "react";
import { Icon } from "@/components/ui/Icon";
import type { Role } from "@/components/pilot/validate";
import styles from "./build.module.css";

/** The call to the pilot and early-access form, with the role the visitor most likely has chosen. */
export function PilotCta({
  id,
  role,
  title,
  children,
  cta,
  secondary,
}: {
  id: string;
  role: Role;
  title: string;
  children: ReactNode;
  cta: string;
  secondary?: { href: string; label: string };
}) {
  return (
    <section id={id} className="page block" aria-labelledby={`${id}-title`}>
      <div className={styles.cta}>
        <div className={styles.ctaText}>
          <p className="eyebrow">Pilot and early access</p>
          <h2 id={`${id}-title`}>{title}</h2>
          <p>{children}</p>
        </div>
        <div className={styles.ctaActions}>
          <Link href={`/pilot?role=${role}`} className="btn btn-primary">
            {cta} <Icon name="arrow" size={14} />
          </Link>
          {secondary ? (
            <Link href={secondary.href} className="btn btn-ghost">
              {secondary.label}
            </Link>
          ) : null}
        </div>
      </div>
    </section>
  );
}
