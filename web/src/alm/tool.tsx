// Pieces every ALM tool shares: the two-panel layout with a rail that closes, the assumptions
// header with the what-if state and the list of what changed, numeric inputs, the headline with
// its standing, key-value rows, tables and scenario chips.
import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from 'react';
import type { AlmParams, CompareTo, RateScenario } from '../api/types';
import { SCENARIO_LABEL, standingLabel, standingTone, type Standing } from '../lib/alm';
import { nice } from '../lib/fmt';
import { usePref } from '../lib/prefs';
import { Badge, Chip, useOutsideClose } from '../components/ui';
import { useAlm } from './state';

export const BASEL: RateScenario[] = ['PARALLEL_UP', 'PARALLEL_DOWN', 'STEEPENER', 'FLATTENER', 'SHORT_UP', 'SHORT_DOWN'];

interface Rail {
  open: boolean;
  toggle: () => void;
}
const RailCtx = createContext<Rail>({ open: true, toggle: () => {} });

export function ToolLayout({ assumptions, children, ready }: { assumptions: ReactNode; children: ReactNode; ready?: boolean }) {
  const [open, setOpen] = usePref<boolean>('alm-assumptions', true);
  const rail = useMemo<Rail>(() => ({ open, toggle: () => setOpen((o) => !o) }), [open, setOpen]);
  return (
    <RailCtx.Provider value={rail}>
      <div className={`alm-layout${open ? '' : ' single'}`} data-ready={ready ? '1' : undefined}>
        {open && <aside className="assumptions">{assumptions}</aside>}
        <section className="alm-results">{children}</section>
      </div>
    </RailCtx.Provider>
  );
}

type Leaf = string | number | boolean | null;
interface Change {
  path: string[];
  bank: Leaf;
  now: Leaf;
}

function leaves(v: unknown, path: string[], out: Map<string, { path: string[]; value: Leaf }>) {
  if (v !== null && typeof v === 'object' && !Array.isArray(v)) {
    for (const [k, x] of Object.entries(v as Record<string, unknown>)) leaves(x, [...path, k], out);
  } else {
    const value: Leaf = Array.isArray(v) ? JSON.stringify(v) : (v as Leaf);
    out.set(path.join('\u0000'), { path, value });
  }
}

/** Every leaf where the what-if differs from the bank's set. */
export function diffParams(bank: AlmParams, working: AlmParams): Change[] {
  const a = new Map<string, { path: string[]; value: Leaf }>();
  const b = new Map<string, { path: string[]; value: Leaf }>();
  leaves(bank, [], a);
  leaves(working, [], b);
  const out: Change[] = [];
  for (const [k, w] of b) {
    const base = a.get(k);
    if (!base || base.value !== w.value) out.push({ path: w.path, bank: base?.value ?? null, now: w.value });
  }
  return out;
}

function setPath(obj: unknown, path: string[], value: Leaf) {
  let cur = obj as Record<string, unknown>;
  for (const k of path.slice(0, -1)) cur = cur[k] as Record<string, unknown>;
  const last = path[path.length - 1];
  cur[last] = typeof value === 'string' && value.startsWith('[') ? JSON.parse(value) : value;
}

const fmtLeaf = (v: Leaf) => (v === null ? '—' : typeof v === 'number' ? String(Number(v.toFixed(4))) : String(v));

