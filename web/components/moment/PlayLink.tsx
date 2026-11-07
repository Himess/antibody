"use client";

import type { ReactNode } from "react";
import { PLAY_EVENT } from "@/lib/moment";

/** A link to the one-moment panel that also replays it (the panel keeps the final state when motion is reduced). */
export function PlayLink({ href, className, children }: { href: string; className?: string; children: ReactNode }) {
  return (
    <a href={href} className={className} onClick={() => window.dispatchEvent(new Event(PLAY_EVENT))}>
      {children}
    </a>
  );
}
