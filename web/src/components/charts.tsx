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

/** Every how many categories a label fits: thins the axis instead of letting labels collide. */
function labelStep<T>(data: [string, ...T[]][], slot: number): number {
  const longest = Math.max(1, ...data.map((d) => d[0].length));
  return Math.max(1, Math.ceil((longest * 6.6 + 8) / Math.max(slot, 1)));
}

export const SERIES = ['var(--s1)', 'var(--s2)', 'var(--s3)', 'var(--s4)', 'var(--s5)', 'var(--s6)'];

export type Fmt = (v: number) => string;

/** Round gridline values for a range: 3 to 6 ticks on a 1, 2, 2.5 or 5 step, covering it with the
 * least slack. Zero is always one of them when the range spans it. */
export function niceTicks(lo: number, hi: number): number[] {
  if (!(hi > lo)) return [lo];
  const raw = (hi - lo) / 4;
  const p = 10 ** Math.floor(Math.log10(raw));
  let best: { s: number; a: number; b: number } | null = null;
  for (const q of [p / 10, p, p * 10]) {
    for (const m of [1, 2, 2.5, 5]) {
      const s = q * m;
      const a = Math.floor(lo / s + 1e-9) * s;
      const b = Math.ceil(hi / s - 1e-9) * s;
      const n = Math.round((b - a) / s);
      if (n < 2 || n > 5) continue;
      if (!best || b - a < best.b - best.a - 1e-9 || (Math.abs(b - a - (best.b - best.a)) < 1e-9 && s > best.s)) best = { s, a, b };
    }
  }
  if (!best) return [lo, hi];
  const out: number[] = [];
  for (let v = best.a; v <= best.b + best.s / 2; v += best.s) out.push(Math.abs(v) < best.s / 1e6 ? 0 : v);
  return out;
}

/** An axis value without the zeros a significant-figures format pads it with: "5.00m" → "5m",
 * "1.50bn" → "1.5bn". Gridlines sit on round values, so the padding only adds noise. */
export function trimZeros(s: string): string {
  return s.replace(/(\d)\.(\d*?)0+(?=\D*$)/, (_, d: string, keep: string) => (keep ? `${d}.${keep}` : d));
}

/** Whether a bar chart labels its bars: every one (`all`), none (`none`, with a gridded axis and
 * the tooltip for exact values), or `auto`: every bar when there are at most eight and each label
 * fits its bar, otherwise none. Never some bars and not others. */
export type Labels = 'auto' | 'all' | 'none';

/** Category bars. Labelled: the value above every bar, no axis. Unlabelled: three to five
 * hairline gridlines with their values on the left. Either way, hovering a bar shows its exact
 * value (and `detail`, when given) and lifts it. */
