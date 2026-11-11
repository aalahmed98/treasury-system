// Pieces every ALM tool shares: the two-panel layout with a rail that closes, the assumptions
// header with the what-if state and the list of what changed, numeric inputs, the headline with
// its standing, key-value rows, tables and scenario chips.
import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from 'react';
import type { CompareTo, RateScenario } from '../api/types';
import { standingLabel, standingTone, type Standing } from '../lib/alm';
import { nice } from '../lib/fmt';
import { usePref } from '../lib/prefs';
import { Badge, Chip, useOutsideClose } from '../components/ui';
import { useAlm, useTool } from './state';
import { hedgesSummary } from './hedge';
import type { AlmCoverage } from '../api/types';
import { fmtMoneyShort } from '../lib/fmt';

/** What a lever applies to in the snapshot: `undefined` while loading or for a lever the server does
 * not measure, `0` when there is nothing for it to act on. */
export function useCoverage(): (path: string) => number | undefined {
  const q = useTool<AlmCoverage>('coverage');
  const amounts = q.data?.amounts;
  return useCallback((path: string) => amounts?.[path], [amounts]);
}

/** The tooltip and the empty flag for a lever, from its coverage. */
export function coverHint(amount: number | undefined): { empty: boolean; title?: string } {
  if (amount === undefined) return { empty: false };
  if (Math.abs(amount) < 0.5) return { empty: true, title: 'nothing in this snapshot for this assumption to act on' };
  return { empty: false, title: `applies to ${fmtMoneyShort(amount)} in this snapshot` };
}

export const BASEL: RateScenario[] = ['PARALLEL_UP', 'PARALLEL_DOWN', 'STEEPENER', 'FLATTENER', 'SHORT_UP', 'SHORT_DOWN'];

interface Rail {
  open: boolean;
  toggle: () => void;
}
const RailCtx = createContext<Rail>({ open: true, toggle: () => {} });

export function ToolLayout({ assumptions, children, ready, busy }: { assumptions: ReactNode; children: ReactNode; ready?: boolean; /** A recompute is on the way: the figures on screen are about to change. */ busy?: boolean }) {
  const [open, setOpen] = usePref<boolean>('alm-assumptions', true);
  const rail = useMemo<Rail>(() => ({ open, toggle: () => setOpen((o) => !o) }), [open, setOpen]);
  return (
    <RailCtx.Provider value={rail}>
      <div className={`alm-layout${open ? '' : ' collapsed'}`} data-ready={ready ? '1' : undefined}>
        {open ? (
          <aside className="assumptions">{assumptions}</aside>
        ) : (
          // Closed, the rail leaves a tab where it stood, so it comes back from the place it went.
          <button type="button" className="assumptions-tab noprint" onClick={rail.toggle} aria-expanded={false} title="show the assumptions">
            <span aria-hidden>»</span>
            <span className="label">Assumptions</span>
          </button>
        )}
        <section className="alm-results" data-busy={busy ? '1' : undefined} aria-busy={busy || undefined}>
          {children}
        </section>
      </div>
    </RailCtx.Provider>
  );
}

type Leaf = string | number | boolean | null;
export interface Change {
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
/** Every leaf that differs between two versions of a document: changed, added (bank null) or
 * removed (now null). Works on any plain JSON document: ALM parameters, FX limits. */
export function diffParams(bank: object, working: object): Change[] {
  const a = new Map<string, { path: string[]; value: Leaf }>();
  const b = new Map<string, { path: string[]; value: Leaf }>();
  leaves(bank, [], a);
  leaves(working, [], b);
  const out: Change[] = [];
  for (const [k, w] of b) {
    const base = a.get(k);
    if (!base || base.value !== w.value) out.push({ path: w.path, bank: base?.value ?? null, now: w.value });
  }
  for (const [k, base] of a) if (!b.has(k)) out.push({ path: base.path, bank: base.value, now: null });
  return out;
}

function setPath(obj: unknown, path: string[], value: Leaf) {
  let cur = obj as Record<string, unknown>;
  for (const k of path.slice(0, -1)) {
    if (cur[k] === undefined || cur[k] === null) cur[k] = {};
    cur = cur[k] as Record<string, unknown>;
  }
  const last = path[path.length - 1];
  // Reverting something the bank's version does not have removes it.
  if (value === null) delete cur[last];
  else cur[last] = typeof value === 'string' && value.startsWith('[') ? JSON.parse(value) : value;
}

const fmtLeaf = (v: Leaf) => (v === null ? '—' : typeof v === 'number' ? String(Number(v.toFixed(4))) : String(v));

/** The list of changed parameters, bank value to new value, with a revert on each. */
export function ChangeList({ changes, onRevert, empty }: { changes: Change[]; onRevert: (c: Change) => void; empty: string }) {
  return (
    <div className="changes">
      {changes.map((c) => (
        <div key={c.path.join('.')} className="change">
          <span className="t-secondary">{c.path.map(nice).join(' › ')}</span>
          <span className="num">
            <span className="t-muted">{fmtLeaf(c.bank)}</span> → <span className="t-medium">{fmtLeaf(c.now)}</span>
          </span>
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => onRevert(c)} title="back to the bank's value">
            revert
          </button>
        </div>
      ))}
      {changes.length === 0 && <div className="t-muted">{empty}</div>}
    </div>
  );
}

export function revertPath(p: object, c: Change) {
  setPath(p, c.path, c.bank);
}

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
                  <ChangeList changes={changes} onRevert={(c) => edit((p) => revertPath(p, c))} empty="nothing differs from the bank's set" />
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

