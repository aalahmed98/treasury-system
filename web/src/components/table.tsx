// A virtualised, sortable, resizable table over pre-formatted rows, plus CSV export. Columns are
// plain objects: the pages describe what to show, the table does the rest.
import { useMemo, useRef, useState, type ReactNode } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';

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
}

export interface Sort {
  id: string;
  desc: boolean;
}

export interface DataTableProps<T> {
  columns: Col<T>[];
  rows: T[];
  rowKey: (row: T) => string;
  selected?: string | null;
  onSelect?: (row: T) => void;
  rowClass?: (row: T) => string;
  sort: Sort | null;
  onSort: (s: Sort | null) => void;
  rowHeight?: number;
  testId?: string;
  empty?: string;
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

export function DataTable<T>({ columns, rows, rowKey, selected, onSelect, rowClass, sort, onSort, rowHeight = 22, testId, empty = 'nothing to show' }: DataTableProps<T>) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [widths, setWidths] = useState<Record<string, number>>({});
  const sorted = useMemo(() => sortRows(rows, columns, sort), [rows, columns, sort]);
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

  return (
    <div className="tablewrap" ref={scrollRef} data-testid={testId}>
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
                  <div className="resizer" onMouseDown={(e) => startResize(c, e)} />
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody>
          {sorted.length === 0 && (
            <tr>
              <td colSpan={columns.length} className="t-muted" style={{ height: 32 }}>
                {empty}
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
            const cls = [vi.index % 2 ? 'odd' : '', selected === key ? 'selected' : '', onSelect ? 'selectable' : '', rowClass?.(row) ?? ''].filter(Boolean).join(' ');
            return (
              <tr key={key} className={cls} style={{ height: rowHeight }} onClick={() => onSelect?.(row)} data-key={key}>
                {columns.map((c) => (
                  <td key={c.id} className={c.num ? 'num' : ''}>
                    {c.cell(row)}
                  </td>
                ))}
              </tr>
            );
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
