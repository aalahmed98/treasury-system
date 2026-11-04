// Charts as plain SVG: thin marks with gaps, a hairline baseline, labels in the muted ink, series
// colours from the palette in fixed order, and a hover tooltip. The egui painters are the spec.
import { useCallback, useRef, useState } from 'react';

export function useMeasure<T extends HTMLElement>(): [(el: T | null) => void, number] {
  const [w, setW] = useState(0);
  const ro = useRef<ResizeObserver | null>(null);
  // A callback ref: the element may appear only after the data does, so an effect at mount is
  // too early to observe it.
  const setRef = useCallback((el: T | null) => {
    ro.current?.disconnect();
    ro.current = null;
    if (!el) return;
    const obs = new ResizeObserver((entries) => {
      for (const e of entries) setW(Math.floor(e.contentRect.width));
    });
    obs.observe(el);
    ro.current = obs;
    setW(Math.floor(el.getBoundingClientRect().width));
  }, []);
  return [setRef, w];
}

interface Tip {
  x: number;
  y: number;
  text: string;
}

function Tooltip({ tip }: { tip: Tip | null }) {
  if (!tip) return null;
  return (
    <div className="tip" style={{ left: tip.x + 12, top: tip.y + 12 }}>
      {tip.text}
    </div>
  );
}

export const SERIES = ['var(--s1)', 'var(--s2)', 'var(--s3)', 'var(--s4)', 'var(--s5)', 'var(--s6)'];

export type Fmt = (v: number) => string;

/** Category bars: the value above each bar, the category under it. */
/** Which bars carry a value label: every one, the extremes only, or none (the tooltip still has
 * every value). `auto` labels every bar when there is room and only the extremes when there is not. */
export type Labels = 'auto' | 'all' | 'extremes' | 'none';

export function Bars({ data, color = SERIES[0], height = 150, fmt, labels = 'auto' }: { data: [string, number][]; color?: string; height?: number; fmt: Fmt; labels?: Labels }) {
  const [ref, width] = useMeasure<HTMLDivElement>();
  const [tip, setTip] = useState<Tip | null>(null);
  const [hot, setHot] = useState<number | null>(null);
  if (!data.length) return <div className="t-muted">no data</div>;
  const top = 16;
  const bottom = 18;
  const plotH = height - top - bottom;
  const lo = Math.min(0, ...data.map((d) => d[1]));
  const hi = Math.max(0, ...data.map((d) => d[1]));
  const span = Math.max(hi - lo, 1e-9);
  const yOf = (v: number) => top + plotH * (1 - (v - lo) / span);
  const baseY = yOf(0);
  const slot = width / data.length;
  const barW = Math.min(40, Math.max(6, slot * 0.56));
  const mode: Labels = labels === 'auto' ? (slot >= 52 ? 'all' : 'extremes') : labels;
  const maxI = data.reduce((m, d, i) => (d[1] > data[m][1] ? i : m), 0);
  const minI = data.reduce((m, d, i) => (d[1] < data[m][1] ? i : m), 0);
  const labelled = (i: number) => mode === 'all' || (mode === 'extremes' && (i === maxI || (i === minI && data[minI][1] < 0)));
  return (
    <div ref={ref} style={{ position: 'relative' }}>
      {width > 0 && (
        <svg
          className="chart"
          width={width}
          height={height}
          onMouseLeave={() => {
            setTip(null);
            setHot(null);
          }}
        >
          {data.map(([label, v], i) => {
            const cx = slot * (i + 0.5);
            const y = yOf(v);
            const y0 = Math.min(y, baseY);
            const h = Math.abs(y - baseY);
            return (
              <g
                key={i}
                onMouseMove={(e) => {
                  setHot(i);
                  setTip({ x: e.clientX, y: e.clientY, text: `${label}: ${fmt(v)}` });
                }}
              >
                <rect x={cx - slot / 2} y={0} width={slot} height={height} fill="transparent" />
                <rect x={cx - barW / 2} y={y0} width={barW} height={h} rx={3} fill={color} opacity={hot === null || hot === i ? 1 : 0.8} />
                {v !== 0 && labelled(i) && (
                  <text className="value" x={cx} y={v >= 0 ? y0 - 3 : baseY - 3} textAnchor="middle">
                    {fmt(v)}
                  </text>
                )}
                <text x={cx} y={height - 4} textAnchor="middle">
                  {label}
                </text>
              </g>
            );
          })}
          <line className="baseline" x1={0} x2={width} y1={baseY} y2={baseY} />
        </svg>
      )}
      <Tooltip tip={tip} />
    </div>
  );
}