export function AssumptionsHeader({ title = 'Assumptions' }: { title?: string }) {
  const { overrides, reset, bankVersion, bankChanged, bank, working, edit } = useAlm();
  const { toggle } = useContext(RailCtx);
  const changes = useMemo(() => (bank && working && overrides ? diffParams(bank, working) : []), [bank, working, overrides]);
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const ref = useOutsideClose<HTMLDivElement>(open, close);
  return (
    <>
      <div className="row" style={{ marginBottom: 4 }}>
        <span className="t-heading">{title}</span>
        <span style={{ flex: 1 }} />
        {overrides ? (
          <>
            <div className="anchor" ref={ref}>
              <button type="button" className="chip chip-sm" style={{ background: 'var(--warn-soft)', color: 'var(--warn)' }} onClick={() => setOpen((o) => !o)} aria-expanded={open} title="unsaved edits; Setup saves a version">
                what-if · {changes.length} {changes.length === 1 ? 'change' : 'changes'}
              </button>
              {open && (
                <div className="popup" role="dialog" aria-label="What changed" style={{ left: 0, right: 'auto', minWidth: 320 }}>
                  <div className="t-eyebrow" style={{ marginBottom: 6 }}>
                    Against bank v{bankVersion}
                  </div>
                  <div className="changes">
                    {changes.map((c) => (
                      <div key={c.path.join('.')} className="change">
                        <span className="t-secondary">{c.path.map(nice).join(' › ')}</span>
                        <span className="num">
                          <span className="t-muted">{fmtLeaf(c.bank)}</span> → <span className="t-medium">{fmtLeaf(c.now)}</span>
                        </span>
                        <button type="button" className="btn btn-ghost btn-sm" onClick={() => edit((p) => setPath(p, c.path, c.bank))} title="back to the bank's value">
                          revert
                        </button>
                      </div>
                    ))}
                    {changes.length === 0 && <div className="t-muted">nothing differs from the bank's set</div>}
                  </div>
                </div>
              )}
            </div>
            <button type="button" className="btn btn-sm" onClick={reset} title="back to the bank's saved parameters">
              Reset
            </button>
          </>
        ) : (
          <Badge title="the bank's active parameter set">bank v{bankVersion ?? '…'}</Badge>
        )}
        <button type="button" className="btn btn-ghost btn-sm" onClick={toggle} aria-label="hide assumptions" title="hide the assumptions; results take the full width">
          ✕
        </button>
      </div>
      {bankChanged && <div className="notice warn">the bank's parameters changed while you were editing; reset to pick them up</div>}
    </>
  );
}

export function Group({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <>
      <div className="t-eyebrow" style={{ margin: '10px 0 4px' }}>
        {title}
      </div>
      {children}
    </>
  );
}

export function Row({ label, children }: { label: ReactNode; children: ReactNode }) {
  return (
    <label className="arow">
      <span className="k">{label}</span>
      <span className="v">{children}</span>
    </label>
  );
}

/** A number input: commits on each change, keeps the caret, clamps to the range. */
export function Num({ value, onChange, min, max, step = 1, decimals = 0, suffix, width = 84, disabled }: { value: number; onChange: (v: number) => void; min?: number; max?: number; step?: number; decimals?: number; suffix?: string; width?: number; disabled?: boolean }) {
  return (
    <span className="numwrap">
      <input
        type="number"
        className="input input-sm input-num"
        style={{ width }}
        value={Number.isFinite(value) ? Number(value.toFixed(decimals)) : ''}
        step={step}
        min={min}
        max={max}
        disabled={disabled}
        onChange={(e) => {
          const n = Number(e.target.value);
          if (!Number.isFinite(n)) return;
          const c = Math.min(max ?? Infinity, Math.max(min ?? -Infinity, n));
          onChange(c);
        }}
      />
      {suffix && <span className="suffix t-small">{suffix}</span>}
    </span>
  );
}

export const Pct = ({ value, onChange, max = 100 }: { value: number; onChange: (v: number) => void; max?: number }) => <Num value={value} onChange={onChange} min={0} max={max} step={0.5} decimals={1} suffix="%" />;
export const Bp = ({ value, onChange }: { value: number; onChange: (v: number) => void }) => <Num value={value} onChange={onChange} min={-2000} max={2000} step={5} decimals={0} suffix="bp" />;

/** The one number a tool is about, its standing beside it, and two lines of context. */
export function Headline({ value, standing, line1, line2, testId }: { value: string; standing: Standing; line1: string; line2: string; testId?: string }) {
  return (
    <div style={{ marginBottom: 10 }} data-testid={testId}>
      <div className="row" style={{ alignItems: 'center', gap: 10 }}>
        <span className="t-hero">{value}</span>
        <Badge tone={standingTone(standing)} dot>
          {standingLabel(standing)}
        </Badge>
      </div>
      <div className="t-small">{line1}</div>
      <div className="t-small">{line2}</div>
    </div>
  );
}

export function KV({ rows, boldLast }: { rows: [string, string][]; boldLast?: boolean }) {
  return (
    <div className="kv">
      {rows.map(([k, v], i) => {
        const emph = boldLast && i === rows.length - 1;
        return (
          <div key={k + i} className={emph ? 'emph' : ''} style={{ display: 'contents' }}>
            <span className="k">{k}</span>
            <span className="v">{v}</span>
          </div>
        );
      })}
    </div>
  );
}

/** A striped table: the first column is text, the rest right-aligned figures. */
export function SimpleTable({ headers, rows, testId }: { headers: string[]; rows: string[][]; testId?: string }) {
  if (!rows.length) return <div className="t-muted">none</div>;
  return (
    <table className="table" data-testid={testId}>
      <thead>
        <tr>
          {headers.map((h, i) => (
            <th key={h} className={i > 0 ? 'num' : ''}>
              {h}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((r, i) => (
          <tr key={i}>
            {r.map((c, j) => (
              <td key={j} className={j > 0 ? 'num' : ''}>
                {c}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function ScenarioPicker({ includeBase }: { includeBase: boolean }) {
  const { scenario, setScenario, customBp, setCustomBp } = useAlm();
  const all: RateScenario[] = [...(includeBase ? (['BASE'] as RateScenario[]) : []), ...BASEL, 'CUSTOM'];
  return (
    <div className="chips" style={{ marginBottom: 4 }}>
      {all.map((s) => (
        <Chip key={s} small selected={scenario === s} onClick={() => setScenario(s)}>
          {SCENARIO_LABEL[s]}
        </Chip>
      ))}
      {scenario === 'CUSTOM' && <Bp value={customBp} onChange={setCustomBp} />}
    </div>
  );
}

export function ComparePicker() {
  const { compareTo, setCompareTo } = useAlm();
  const opts: [CompareTo, string][] = [
    ['PRIOR', 'Prior month'],
    ['BUDGET', 'Budget'],
  ];
  return (
    <div className="chips">
      {opts.map(([c, label]) => (
        <Chip key={c} small selected={compareTo === c} onClick={() => setCompareTo(c)}>
          {label}
        </Chip>
      ))}
    </div>
  );
}

export function ResultsHeader({ title, loading, error }: { title: string; loading: boolean; error?: string | null }) {
  const { open, toggle } = useContext(RailCtx);
  const { overrides } = useAlm();
  return (
    <div className="row" style={{ marginBottom: 6 }}>
      {!open && (
        <button type="button" className="btn btn-sm noprint" onClick={toggle} title="show the assumptions">
          Assumptions
        </button>
      )}
      <h1 className="t-title">{title}</h1>
      {!open && overrides && <Badge tone="warn">what-if</Badge>}
      {loading && <span className="t-small">computing…</span>}
      {error && <span className="t-bad">{error}</span>}
      <span style={{ flex: 1 }} />
      <button type="button" className="btn btn-ghost btn-sm noprint" onClick={() => window.print()} title="print this tool's results">
        Print
      </button>
    </div>
  );
}
