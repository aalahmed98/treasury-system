// Pieces the five PMU pages share: number formats, the period picker, the loading and error
// states, and the full-year income by portfolio table (on Income and on Funding).
import type { ReactNode } from 'react';
import { useSearchParams } from 'react-router';
import type { BookYear, PmuIncomeParts } from '../../api/types';
import { fmtMoneyShort, signed } from '../../lib/fmt';
import { Chip } from '../../components/ui';
import { Empty, SkeletonLines, SkeletonStrip } from '../../components/feedback';

/** BHD, short: 639m, 1.82m, 38.0k. */
export const bhd = (v: number) => fmtMoneyShort(v);
/** DV01 in BHD per basis point, signed: "-258k". */
export const dv01 = (v: number) => signed(v);
export const pct = (v: number | null | undefined, dp = 2) => (v === null || v === undefined || !Number.isFinite(v) ? '—' : `${v.toFixed(dp)}%`);
export const share = (v: number) => `${(v * 100).toFixed(1)}%`;

/** A change since the previous business day, for a strip's sub line. */
export function since(now: number, prev: number | undefined, fmt: (v: number) => string = signed): string {
  if (prev === undefined) return '';
  const d = now - prev;
  if (Math.abs(d) < 0.5) return 'unchanged since yesterday';
  return `${d > 0 ? '+' : ''}${fmt(d).replace(/^\+/, '')} since yesterday`;
}

const day = (s: string) => new Date(`${s}T00:00:00Z`);
const iso = (d: Date) => d.toISOString().slice(0, 10);

export type Preset = 'mtd' | 'qtd' | 'ytd' | '30d' | 'custom';
export const PRESETS: [Preset, string][] = [
  ['mtd', 'Month to date'],
  ['qtd', 'Quarter to date'],
  ['ytd', 'Year to date'],
  ['30d', 'Last 30 days'],
];

export function presetRange(p: Preset, today: string): { from: string; to: string } {
  const t = day(today);
  const y = t.getUTCFullYear();
  const m = t.getUTCMonth();
  switch (p) {
    case 'qtd':
      return { from: iso(new Date(Date.UTC(y, m - (m % 3), 1))), to: today };
    case 'ytd':
      return { from: `${y}-01-01`, to: today };
    case '30d':
      return { from: iso(new Date(t.getTime() - 29 * 86_400_000)), to: today };
    default:
      return { from: iso(new Date(Date.UTC(y, m, 1))), to: today };
  }
}

/** The period in the address bar (`?p=ytd` or `?from=&to=`), so a view can be shared as a link. */
export function usePeriod(today: string | undefined): { preset: Preset; from?: string; to?: string; set: (p: Preset, from?: string, to?: string) => void } {
  const [params, setParams] = useSearchParams();
  const raw = params.get('p') as Preset | null;
  const preset: Preset = params.get('from') ? 'custom' : raw && PRESETS.some(([k]) => k === raw) ? raw : 'mtd';
  const range = preset === 'custom' ? { from: params.get('from') ?? undefined, to: params.get('to') ?? today } : today ? presetRange(preset, today) : { from: undefined, to: undefined };
  const set = (p: Preset, from?: string, to?: string) => {
    const next = new URLSearchParams(params);
    next.delete('p');
    next.delete('from');
    next.delete('to');
    if (p === 'custom' && from && to) {
      next.set('from', from);
      next.set('to', to);
    } else if (p !== 'mtd') next.set('p', p);
    setParams(next, { replace: true });
  };
  return { preset, from: range.from, to: range.to, set };
}

export function PeriodPicker({ today, period }: { today: string; period: ReturnType<typeof usePeriod> }) {
  return (
    <div className="row period" style={{ gap: 4, flexWrap: 'wrap' }} data-testid="period">
      {PRESETS.map(([k, label]) => (
        <Chip key={k} small selected={period.preset === k} onClick={() => period.set(k)}>
          {label}
        </Chip>
      ))}
      <span className="vsep" />
      <input
        className="input input-sm"
        type="date"
        aria-label="From"
        max={period.to ?? today}
        value={period.from ?? ''}
        onChange={(e) => e.target.value && period.set('custom', e.target.value, period.to ?? today)}
      />
      <span className="t-small">to</span>
      <input
        className="input input-sm"
        type="date"
        aria-label="To"
        max={today}
        min={period.from}
        value={period.to ?? ''}
        onChange={(e) => e.target.value && period.from && period.set('custom', period.from, e.target.value)}
      />
    </div>
  );
}

export function Loading<T>({ q, children, cells = 6 }: { q: { data?: T; isError: boolean; error: Error | null; refetch: () => unknown }; children: (d: T) => ReactNode; cells?: number }) {
  if (q.data) return <>{children(q.data)}</>;
  if (q.isError)
    return (
      <Empty tone="warn" title="The portfolio could not be loaded" detail={q.error?.message}>
        <button type="button" className="btn btn-sm" onClick={() => void q.refetch()}>
          Try again
        </button>
      </Empty>
    );
  return (
    <>
      <SkeletonStrip cells={cells} />
      <div style={{ marginTop: 12 }}>
        <SkeletonLines n={8} />
      </div>
    </>
  );
}

/** This year's income by portfolio: what is booked to date, what the rest of the year adds. */
export function BookYearTable({ rows, names, testId }: { rows: BookYear[]; names: Record<string, string>; testId?: string }) {
  const total = rows.reduce(
    (t, r) => ({ a: t.a + r.actual.net_nii, f: t.f + r.forecast.net_nii, c: t.c + r.actual.carry + r.forecast.carry }),
    { a: 0, f: 0, c: 0 },
  );
  const max = Math.max(1, ...rows.map((r) => Math.abs(r.actual.net_nii) + Math.abs(r.forecast.net_nii)));
  return (
    <table className="table" data-testid={testId}>
      <thead>
        <tr>
          <th>Portfolio</th>
          <th className="num">To date</th>
          <th className="num">Rest of year</th>
          <th className="num">Full year NII</th>
          <th className="num">Full year carry after FTP</th>
          <th style={{ width: 140 }} />
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.book}>
            <td>{names[r.book] ?? r.book}</td>
            <td className="num">{bhd(r.actual.net_nii)}</td>
            <td className="num t-secondary">{bhd(r.forecast.net_nii)}</td>
            <td className="num t-medium">{bhd(r.actual.net_nii + r.forecast.net_nii)}</td>
            <td className="num">{bhd(r.actual.carry + r.forecast.carry)}</td>
            <td>
              <span className="split-bar" title="booked to date, then forecast">
                <span style={{ width: `${(100 * Math.abs(r.actual.net_nii)) / max}%` }} />
                <span className="light" style={{ width: `${(100 * Math.abs(r.forecast.net_nii)) / max}%` }} />
              </span>
            </td>
          </tr>
        ))}
      </tbody>
      <tfoot>
        <tr>
          <td>Total</td>
          <td className="num">{bhd(total.a)}</td>
          <td className="num">{bhd(total.f)}</td>
          <td className="num">{bhd(total.a + total.f)}</td>
          <td className="num">{bhd(total.c)}</td>
          <td />
        </tr>
      </tfoot>
    </table>
  );
}

/** The parts of income in one row of a table. */
export const INCOME_COLS: [keyof PmuIncomeParts, string][] = [
  ['coupon', 'Coupon'],
  ['amortisation', 'Amortisation'],
  ['ftp', 'FTP'],
  ['carry', 'Carry after FTP'],
  ['irs', 'IRS carry'],
  ['net_nii', 'Net NII'],
];
