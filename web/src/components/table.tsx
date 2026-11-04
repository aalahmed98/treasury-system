// A virtualised, sortable, resizable table over pre-formatted rows, with keyboard selection, a
// column picker and density, plus CSV export. Columns are plain objects: the pages describe what
// to show, the table does the rest.
import { memo, useCallback, useMemo, useRef, useState, type ReactNode } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { usePref } from '../lib/prefs';
import { useOutsideClose } from './ui';

export interface Col<T> {
  id: string;
  header: string;
  width: number;
  /** Right-aligned figures. */
  num?: boolean;
  /** Sort comparator; absent means the column does not sort. */
  sort?: (a: T, b: T) => number;
  cell: (row: T) => ReactNode;
  /** The CSV value; defaults to the cell's text when it is a string. */
  csv?: (row: T) => string;
  title?: string;
  /** Always shown; the column picker does not offer to hide it. */
  fixed?: boolean;
}

export interface Sort {
  id: string;
  desc: boolean;
}

export type SelectVia = 'click' | 'key';

export interface DataTableProps<T> {
  columns: Col<T>[];
  rows: T[];
  rowKey: (row: T) => string;
  selected?: string | null;
  /** A click toggles in the pages; the arrow keys always select. */
  onSelect?: (row: T, via: SelectVia) => void;
  /** Enter on the selected row. */
  onActivate?: (row: T) => void;
  onEscape?: () => void;
  rowClass?: (row: T) => string;
  sort: Sort | null;
  onSort: (s: Sort | null) => void;
  rowHeight?: number;
  testId?: string;
  /** Shown instead of rows when there are none; a node for a real empty state. */
  empty?: ReactNode;
  /** Shown instead of rows while the first load is on the way. */
  loading?: boolean;
  /** Rows picked with Shift-click, Ctrl-click or Ctrl A, for an action on all of them at once. */
  multi?: ReadonlySet<string>;
  onMulti?: (keys: Set<string>) => void;
}

export function useSort(initial: Sort | null = null): [Sort | null, (s: Sort | null) => void] {
  return useState<Sort | null>(initial);
}

export function sortRows<T>(rows: T[], columns: Col<T>[], sort: Sort | null): T[] {
  if (!sort) return rows;
  const col = columns.find((c) => c.id === sort.id);
  if (!col?.sort) return rows;
  const cmp = col.sort;
  const out = [...rows].sort(cmp);
  return sort.desc ? out.reverse() : out;
}

export const byString = <T,>(f: (r: T) => string) => (a: T, b: T) => f(a).localeCompare(f(b));
export const byNumber = <T,>(f: (r: T) => number) => (a: T, b: T) => f(a) - f(b);

interface RowProps<T> {
  row: T;
  dataKey: string;
  columns: Col<T>[];
  cls: string;
  height: number;
  onPick?: (row: T, e: React.MouseEvent) => void;
}

function RowInner<T>({ row, dataKey, columns, cls, height, onPick }: RowProps<T>) {
  return (
    <tr className={cls} style={{ height }} onClick={onPick ? (e) => onPick(row, e) : undefined} data-key={dataKey} aria-selected={cls.includes('selected') || cls.includes('checked') || undefined}>
      {columns.map((c) => (
        <td key={c.id} className={c.num ? 'num' : ''}>
          {c.cell(row)}
        </td>
      ))}
    </tr>
  );
}
// A row re-renders only when its data, its classes or the column set change, not on every scroll.
const Row = memo(RowInner) as typeof RowInner;

