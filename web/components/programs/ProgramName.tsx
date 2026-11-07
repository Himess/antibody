import styles from "./programs.module.css";

/** A program's name with each hyphenated word kept whole: "pre-launch" never breaks at its hyphen. */
export function ProgramName({ name }: { name: string }) {
  const parts = name.split(/(\S+-\S+)/);
  return (
    <>
      {parts.map((p, i) =>
        i % 2 ? (
          <span key={i} className={styles.nowrap}>
            {p}
          </span>
        ) : (
          p
        ),
      )}
    </>
  );
}