/** A group of assumptions. With children it collapses; the first groups a tool needs stay open and
 * the long tails (run-off rates, shocks per currency) start closed with their count on show. */
export function Group({ title, help, open = true, count, children }: { title: string; help?: string; open?: boolean; count?: number; children?: ReactNode }) {
  if (children === undefined) {
    return (
      <div className="t-eyebrow" style={{ margin: '10px 0 4px' }}>
        {title}
      </div>
    );
  }
  return (
    <details className="agroup" open={open}>
      <summary>
        <span className="t-eyebrow">{title}</span>
        {count !== undefined && <span className="t-small">{count}</span>}
      </summary>
      {help && <div className="agroup-help">{help}</div>}
      {children}
    </details>
  );
}

export function Row({ label, children, cover }: { label: ReactNode; children: ReactNode; /** The lever's parameter path, to show what it applies to. */ cover?: string }) {
  if (cover) return <CoveredRow label={label} cover={cover}>{children}</CoveredRow>;
  return (
    <label className="arow">
      <span className="k">{label}</span>
      <span className="v">{children}</span>
    </label>
  );
}

function CoveredRow({ label, children, cover }: { label: ReactNode; children: ReactNode; cover: string }) {
  const amount = useCoverage()(cover);
  const { empty, title } = coverHint(amount);
  return (
    <label className={`arow${empty ? ' idle' : ''}`} title={title} data-empty={empty ? '1' : undefined}>
      <span className="k">
        {label}
        {empty && <span className="cover-none">nothing in this snapshot</span>}
      </span>
      <span className="v">{children}</span>
    </label>
  );
}

/** A number input: commits on each change, keeps the caret, clamps to the range. With `base`, the
 * bank's value appears as a ghost beside an edited input; clicking it puts the value back. */
export function Num({ value, onChange, min, max, step = 1, decimals = 0, suffix, width = 84, disabled, base, empty, title }: { value: number; onChange: (v: number) => void; min?: number; max?: number; step?: number; decimals?: number; suffix?: string; width?: number; disabled?: boolean; base?: number; /** Nothing in the snapshot for it to act on. */ empty?: boolean; title?: string }) {
  const edited = base !== undefined && Number.isFinite(base) && Math.abs(base - value) > 1e-9;
  return (
    <span className={`numwrap${edited ? ' edited' : ''}${empty ? ' idle' : ''}`} title={title} data-empty={empty ? '1' : undefined}>
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
      {edited && (
        <button type="button" className="ghost" onClick={() => onChange(base!)} title="the bank's value; click to put it back">
          bank {Number(base!.toFixed(decimals))}
          {suffix ?? ''}
        </button>
      )}
    </span>
  );
}

export const Pct = ({ value, onChange, max = 100, base }: { value: number; onChange: (v: number) => void; max?: number; base?: number }) => <Num value={value} onChange={onChange} min={0} max={max} step={0.5} decimals={1} suffix="%" base={base} />;
export const Bp = ({ value, onChange, base }: { value: number; onChange: (v: number) => void; base?: number }) => <Num value={value} onChange={onChange} min={-2000} max={2000} step={5} decimals={0} suffix="bp" base={base} />;

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
export function SimpleTable({ headers, rows, testId, mark }: { headers: string[]; rows: string[][]; testId?: string; /** The row to pick out, e.g. the scenario the shock bar is on. */ mark?: number }) {
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
          <tr key={i} className={i === mark ? 'marked' : undefined} aria-current={i === mark ? 'true' : undefined}>
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

export function ResultsHeader({ title, loading, error, onRetry, shock, rates }: { title: string; loading: boolean; error?: string | null; onRetry?: () => void; /** The rate shock these figures ran under, in words; it prints with them. */ shock?: string | null; /** A rates tool: trial hedges reach it. */ rates?: boolean }) {
  const { open } = useContext(RailCtx);
  const { overrides, hedges } = useAlm();
  return (
    <div className="row" style={{ marginBottom: 6 }}>
      <h1 className="t-title">{title}</h1>
      {!open && overrides && <Badge tone="warn">what-if</Badge>}
      {shock && (
        <span data-testid="shock-badge">
          <Badge tone="warn" title="a simulation: these figures are under the rate shock chosen above">
            shock · {shock}
          </Badge>
        </span>
      )}
      {rates && hedges.length > 0 && (
        <span data-testid="hedge-badge">
          <Badge tone="warn" title="a simulation: these figures include the trial hedges chosen above, which are not booked">
            with {hedgesSummary(hedges.length)}
          </Badge>
        </span>
      )}
      {loading && !error && (
        <span className="t-small computing" role="status">
          recomputing, the figures below are about to change…
        </span>
      )}
      {error && (
        <span className="row" role="alert">
          <span className="t-bad">{error}</span>
          {onRetry && (
            <button type="button" className="btn btn-sm" onClick={onRetry}>
              Try again
            </button>
          )}
        </span>
      )}
      <span style={{ flex: 1 }} />
      <button type="button" className="btn btn-ghost btn-sm noprint" onClick={() => window.print()} title="print this tool's results">
        Print
      </button>
    </div>
  );
}

/** One line under a rates tool's headline saying what the trial hedges changed. */
export function HedgeEffect({ children }: { children: ReactNode }) {
  return (
    <div className="shock-result" data-testid="hedge-effect">
      {children}
    </div>
  );
}
