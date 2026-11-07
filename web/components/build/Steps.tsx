import type { ReactNode } from "react";
import { Command } from "@/components/ui/Command";
import styles from "./build.module.css";

/** A command to copy, with an optional line saying what it does. */
export type StepCommand = string | { cmd: string; note?: ReactNode };

export type Step = {
  id?: string;
  title: ReactNode;
  body?: ReactNode;
  commands?: StepCommand[];
  after?: ReactNode;
};

/**
 * Numbered steps, each a title, a few lines and the exact commands. The list is announced as an ordered
 * list (its numbers are drawn for the eye only).
 */
export function Steps({ steps, label }: { steps: Step[]; label: string }) {
  return (
    <ol className={styles.steps} aria-label={label} role="list">
      {steps.map((s, i) => (
        <li key={s.id ?? i} id={s.id} className={styles.step}>
          <span className={styles.stepNo} aria-hidden>
            {String(i + 1).padStart(2, "0")}
          </span>
          <div className={styles.stepBody}>
            <h3 className={styles.stepTitle}>{s.title}</h3>
            {s.body ? <div className={styles.stepText}>{s.body}</div> : null}
            {s.commands?.length ? (
              <div className={styles.cmds}>
                {s.commands.map((c, j) => {
                  const cmd = typeof c === "string" ? c : c.cmd;
                  const note = typeof c === "string" ? null : c.note;
                  return (
                    <div key={j} className={styles.cmd}>
                      <Command>{cmd}</Command>
                      {note ? <p className={styles.cmdNote}>{note}</p> : null}
                    </div>
                  );
                })}
              </div>
            ) : null}
            {s.after}
          </div>
        </li>
      ))}
    </ol>
  );
}
