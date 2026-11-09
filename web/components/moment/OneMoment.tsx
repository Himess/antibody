"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Moment } from "@/lib/api/types";
import { cap, shortHash } from "@/lib/format";
import { committer, laneName, lanesOf, momentEnd, PLAY_EVENT, possessive, sharedTxNote, stepsWithCommit, type Lane, type Step } from "@/lib/moment";
import { Icon } from "../ui/Icon";
import styles from "./OneMoment.module.css";

const laneClass = (lane: Lane) => (lane === "near" ? styles.near : lane === "evm:84532" ? styles.base : styles.sepolia);

const timeText = (s: Step) => `${s.seconds.toFixed(1)} s`;

/** What a node's link says after what it shows (WCAG 2.5.3): the chain, the transaction, where it goes. */
const more = (s: Step) =>
  ` on ${laneName(s.chain)}${s.tx ? `, transaction ${s.tx}` : ""}${s.explorer ? ", opens an explorer in a new tab" : ""}`;

/**
 * The one moment, from the commit's block: every node a transaction, animated and scrubbable. The seconds
 * are the recorded ones (`/v1/moments`): NEAR steps at their blocks, each pause when the relayer got its
 * chain's receipt. Wide screens get lanes left to right; phones get the same lanes top to bottom.
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
  const who = possessive(committer(moment).name);
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

  // "Watch the one moment" replays it from the commit; with reduced motion it shows the final state.
  useEffect(() => {
    if (variant === "present") return;
    const onPlay = () => {
      played.current = true;
      last.current = null;
      if (reduced.current) {
        setPlaying(false);
        setT(end);
        return;
      }
      setT(0);
      setPlaying(true);
    };
    window.addEventListener(PLAY_EVENT, onPlay);
    return () => window.removeEventListener(PLAY_EVENT, onPlay);
  }, [variant, end]);

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

  // Presentation mode: Space or a click plays or pauses, Home replays, arrows step, Enter toggles full screen.
  // No bare letter keys (WCAG 2.1.4): a word said to speech input could set one off.
  useEffect(() => {
    if (variant !== "present") return;
    const toggle = () => {
      if (playing) setPlaying(false);
      else play();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.ctrlKey || e.metaKey || e.altKey) return;
      if (e.key === " ") {
        e.preventDefault();
        toggle();
      } else if (e.key === "Home") {
        e.preventDefault();
        setT(0);
        last.current = null;
        setPlaying(true);
      } else if (e.key === "Enter" && !(e.target instanceof Element && e.target.closest("a, button, summary"))) {
        // Enter on a focused link or control keeps its own meaning (the exit link, a node's explorer link).
        if (document.fullscreenElement) void document.exitFullscreen().catch(() => {});
        else if (document.documentElement.requestFullscreen) void document.documentElement.requestFullscreen().catch(() => {});
      } else if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
        setPlaying(false);
        const times = steps.map((s) => s.seconds_exact);
        setT((cur) => {
          if (e.key === "ArrowRight") return times.find((x) => x > cur + 0.01) ?? end;
          return [...times].reverse().find((x) => x < cur - 0.01) ?? 0;
        });
      }
    };
    // A click or a tap anywhere on the stage, except on a link or a control.
    const onClick = (e: MouseEvent) => {
      if (e.target instanceof Element && e.target.closest("a, button, input, label")) return;
      toggle();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("click", onClick);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("click", onClick);
    };
  }, [variant, playing, play, steps, end]);

  const current = [...steps].reverse().find((s) => s.seconds_exact <= t + 1e-6) ?? steps[0];
  const pct = (s: number) => `${(s / span) * 100}%`;
  const signed = steps.find((s) => s.key === "permits_signed");
  const evm = steps.filter((s) => s.chain.startsWith("evm:"));

  return (
    <div ref={root} className={`${styles.root} ${styles[variant] ?? ""}`}>
      <div className={styles.readout} aria-live="off">
        <div className={styles.clock}>
          <span className={styles.clockLabel}>from {who} commit block</span>
          <span className={`${styles.clockValue} num`}>
            {t.toFixed(1)}
            <span className={styles.unit}> s</span>
          </span>
        </div>
        <div className={styles.now}>
          <span className={`${styles.chip} ${laneClass(current.chain)}`}>{laneName(current.chain)}</span>
          <strong>{current.label}</strong>
          {current.detail?.by ? <span className={styles.nowBy}>{cap(String(current.detail.by))}</span> : null}
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
          {/* At rest at the end, the playhead would sit on the last step's label: it shows while the replay runs. */}
          {playing || t < end - 1e-6 ? (
            <span className={styles.playhead} style={{ left: `calc(var(--lane-label) + (100% - var(--lane-label)) * ${t / span})` }} aria-hidden />
          ) : null}
        </div>
      </div>

      {/* Shown below 760 px in every variant (the stage included, on a phone); the wide chart takes over above it. */}
      <PhoneChart steps={steps} lanes={lanes} t={t} end={end} playing={playing} />

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

      {variant === "hero" ? <StepList steps={steps} /> : null}
    </div>
  );
}