export function Bars({
  data,
  color = SERIES[0],
  height = 150,
  fmt,
  labels = 'auto',
  dim,
  dimLegend,
  colorOf,
  detail,
}: {
  data: [string, number][];
  color?: string;
  height?: number;
  fmt: Fmt;
  labels?: Labels;
  /** Bars drawn lighter, for example forecast months beside actual ones. */
  dim?: boolean[];
  /** Legend for the two styles: [solid, light]. */
  dimLegend?: [string, string];
  /** A colour per bar instead of one for all (losses in the bad colour, for instance). */
  colorOf?: (v: number, i: number) => string;
  /** The tooltip for bar `i` in place of "category: value". */
  detail?: (i: number) => string;
}) {
  const [ref, width] = useMeasure<HTMLDivElement>();
  const [tip, setTip] = useState<Tip | null>(null);
  const [hot, setHot] = useState<number | null>(null);
  if (!data.length) return <div className="t-muted">no data</div>;
  const values = data.map((d) => d[1]);
  const widest = Math.max(...values.map((v) => fmt(v).length)) * 6.6 + 10;
  const fits = data.length <= 8 && width / data.length >= widest;
  const labelled = labels === 'all' || (labels === 'auto' && fits);
  const ticks = labelled ? [] : niceTicks(Math.min(0, ...values), Math.max(0, ...values));
  const tickText = (t: number) => trimZeros(fmt(t));
  const left = labelled ? 0 : Math.max(30, Math.max(...ticks.map((t) => tickText(t).length)) * 6.2 + 10);
  // A labelled negative bar carries its value under its end, so it needs room there.
  const top = labelled ? 16 : 8;
  const bottom = 18 + (labelled && values.some((v) => v < 0) ? 14 : 0);
  const plotH = height - top - bottom;
  const lo = labelled ? Math.min(0, ...values) : ticks[0];
  const hi = labelled ? Math.max(0, ...values) : ticks[ticks.length - 1];
  const span = Math.max(hi - lo, 1e-9);
  const yOf = (v: number) => top + plotH * (1 - (v - lo) / span);
  const baseY = yOf(0);
  const slot = Math.max(1, width - left) / data.length;
  const barW = Math.min(40, Math.max(6, slot * 0.56));
  const step = labelStep(data, slot);
  return (
    <div ref={ref} style={{ position: 'relative' }}>
      {dimLegend && (
        <div className="legend">
          <span>
            <span className="sw" style={{ background: color }} />
            {dimLegend[0]}
          </span>
          <span>
            <span className="sw" style={{ background: color, opacity: 0.4 }} />
            {dimLegend[1]}
          </span>
        </div>
      )}
      {width > 0 && (
        <svg
          className="chart"
          width={width}
          height={height}
          data-labels={labelled ? 'all' : 'none'}
          onMouseLeave={() => {
            setTip(null);
            setHot(null);
          }}
        >
          {ticks.map((t) => (
            <g key={t}>
              {t !== 0 && <line className="grid" x1={left} x2={width} y1={yOf(t)} y2={yOf(t)} />}
              <text x={left - 6} y={yOf(t) + 4} textAnchor="end">
                {tickText(t)}
              </text>
            </g>
          ))}
          {data.map(([label, v], i) => {
            const cx = left + slot * (i + 0.5);
            const y = yOf(v);
            const y0 = Math.min(y, baseY);
            const h = Math.abs(y - baseY);
            return (
              <g
                key={i}
                onMouseMove={(e) => {
                  setHot(i);
                  setTip({ x: e.clientX, y: e.clientY, text: detail ? detail(i) : `${label}: ${fmt(v)}${dim?.[i] && dimLegend ? ` (${dimLegend[1]})` : ''}` });
                }}
              >
                <rect x={cx - slot / 2} y={0} width={slot} height={height} fill="transparent" />
                <rect x={cx - barW / 2} y={y0} width={barW} height={h} rx={3} fill={colorOf ? colorOf(v, i) : color} opacity={(dim?.[i] ? 0.4 : 1) * (hot === null || hot === i ? 1 : 0.8)} />
                {labelled && v !== 0 && (
                  <text className="value" x={cx} y={v >= 0 ? y0 - 3 : y0 + h + 12} textAnchor="middle">
                    {fmt(v)}
                  </text>
                )}
                {i % step === 0 && (
                  <text x={cx} y={height - 4} textAnchor="middle">
                    {label}
                  </text>
                )}
              </g>
            );
          })}
          <line className="baseline" x1={left} x2={width} y1={baseY} y2={baseY} />
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
  const step = labelStep(data, slot);
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
                {i % step === 0 && (
                  <text x={cx} y={height - 4} textAnchor="middle">
                    {label}
                  </text>
                )}
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

/** Two series stacked in one bar per category (both non-negative), with a 2 px surface gap
 * between the segments and a legend; the total sits above each bar where there is room. */
export function StackedBars({ data, names, colors = [SERIES[0], SERIES[3]], height = 160, fmt, testId }: { data: [string, number, number][]; names: [string, string]; colors?: [string, string]; height?: number; fmt: Fmt; testId?: string }) {
  const [ref, width] = useMeasure<HTMLDivElement>();
  const [tip, setTip] = useState<Tip | null>(null);
  if (!data.length) return <div className="t-muted">no data</div>;
  const top = 16;
  const bottom = 18;
  const plotH = height - top - bottom;
  const hi = Math.max(1e-9, ...data.map((d) => Math.max(0, d[1]) + Math.max(0, d[2])));
  const hOf = (v: number) => (plotH * Math.max(0, v)) / hi;
  const slot = width / data.length;
  const barW = Math.min(44, Math.max(8, slot * 0.56));
  const base = top + plotH;
  const step = labelStep(data, slot);
  return (
    <div ref={ref} style={{ position: 'relative' }} data-testid={testId}>
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
            const ha = hOf(a);
            const hb = hOf(b);
            const gap = ha > 0 && hb > 0 ? 2 : 0;
            return (
              <g key={i} onMouseMove={(e) => setTip({ x: e.clientX, y: e.clientY, text: `${label}: ${names[0]} ${fmt(a)} · ${names[1]} ${fmt(b)} · total ${fmt(a + b)}` })}>
                <rect x={cx - slot / 2} y={0} width={slot} height={height} fill="transparent" />
                {ha > 0 && <rect x={cx - barW / 2} y={base - ha} width={barW} height={ha} rx={ha > 4 ? 2 : 0} fill={colors[0]} />}
                {hb > 0 && <rect x={cx - barW / 2} y={base - ha - gap - hb} width={barW} height={hb} rx={3} fill={colors[1]} />}
                {a + b > 0 && slot >= 44 && (
                  <text className="value" x={cx} y={base - ha - gap - hb - 3} textAnchor="middle">
                    {fmt(a + b)}
                  </text>
                )}
                {i % step === 0 && (
                  <text x={cx} y={height - 4} textAnchor="middle">
                    {label}
                  </text>
                )}
              </g>
            );
          })}
          <line className="baseline" x1={0} x2={width} y1={base} y2={base} />
        </svg>
      )}
      <Tooltip tip={tip} />
    </div>
  );
}