/** Two bars per category, for example actual against budget, with a legend. */
export function BarsPair({ data, names, height = 150, fmt }: { data: [string, number, number][]; names: [string, string]; height?: number; fmt: Fmt }) {
  const [ref, width] = useMeasure<HTMLDivElement>();
  const [tip, setTip] = useState<Tip | null>(null);
  if (!data.length) return <div className="t-muted">no data</div>;
  const colors = [SERIES[0], SERIES[3]];
  const top = 8;
  const bottom = 18;
  const plotH = height - top - bottom;
  const all = data.flatMap((d) => [d[1], d[2]]);
  const lo = Math.min(0, ...all);
  const hi = Math.max(0, ...all);
  const span = Math.max(hi - lo, 1e-9);
  const yOf = (v: number) => top + plotH * (1 - (v - lo) / span);
  const baseY = yOf(0);
  const slot = width / data.length;
  const barW = Math.min(26, Math.max(4, slot * 0.3));
  return (
    <div ref={ref} style={{ position: 'relative' }}>
      <div className="legend">
        {names.map((n, i) => (
          <span key={n}>
            <span className="sw" style={{ background: colors[i] }} />
            {n}
          </span>
        ))}
      </div>
      {width > 0 && (
        <svg className="chart" width={width} height={height} onMouseLeave={() => setTip(null)}>
          {data.map(([label, a, b], i) => {
            const cx = slot * (i + 0.5);
            return (
              <g key={i} onMouseMove={(e) => setTip({ x: e.clientX, y: e.clientY, text: `${label}: ${names[0]} ${fmt(a)} · ${names[1]} ${fmt(b)}` })}>
                <rect x={cx - slot / 2} y={0} width={slot} height={height} fill="transparent" />
                {[a, b].map((v, j) => {
                  const x = cx + (j - 0.5) * (barW + 2);
                  const y = yOf(v);
                  return <rect key={j} x={x - barW / 2} y={Math.min(y, baseY)} width={barW} height={Math.abs(y - baseY)} rx={2} fill={colors[j]} opacity={0.92} />;
                })}
                <text x={cx} y={height - 4} textAnchor="middle">
                  {label}
                </text>
              </g>
            );
          })}
          <line className="baseline" x1={0} x2={width} y1={baseY} y2={baseY} />
        </svg>
      )}
      <Tooltip tip={tip} />
    </div>
  );
}

export interface Series {
  name: string;
  points: [number, number][];
  color?: string;
}

/** Lines over a numeric x axis with three gridlines, a crosshair and a tooltip. */
export function LineChart({ series, height = 150, yFmt, xFmt, refY, markX, markLabel }: { series: Series[]; height?: number; yFmt: Fmt; xFmt: (x: number) => string; /** A reference level, e.g. zero, drawn when it is inside the range. */ refY?: number; /** A marked x position, e.g. today. */ markX?: number; markLabel?: string }) {
  const [ref, width] = useMeasure<HTMLDivElement>();
  const [hover, setHover] = useState<{ x: number; px: number; tipX: number; tipY: number } | null>(null);
  const pts = series.flatMap((s) => s.points);
  if (!pts.length) return <div className="t-muted">no data</div>;
  const left = 46;
  const right = 8;
  const top = 8;
  const bottom = 18;
  const xs = pts.map((p) => p[0]);
  const ys = pts.map((p) => p[1]);
  const x0 = Math.min(...xs);
  const x1 = Math.max(...xs);
  const dataLo = Math.min(...ys);
  const dataHi = Math.max(...ys);
  // The reference level joins the scale only when it is near the data; a zero line under a
  // balance of 1.4bn would flatten the line into a ruler.
  const showRef = refY !== undefined && refY >= dataLo - (dataHi - dataLo) * 1.5 - 1e-9 && refY <= dataHi + (dataHi - dataLo) * 1.5 + 1e-9;
  let y0 = Math.min(...ys, showRef ? refY! : Infinity);
  let y1 = Math.max(...ys, showRef ? refY! : -Infinity);
  if (y1 - y0 < 1e-9) {
    y0 -= 1;
    y1 += 1;
  }
  const pad = (y1 - y0) * 0.08;
  y0 -= pad;
  y1 += pad;
  const plotW = Math.max(1, width - left - right);
  const plotH = height - top - bottom;
  const xOf = (x: number) => left + (plotW * (x - x0)) / Math.max(x1 - x0, 1e-9);
  const yOf = (y: number) => top + plotH * (1 - (y - y0) / (y1 - y0));
  const ticks = [y0 + pad, (y0 + y1) / 2, y1 - pad];
  const xTicks = x1 > x0 ? [x0, (x0 + x1) / 2, x1] : [x0];
  const colors = series.map((s, i) => s.color ?? SERIES[i % SERIES.length]);
  const nearest = (px: number) => {
    const x = x0 + ((px - left) / plotW) * (x1 - x0);
    let best = xs[0];
    let bd = Infinity;
    for (const v of xs) {
      const d = Math.abs(v - x);
      if (d < bd) {
        bd = d;
        best = v;
      }
    }
    return best;
  };
  return (
    <div ref={ref} style={{ position: 'relative' }}>
      {series.length >= 2 && (
        <div className="legend">
          {series.map((s, i) => (
            <span key={s.name}>
              <span className="sw" style={{ background: colors[i] }} />
              {s.name}
            </span>
          ))}
        </div>
      )}
      {width > 0 && (
        <svg
          className="chart"
          width={width}
          height={height}
          onMouseMove={(e) => {
            const r = (e.currentTarget as SVGSVGElement).getBoundingClientRect();
            const px = e.clientX - r.left;
            if (px < left || px > width - right) return setHover(null);
            const x = nearest(px);
            setHover({ x, px: xOf(x), tipX: e.clientX, tipY: e.clientY });
          }}
          onMouseLeave={() => setHover(null)}
        >
          {ticks.map((t, i) => (
            <g key={i}>
              <line className="grid" x1={left} x2={width - right} y1={yOf(t)} y2={yOf(t)} />
              <text x={left - 6} y={yOf(t) + 4} textAnchor="end">
                {yFmt(t)}
              </text>
            </g>
          ))}
          {xTicks.map((t, i) => (
            <text key={i} x={xOf(t)} y={height - 4} textAnchor={i === 0 ? 'start' : i === xTicks.length - 1 ? 'end' : 'middle'}>
              {xFmt(t)}
            </text>
          ))}
          {showRef && <line className="ref" x1={left} x2={width - right} y1={yOf(refY)} y2={yOf(refY)} />}
          {markX !== undefined && (
            <g>
              <line className="mark" x1={xOf(markX)} x2={xOf(markX)} y1={top} y2={height - bottom} />
              {markLabel && (
                <text x={xOf(markX) + 4} y={top + 10} className="mark-label">
                  {markLabel}
                </text>
              )}
            </g>
          )}
          {series.map((s, i) => (
            <polyline key={s.name} fill="none" stroke={colors[i]} strokeWidth={1.8} strokeLinejoin="round" strokeLinecap="round" points={s.points.map((p) => `${xOf(p[0])},${yOf(p[1])}`).join(' ')} />
          ))}
          {hover && (
            <g>
              <line x1={hover.px} x2={hover.px} y1={top} y2={height - bottom} stroke="var(--border)" strokeDasharray="2 3" />
              {series.map((s, i) => {
                const p = s.points.find((q) => q[0] === hover.x);
                return p ? <circle key={s.name} cx={xOf(p[0])} cy={yOf(p[1])} r={3.5} fill={colors[i]} stroke="var(--surface)" strokeWidth={2} /> : null;
              })}
            </g>
          )}
        </svg>
      )}
      {hover && (
        <Tooltip
          tip={{
            x: hover.tipX,
            y: hover.tipY,
            text: `${xFmt(hover.x)} · ${series
              .map((s) => {
                const p = s.points.find((q) => q[0] === hover.x);
                return p ? `${s.name} ${yFmt(p[1])}` : null;
              })
              .filter(Boolean)
              .join(' · ')}`,
          }}
        />
      )}
    </div>
  );
}

