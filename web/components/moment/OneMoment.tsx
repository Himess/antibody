"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Moment } from "@/lib/api/types";
import { laneName, lanesOf, momentEnd, stepsWithCommit, type Lane } from "@/lib/moment";
import { shortHash } from "@/lib/format";
import { Icon } from "../ui/Icon";
import styles from "./OneMoment.module.css";

type Step = ReturnType<typeof stepsWithCommit>[number];

const laneClass = (lane: Lane) => (lane === "near" ? styles.near : lane === "evm:84532" ? styles.base : styles.sepolia);

/**
 * The one moment, from A's commit block: every node a transaction, animated and scrubbable. The recorded
 * seconds are each chain's own block time (the results file's `seconds_from_commit`).
 */
export function OneMoment({
  moment,
  variant = "full",
  autoplay = true,
  initialSpeed = 10,
}: {
  moment: Moment;
  variant?: "hero" | "full" | "present";
  autoplay?: boolean;
  initialSpeed?: 10 | 1;
}) {
  const steps = useMemo(() => stepsWithCommit(moment), [moment]);
  const lanes = useMemo(() => lanesOf(moment), [moment]);
  const end = momentEnd(moment);
  const span = Math.ceil((end + 4) / 10) * 10;
  const [t, setT] = useState(end);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState<10 | 1>(initialSpeed);
  const played = useRef(false);
  const root = useRef<HTMLDivElement>(null);
  const last = useRef<number | null>(null);

  const reduced = useRef(false);
  useEffect(() => {
    reduced.current = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  }, []);

  const play = useCallback(() => {
    setT((cur) => (cur >= end ? 0 : cur));
    last.current = null;
    setPlaying(true);
  }, [end]);

  // Play once when it comes into view (never with reduced motion: the final state shows instead).
  useEffect(() => {
    if (!autoplay || !root.current) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting) && !played.current && !reduced.current) {
          played.current = true;
          setT(0);
          last.current = null;
          setPlaying(true);
        }
      },
      { threshold: 0.35 },
    );
    io.observe(root.current);
    return () => io.disconnect();
  }, [autoplay]);

  useEffect(() => {
    if (!playing) return;
    let raf = 0;
    const tick = (now: number) => {
      const dt = last.current === null ? 0 : (now - last.current) / 1000;
      last.current = now;
      let stop = false;
      setT((cur) => {
        const next = cur + dt * speed;
        if (next >= end) {
          stop = true;
          return end;
        }
        return next;
      });
      if (stop) {
        setPlaying(false);
        return;
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [playing, speed, end]);

  // Presentation mode: Space plays or pauses, R replays, arrows step.
  useEffect(() => {
    if (variant !== "present") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement) return;
      if (e.key === " ") {
        e.preventDefault();
        if (playing) setPlaying(false);
        else play();
      } else if (e.key.toLowerCase() === "r") {
        setT(0);
        last.current = null;
        setPlaying(true);
      } else if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
        setPlaying(false);
        const times = steps.map((s) => s.seconds_exact);
        setT((cur) => {
          if (e.key === "ArrowRight") return times.find((x) => x > cur + 0.01) ?? end;
          return [...times].reverse().find((x) => x < cur - 0.01) ?? 0;
        });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [variant, playing, play, steps, end]);

  const current = [...steps].reverse().find((s) => s.seconds_exact <= t + 1e-6) ?? steps[0];
  const pct = (s: number) => `${(s / span) * 100}%`;
  const signed = steps.find((s) => s.key === "permits_signed");
  const evm = steps.filter((s) => s.chain.startsWith("evm:"));

  return (
    <div ref={root} className={`${styles.root} ${styles[variant]}`}>
      <div className={styles.readout} aria-live="off">
        <div className={styles.clock}>
          <span className={styles.clockLabel}>from A&apos;s commit block</span>
          <span className={`${styles.clockValue} num`}>{t.toFixed(1)}<span className={styles.unit}> s</span></span>
        </div>
        <div className={styles.now}>
          <span className={`${styles.chip} ${laneClass(current.chain)}`}>{laneName(current.chain)}</span>
          <strong>{current.label}</strong>
          {current.detail?.by ? <span className={styles.nowBy}>{String(current.detail.by)}</span> : null}
        </div>
      </div>

      <div className={styles.chart} role="group" aria-label={`Timeline from 0 to ${end.toFixed(1)} seconds after the commit`}>
        <div className={styles.axis} aria-hidden>
          {Array.from({ length: span / 10 + 1 }, (_, i) => i * 10).map((s) => (
            <span key={s} className={styles.tick} style={{ left: pct(s) }}>
              {s} s
            </span>
          ))}
        </div>
        <div className={styles.lanes}>
          <svg className={styles.wires} viewBox={`0 0 1000 ${lanes.length * 100}`} preserveAspectRatio="none" aria-hidden>
            {signed
              ? evm.map((s) => {
                  const li = lanes.indexOf(s.chain);
                  const x1 = (signed.seconds_exact / span) * 1000;
                  const x2 = (s.seconds_exact / span) * 1000;
                  const y1 = 50;
                  const y2 = li * 100 + 50;
                  const lit = t >= s.seconds_exact;
                  const going = t >= signed.seconds_exact;
                  return (
                    <path
                      key={s.key}
                      d={`M${x1},${y1 + 10} C${x1},${(y1 + y2) / 2 + 20} ${x1 + (x2 - x1) * 0.3},${y2} ${x2},${y2}`}
                      className={`${styles.wire} ${lit ? styles.wireLit : going ? styles.wireGoing : ""}`}
                      vectorEffect="non-scaling-stroke"
                    />
                  );
                })
              : null}
          </svg>
          {lanes.map((lane) => {
            const mine = steps.filter((s) => s.chain === lane);
            const first = lane === "near" ? 0 : (signed?.seconds_exact ?? mine[0]?.seconds_exact ?? 0);
            const laneEnd = Math.max(...mine.map((s) => s.seconds_exact));
            const fill = Math.max(0, Math.min(t, laneEnd) - first);
            return (
              <div key={lane} className={`${styles.lane} ${laneClass(lane)}`}>
                <span className={styles.laneName}>{laneName(lane)}</span>
                <div className={styles.track}>
                  <span className={styles.rail} style={{ left: pct(first), width: `calc(${pct(laneEnd)} - ${pct(first)})` }} />
                  <span className={styles.railFill} style={{ left: pct(first), width: pct(fill) }} />
                  {mine.map((s, i) => (
                    <Node key={s.key} step={s} left={pct(s.seconds_exact)} done={t >= s.seconds_exact - 1e-6} place={lane === "near" ? (i % 2 ? "below" : "above") : "above"} />
                  ))}
                </div>
              </div>
            );
          })}
          <span className={styles.playhead} style={{ left: `calc(var(--lane-label) + (100% - var(--lane-label)) * ${t / span})` }} aria-hidden />
        </div>
      </div>

      <div className={styles.controls}>
        <button type="button" className="btn btn-sm" onClick={() => (playing ? setPlaying(false) : play())} aria-label={playing ? "Pause the replay" : "Play the replay"}>
          <Icon name={playing ? "pause" : "play"} size={14} />
          {playing ? "Pause" : t >= end ? "Replay" : "Play"}
        </button>
        <div className={styles.speed} role="radiogroup" aria-label="Replay speed">
          {([10, 1] as const).map((s) => (
            <button key={s} type="button" role="radio" aria-checked={speed === s} className={`btn btn-sm ${speed === s ? styles.on : "btn-ghost"}`} onClick={() => setSpeed(s)}>
              {s === 10 ? "10×" : "Real time"}
            </button>
          ))}
        </div>
        <label className={styles.scrub}>
          <span className="visually-hidden">Seconds from the commit</span>
          <input
            type="range"
            min={0}
            max={end}
            step={0.1}
            value={Number(t.toFixed(1))}
            onChange={(e) => {
              setPlaying(false);
              setT(Number(e.target.value));
            }}
            aria-valuetext={`${t.toFixed(1)} seconds: ${current.label}`}
          />
        </label>
      </div>

      {variant !== "present" ? <StepList steps={steps} t={t} compact={variant === "hero"} /> : null}
    </div>
  );
}

function Node({ step, left, done, place }: { step: Step; left: string; done: boolean; place: "above" | "below" }) {
  const minor = Boolean(step.detail?.minor);
  // The accessible name starts with what the node shows (WCAG 2.5.3), then the chain and the transaction.
  const more = ` on ${laneName(step.chain)}${step.tx ? `, transaction ${step.tx}` : ""}${step.explorer ? ", opens an explorer in a new tab" : ""}`;
  const body = (
    <>
      <span className={styles.dot} aria-hidden />
      <span className={`${styles.nodeText} ${place === "below" ? styles.belowText : styles.aboveText}`}>
        <span className={`${styles.nodeTime} num`}>{step.seconds === 0 ? "0" : step.seconds.toFixed(1)} s</span>{" "}
        <span className={styles.nodeLabel}>{step.label}</span>
        <span className="visually-hidden">{more}</span>
      </span>
    </>
  );
  const cls = `${styles.node} ${done ? styles.done : ""} ${minor ? styles.minor : ""}`;
  return step.explorer ? (
    <a className={cls} style={{ left }} href={step.explorer} target="_blank" rel="noreferrer noopener">
      {body}
    </a>
  ) : (
    <span className={cls} style={{ left }}>
      {body}
    </span>
  );
}

function StepList({ steps, t, compact }: { steps: Step[]; t: number; compact: boolean }) {
  return (
    <ol className={`${styles.list} ${compact ? styles.compact : ""}`} aria-label="Steps, each with its transaction">
      {steps.map((s) => {
        const done = t >= s.seconds_exact - 1e-6;
        return (
          <li key={s.key} className={`${styles.item} ${done ? styles.itemDone : ""} ${s.detail?.minor ? styles.itemMinor : ""}`}>
            <span className={`${styles.itemTime} num`}>{s.seconds === 0 ? "0.0" : s.seconds.toFixed(1)} s</span>
            <span className={styles.itemBody}>
              <span className={styles.itemLabel}>{s.label}</span>
              <span className={`${styles.chip} ${laneClass(s.chain)}`}>{laneName(s.chain)}</span>
            </span>
            {s.explorer ? (
              <a className={`${styles.itemTx} mono`} href={s.explorer} target="_blank" rel="noreferrer noopener" title={s.tx ?? undefined}>
                {shortHash(s.tx, 10)}
                <span className="visually-hidden">{`, ${s.label}: transaction ${s.tx}, opens an explorer in a new tab`}</span>
                <Icon name="external" size={12} />
              </a>
            ) : (
              <span className="muted small">—</span>
            )}
          </li>
        );
      })}
    </ol>
  );
}