function Node({ step, left, done, place }: { step: Step; left: string; done: boolean; place: "above" | "below" }) {
  const minor = Boolean(step.detail?.minor);
  // The accessible name starts with what the node shows (WCAG 2.5.3), then the chain and the transaction.
  const body = (
    <>
      <span className={styles.dot} aria-hidden />
      <span className={`${styles.nodeText} ${place === "below" ? styles.belowText : styles.aboveText}`}>
        <span className={`${styles.nodeTime} num`}>{timeText(step)}</span> <span className={styles.nodeLabel}>{step.label}</span>
        <span className="visually-hidden">{more(step)}</span>
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

/** The phone chart's geometry, in CSS pixels: time runs down, rails on the left, one label per step on the right. */
const V = { pad: 14, tickW: 24, axis: 32, rail: 44, gap: 20, label: 16, row: 24, pps: 5, maxH: 480 };

/** Phones: the same lanes top to bottom, nodes placed by their seconds, the pause permits fanning out to each chain. */
function PhoneChart({ steps, lanes, t, end, playing }: { steps: Step[]; lanes: Lane[]; t: number; end: number; playing: boolean }) {
  const pps = Math.min(V.pps, V.maxH / end);
  const y = (s: number) => V.pad + s * pps;
  const x = (lane: Lane) => V.rail + Math.max(0, lanes.indexOf(lane)) * V.gap;
  const labelX = x(lanes[lanes.length - 1]) + V.label;
  // Each label sits level with its node unless it would crowd the one above; then it moves down, joined by its leader.
  const tops: number[] = [];
  for (const s of steps) tops.push(Math.max(y(s.seconds_exact), (tops[tops.length - 1] ?? -Infinity) + V.row));
  const height = Math.ceil(Math.max(y(end), tops[tops.length - 1] ?? 0) + V.pad);
  const nearEnd = Math.max(...steps.filter((s) => s.chain === "near").map((s) => s.seconds_exact));
  const signed = steps.find((s) => s.key === "permits_signed");
  const ticks = Array.from({ length: Math.floor(end / 10) + 1 }, (_, i) => i * 10);
  const yt = y(Math.min(t, end));
  const nx = x("near");
  return (
    <div className={styles.phone} role="group" aria-label={`Timeline from 0 to ${end.toFixed(1)} seconds after the commit`}>
      <div className={styles.legend} aria-hidden>
        {lanes.map((l) => (
          <span key={l} className={`${styles.key} ${laneClass(l)}`}>
            {laneName(l)}
          </span>
        ))}
      </div>
      <div className={styles.vbody} style={{ height }}>
        {ticks.map((s) => (
          <span key={s} className={styles.vtick} style={{ top: y(s), width: V.tickW }} aria-hidden>
            {s} s
          </span>
        ))}
        <svg className={styles.vsvg} width={labelX} height={height} viewBox={`0 0 ${labelX} ${height}`} aria-hidden focusable="false">
          {ticks.map((s) => (
            <line key={s} className={styles.vgrid} x1={V.axis} x2={labelX - 8} y1={y(s)} y2={y(s)} />
          ))}
          <g className={styles.near}>
            <line className={styles.vrail} x1={nx} x2={nx} y1={y(0)} y2={y(nearEnd)} />
            <line className={styles.vfill} x1={nx} x2={nx} y1={y(0)} y2={y(Math.min(t, nearEnd))} />
          </g>
          {lanes.slice(1).map((lane) => {
            const mine = steps.filter((s) => s.chain === lane);
            if (!mine.length) return null;
            const from = signed?.seconds_exact ?? mine[0].seconds_exact;
            const to = Math.max(...mine.map((s) => s.seconds_exact));
            const lx = x(lane);
            const y0 = y(from);
            const y1 = y0 + 16;
            const y2 = y(to);
            const going = t >= from - 1e-6;
            const lit = t >= to - 1e-6;
            return (
              <g key={lane} className={laneClass(lane)}>
                <path className={`${styles.wire} ${lit ? styles.wireLit : going ? styles.wireGoing : ""}`} d={`M${nx},${y0} C${nx},${y0 + 10} ${lx},${y0 + 6} ${lx},${y1}`} />
                <line className={styles.vrail} x1={lx} x2={lx} y1={y1} y2={y2} />
                {yt > y1 ? <line className={styles.vfill} x1={lx} x2={lx} y1={y1} y2={Math.min(yt, y2)} /> : null}
              </g>
            );
          })}
          {steps.map((s, i) => {
            const minor = Boolean(s.detail?.minor);
            const cx = x(s.chain);
            const cy = y(s.seconds_exact);
            const r = minor ? 4 : 6.5;
            const done = t >= s.seconds_exact - 1e-6;
            return (
              <g key={s.key} className={`${laneClass(s.chain)} ${done ? styles.vdone : ""} ${s.chain === "near" ? "" : styles.vevm}`}>
                <path className={styles.vlead} d={`M${cx + r + 3},${cy} L${labelX - 6},${tops[i]}`} />
                <circle className={styles.vdot} cx={cx} cy={cy} r={r} />
              </g>
            );
          })}
          {playing || t < end - 1e-6 ? (
            <>
              <line className={styles.vhead} x1={V.axis} x2={labelX - 8} y1={yt} y2={yt} />
              <circle className={styles.vheadDot} cx={V.axis} cy={yt} r={3.5} />
            </>
          ) : null}
        </svg>
        {steps.map((s, i) => (
          <PhoneLabel key={s.key} step={s} top={tops[i]} left={labelX} done={t >= s.seconds_exact - 1e-6} />
        ))}
      </div>
    </div>
  );
}

function PhoneLabel({ step, top, left, done }: { step: Step; top: number; left: number; done: boolean }) {
  const cls = `${styles.vlabel} ${done ? styles.done : ""} ${step.detail?.minor ? styles.minor : ""}`;
  const style = { top: top - V.row / 2, left };
  const body = (
    <>
      <span className={`${styles.vtime} num`}>{timeText(step)}</span>
      <span className={styles.vname}>{step.label}</span>
      {step.explorer ? <Icon name="external" size={12} /> : null}
      <span className="visually-hidden">{more(step)}</span>
    </>
  );
  return step.explorer ? (
    <a className={cls} style={style} href={step.explorer} target="_blank" rel="noreferrer noopener">
      {body}
    </a>
  ) : (
    <span className={cls} style={style}>
      {body}
    </span>
  );
}

/**
 * The landing's list under the chart (wide screens): two columns, each read top to bottom in time order.
 * Steps are on NEAR testnet unless tagged with their chain.
 */
function StepList({ steps }: { steps: Step[] }) {
  return (
    <ol className={styles.list} style={{ ["--rows" as string]: Math.ceil(steps.length / 2) }} aria-label="Steps, each with its transaction (on NEAR testnet unless another chain is named)">
      {steps.map((s) => (
        <li key={s.key} className={`${styles.item} ${s.detail?.minor ? styles.itemMinor : ""}`}>
          <span className={`${styles.itemTime} num`}>{timeText(s)}</span>
          <span className={styles.itemBody}>
            <span className={styles.itemLabel}>{s.label}</span>
            {s.chain === "near" ? null : <span className={`${styles.chip} ${laneClass(s.chain)}`}>{laneName(s.chain)}</span>}
            {/* Two steps may share one transaction (a request, then its answer in a later receipt): say so rather than
                show one link twice as if it were two events. */}
            {sharedTxNote(s, steps) ? <span className={styles.itemNote}>{sharedTxNote(s, steps)}</span> : null}
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
      ))}
    </ol>
  );
}