/** A waterfall: totals as full bars from the baseline, steps floating from the running level. */
export function Waterfall({ steps, height = 200, fmt }: { steps: [string, number, boolean][]; height?: number; fmt: Fmt }) {
  const [ref, width] = useMeasure<HTMLDivElement>();
  const [tip, setTip] = useState<Tip | null>(null);
  if (!steps.length) return <div className="t-muted">no data</div>;
  const top = 16;
  const bottom = 18;
  const plotH = height - top - bottom;
  let level = 0;
  const spans: [number, number][] = steps.map(([, v, total]) => {
    if (total) {
      level = v;
      return [0, v];
    }
    const s: [number, number] = [level, level + v];
    level += v;
    return s;
  });
  const lo = Math.min(0, ...spans.flat());
  const hi = Math.max(0, ...spans.flat());
  const span = Math.max(hi - lo, 1e-9);
  const yOf = (v: number) => top + plotH * (1 - (v - lo) / span);
  const slot = width / steps.length;
  const barW = Math.min(56, Math.max(6, slot * 0.56));
  return (
    <div ref={ref} style={{ position: 'relative' }}>
      {width > 0 && (
        <svg className="chart" width={width} height={height} onMouseLeave={() => setTip(null)}>
          <line className="baseline" x1={0} x2={width} y1={yOf(0)} y2={yOf(0)} />
          {steps.map(([label, v, total], i) => {
            const [a, b] = spans[i];
            const cx = slot * (i + 0.5);
            const ya = yOf(a);
            const yb = yOf(b);
            const color = total ? SERIES[0] : v >= 0 ? 'var(--good)' : 'var(--bad)';
            const text = total ? fmt(v) : v >= 0 ? `+${fmt(v)}` : fmt(v);
            return (
              <g key={i} onMouseMove={(e) => setTip({ x: e.clientX, y: e.clientY, text: `${label}: ${text}` })}>
                <rect x={cx - slot / 2} y={0} width={slot} height={height} fill="transparent" />
                <rect x={cx - barW / 2} y={Math.min(ya, yb)} width={barW} height={Math.abs(ya - yb)} rx={3} fill={color} opacity={0.9} />
                {i + 1 < steps.length && <line x1={cx + barW / 2} x2={cx + slot - barW / 2} y1={yb} y2={yb} stroke="var(--border)" />}
                <text className="value" x={cx} y={Math.min(ya, yb) - 3} textAnchor="middle">
                  {text}
                </text>
                <text x={cx} y={height - 4} textAnchor="middle">
                  {label}
                </text>
              </g>
            );
          })}
        </svg>
      )}
      <Tooltip tip={tip} />
    </div>
  );
}