export function DataTable<T>({ columns, rows, rowKey, selected, onSelect, onActivate, onEscape, rowClass, sort, onSort, rowHeight = 22, testId, empty = 'nothing to show', loading, multi, onMulti }: DataTableProps<T>) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [widths, setWidths] = useState<Record<string, number>>({});
  const sorted = useMemo(() => sortRows(rows, columns, sort), [rows, columns, sort]);
  // The latest state behind a stable click handler, so rows keep their memo across clicks.
  const latest = useRef({ sorted, selected, multi, onSelect, onMulti, rowKey, anchor: null as string | null });
  latest.current = { ...latest.current, sorted, selected, multi, onSelect, onMulti, rowKey };
  const pick = useCallback((row: T, e: React.MouseEvent) => {
    const st = latest.current;
    const key = st.rowKey(row);
    if (st.onMulti && (e.shiftKey || e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      const next = new Set(st.multi ?? []);
      // The first modifier click starts the set from the row already selected, as a desk expects.
      if (next.size === 0 && st.selected && st.selected !== key) {
        next.add(st.selected);
        if (st.anchor === null) latest.current.anchor = st.selected;
      }
      if (e.shiftKey && st.anchor !== null) {
        const keys = st.sorted.map(st.rowKey);
        const a = keys.indexOf(st.anchor);
        const b = keys.indexOf(key);
        if (a >= 0 && b >= 0) for (let i = Math.min(a, b); i <= Math.max(a, b); i++) next.add(keys[i]);
        else next.add(key);
      } else if (next.has(key)) next.delete(key);
      else next.add(key);
      st.onMulti(next);
      if (!e.shiftKey) latest.current.anchor = key;
      return;
    }
    latest.current.anchor = key;
    if (st.onMulti && st.multi && st.multi.size) st.onMulti(new Set());
    st.onSelect?.(row, 'click');
  }, []);
  const virt = useVirtualizer({ count: sorted.length, getScrollElement: () => scrollRef.current, estimateSize: () => rowHeight, overscan: 12 });
  const items = virt.getVirtualItems();
  const total = virt.getTotalSize();
  const before = items.length ? items[0].start : 0;
  const after = items.length ? total - items[items.length - 1].end : 0;
  const widthOf = (c: Col<T>) => widths[c.id] ?? c.width;
  const tableWidth = columns.reduce((w, c) => w + widthOf(c), 0);

  const startResize = (c: Col<T>, e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const x0 = e.clientX;
    const w0 = widthOf(c);
    const move = (ev: MouseEvent) => setWidths((w) => ({ ...w, [c.id]: Math.max(40, w0 + ev.clientX - x0) }));
    const up = () => {
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  };

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (!sorted.length) return;
      const idx = selected == null ? -1 : sorted.findIndex((r) => rowKey(r) === selected);
      const pageStep = Math.max(1, Math.floor((scrollRef.current?.clientHeight ?? 400) / rowHeight) - 1);
      let next: number | null = null;
      switch (e.key) {
        case 'ArrowDown':
          next = Math.min(sorted.length - 1, idx + 1);
          break;
        case 'ArrowUp':
          next = Math.max(0, idx < 0 ? 0 : idx - 1);
          break;
        case 'PageDown':
          next = Math.min(sorted.length - 1, idx + pageStep);
          break;
        case 'PageUp':
          next = Math.max(0, idx - pageStep);
          break;
        case 'Home':
          next = 0;
          break;
        case 'End':
          next = sorted.length - 1;
          break;
        case 'Enter':
          if (idx >= 0) {
            e.preventDefault();
            (onActivate ?? ((r: T) => onSelect?.(r, 'key')))(sorted[idx]);
          }
          return;
        case 'Escape':
          if (onMulti && multi && multi.size) onMulti(new Set());
          onEscape?.();
          return;
        case 'a':
          if ((e.ctrlKey || e.metaKey) && onMulti) {
            e.preventDefault();
            onMulti(new Set(sorted.map(rowKey)));
          }
          return;
        default:
          return;
      }
      e.preventDefault();
      if (next !== null && next !== idx) {
        onSelect?.(sorted[next], 'key');
        virt.scrollToIndex(next, { align: 'auto' });
      }
    },
    [sorted, selected, rowKey, rowHeight, onSelect, onActivate, onEscape, virt, multi, onMulti],
  );

  return (
    <div className="tablewrap" ref={scrollRef} data-testid={testId} tabIndex={0} onKeyDown={onKeyDown} aria-label={`${sorted.length} rows; arrow keys move the selection, Enter opens it, Escape closes it`} aria-busy={loading || undefined}>
      <table className="table grid" style={{ width: tableWidth }}>
        <colgroup>
          {columns.map((c) => (
            <col key={c.id} style={{ width: widthOf(c) }} />
          ))}
        </colgroup>
        <thead>
          <tr>
            {columns.map((c) => {
              const active = sort?.id === c.id;
              return (
                <th key={c.id} className={c.num ? 'num' : ''} title={c.title} aria-sort={active ? (sort!.desc ? 'descending' : 'ascending') : undefined}>
                  {c.sort ? (
                    <button type="button" className={`th-sort${active ? ' active' : ''}`} onClick={() => onSort(active ? (sort!.desc ? null : { id: c.id, desc: true }) : { id: c.id, desc: false })}>
                      {c.header}
                      {active && <span aria-hidden>{sort!.desc ? ' ▼' : ' ▲'}</span>}
                    </button>
                  ) : (
                    c.header
                  )}
                  <div className="resizer" onMouseDown={(e) => startResize(c, e)} aria-hidden />
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody>
          {loading && sorted.length === 0 && (
            <tr>
              <td colSpan={columns.length} style={{ padding: 0 }}>
                <div className="skeleton-rows" aria-hidden>
                  {Array.from({ length: 14 }, (_, i) => (
                    <div key={i} className="skeleton-row" style={{ height: rowHeight }}>
                      <span className="skeleton" style={{ width: `${30 + ((i * 29) % 45)}%`, height: 10 }} />
                    </div>
                  ))}
                </div>
              </td>
            </tr>
          )}
          {!loading && sorted.length === 0 && (
            <tr>
              <td colSpan={columns.length} style={{ padding: 0, whiteSpace: 'normal' }}>
                {typeof empty === 'string' ? <div className="empty t-muted">{empty}</div> : empty}
              </td>
            </tr>
          )}
          {before > 0 && (
            <tr aria-hidden style={{ height: before }}>
              <td colSpan={columns.length} />
            </tr>
          )}
          {items.map((vi) => {
            const row = sorted[vi.index];
            const key = rowKey(row);
            const cls = [vi.index % 2 ? 'odd' : '', selected === key ? 'selected' : '', multi?.has(key) ? 'checked' : '', onSelect ? 'selectable' : '', rowClass?.(row) ?? ''].filter(Boolean).join(' ');
            return <Row key={key} row={row} dataKey={key} columns={columns} cls={cls} height={rowHeight} onPick={onSelect || onMulti ? pick : undefined} />;
          })}
          {after > 0 && (
            <tr aria-hidden style={{ height: after }}>
              <td colSpan={columns.length} />
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}

export type Density = 'compact' | 'comfortable';
export const ROW_HEIGHT: Record<Density, number> = { compact: 22, comfortable: 28 };

/** Which columns a table shows and how tall its rows are, remembered per table, with the menu
 * that changes them. */
export function useTableView<T>(name: string, all: Col<T>[]) {
  const [hidden, setHidden] = usePref<string[]>(`cols-${name}`, []);
  const [density, setDensity] = usePref<Density>('density', 'compact');
  const columns = useMemo(() => all.filter((c) => !hidden.includes(c.id)), [all, hidden]);
  const menu = <ViewMenu all={all} hidden={hidden} setHidden={setHidden} density={density} setDensity={setDensity} />;
  return { columns, rowHeight: ROW_HEIGHT[density], menu };
}

function ViewMenu<T>({ all, hidden, setHidden, density, setDensity }: { all: Col<T>[]; hidden: string[]; setHidden: (h: string[]) => void; density: Density; setDensity: (d: Density) => void }) {
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const ref = useOutsideClose<HTMLDivElement>(open, close);
  return (
    <div className="anchor" ref={ref}>
      <button type="button" className="btn btn-ghost btn-sm" onClick={() => setOpen((o) => !o)} aria-expanded={open} aria-haspopup="menu" title="columns and row height">
        View
      </button>
      {open && (
        <div className="popup" role="menu" style={{ minWidth: 200 }}>
          <div className="t-eyebrow">Rows</div>
          <div className="chips" style={{ margin: '4px 0 8px' }}>
            {(['compact', 'comfortable'] as Density[]).map((d) => (
              <button key={d} type="button" className={`chip chip-sm outlined${density === d ? ' selected' : ''}`} onClick={() => setDensity(d)} aria-pressed={density === d}>
                {d}
              </button>
            ))}
          </div>
          <div className="t-eyebrow">Columns</div>
          <div className="stack" style={{ marginTop: 4, gap: 2 }}>
            {all
              .filter((c) => !c.fixed)
              .map((c) => (
                <label key={c.id} className="row menu-check">
                  <input type="checkbox" checked={!hidden.includes(c.id)} onChange={(e) => setHidden(e.target.checked ? hidden.filter((h) => h !== c.id) : [...hidden, c.id])} />
                  {c.header}
                </label>
              ))}
          </div>
          {hidden.length > 0 && (
            <button type="button" className="menu-item t-small" onClick={() => setHidden([])} style={{ marginTop: 6 }}>
              Show all columns
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/** Downloads the rows as a CSV file named after the page and the day. */
export function exportCsv<T>(name: string, columns: Col<T>[], rows: T[]) {
  const esc = (s: string) => (/[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s);
  const text = (c: Col<T>, r: T) => {
    if (c.csv) return c.csv(r);
    const v = c.cell(r);
    return typeof v === 'string' || typeof v === 'number' ? String(v) : '';
  };
  const lines = [columns.map((c) => esc(c.header)).join(',')];
  for (const r of rows) lines.push(columns.map((c) => esc(text(c, r))).join(','));
  const blob = new Blob([`﻿${lines.join('\r\n')}`], { type: 'text/csv;charset=utf-8' });
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = `nbb-${name}-${new Date().toISOString().slice(0, 10)}.csv`;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}
