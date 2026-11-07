import type { ComponentProps } from "react";
import { LiveCheck } from "@/components/ui/LiveCheck";
import styles from "./build.module.css";

/**
 * LiveCheck inside a box that lets its long CLI command scroll rather than widen the page (its own grid
 * sizes its column to the command's longest line; see `.liveWrap` in build.module.css).
 */
export function Check(props: ComponentProps<typeof LiveCheck>) {
  return (
    <div className={styles.liveWrap}>
      <LiveCheck {...props} />
    </div>
  );
}
