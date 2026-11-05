// The FTP curves in force, for every desk: one table of every tenor against every currency with
// the move since the previous publication, the curves drawn side by side, one rate's history, and
// the publications behind them. ALM publishes from the workbench (ALM, FTP).
import { useMemo, useState } from 'react';
import { Link } from 'react-router';
import { endpoints } from '../api/client';
import { useFtp, useFtpHistory } from '../api/queries';
import type { FtpCurve, FtpPublication } from '../api/types';
import { useSession } from '../api/session';
import { fmtDateLong, fmtDateShort, fmtLocal } from '../lib/fmt';
import { Badge, Chip, Section } from '../components/ui';
import { Empty, SkeletonLines, useToast } from '../components/feedback';
import { LineChart, SERIES } from '../components/charts';

/** A currency keeps its colour wherever it is drawn: the four the bank publishes first, in order. */
const ORDER = ['BHD', 'USD', 'AED', 'SAR'];
export function ccyColor(ccy: string, all: string[]): string {
  const i = ORDER.indexOf(ccy);
  if (i >= 0) return SERIES[i];
  const extra = all.filter((c) => !ORDER.includes(c)).sort();
  return SERIES[(ORDER.length + extra.indexOf(ccy)) % SERIES.length];
}

const pct = (v: number) => `${v.toFixed(3)}%`;

/** Every tenor any curve has, shortest first, as [days, label]. */
export function tenorsOf(curves: FtpCurve[]): [number, string][] {
  const m = new Map<number, string>();
  for (const c of curves) for (const p of c.points) if (!m.has(p.tenor_days)) m.set(p.tenor_days, p.tenor);
  return [...m.entries()].sort((a, b) => a[0] - b[0]);
}

/** Tenors down the side, currencies across; each rate with its move in basis points beside it. */
export function FtpTable({ curves, previous, testId }: { curves: FtpCurve[]; previous?: FtpCurve[] | null; testId?: string }) {
  const tenors = tenorsOf(curves);
  const before = useMemo(() => {
    const m = new Map<string, number>();
    for (const c of previous ?? []) for (const p of c.points) m.set(`${c.ccy}:${p.tenor_days}`, p.rate);
    return m;
  }, [previous]);
  return (
    <table className="table ftp-table" data-testid={testId}>
      <thead>
        <tr>
          <th>Tenor</th>
          {curves.map((c) => (
            <th key={c.ccy} className="num" colSpan={previous ? 2 : 1}>
              {c.ccy}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {tenors.map(([days, label]) => (
          <tr key={days}>
            <td className="t-medium">{label}</td>
            {curves.map((c) => {
              const p = c.points.find((x) => x.tenor_days === days);
              const was = before.get(`${c.ccy}:${days}`);
              const bp = p && was !== undefined ? Math.round((p.rate - was) * 100) : null;
              return [
                <td key={c.ccy} className="num">
                  {p ? pct(p.rate) : <span className="t-muted">—</span>}
                </td>,
                previous ? (
                  <td key={`${c.ccy}-d`} className={`num t-small delta ${bp === null || bp === 0 ? 't-muted' : Math.abs(bp) > 25 ? 't-warn' : ''}`}>
                    {bp === null ? (p ? 'new' : '') : bp === 0 ? '0' : `${bp > 0 ? '+' : '−'}${Math.abs(bp)}`}
                  </td>
                ) : null,
              ];
            })}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** The curves drawn across evenly spaced tenors, so the short end is not squeezed into a corner. */
export function FtpCurvesChart({ curves, height = 190 }: { curves: FtpCurve[]; height?: number }) {
  const tenors = tenorsOf(curves);
  const idx = new Map(tenors.map(([d], i) => [d, i]));
  const all = curves.map((c) => c.ccy);
  return (
    <LineChart
      height={height}
      series={curves.map((c) => ({ name: c.ccy, color: ccyColor(c.ccy, all), points: c.points.map((p) => [idx.get(p.tenor_days) ?? 0, p.rate] as [number, number]) }))}
      yFmt={(y) => `${y.toFixed(2)}%`}
      xFmt={(x) => tenors[Math.round(x)]?.[1] ?? ''}
    />
  );
}

function PublishedBy({ p }: { p: FtpPublication }) {
  return (
    <span className="t-small">
      published {fmtLocal(p.published_at)} by {p.published_by ?? 'the application'}
      {p.file_name ? ` from ${p.file_name}` : ''}
      {p.superseded > 0 ? `, replacing ${p.superseded} earlier ${p.superseded === 1 ? 'publication' : 'publications'} that day` : ''}
    </span>
  );
}

export function Ftp() {
  const q = useFtp();
  const { canSeeAlm } = useSession();
  const toast = useToast();
  const c = q.data;
  const cur = c?.current ?? null;
  const ccys = cur?.curves.map((x) => x.ccy) ?? [];
  const [ccy, setCcy] = useState<string | null>(null);
  const [tenor, setTenor] = useState('1Y');
  const shownCcy = ccy && ccys.includes(ccy) ? ccy : (ccys.find((x) => x === 'BHD') ?? ccys[0] ?? '');
  const tenorList = cur ? tenorsOf(cur.curves.filter((x) => x.ccy === shownCcy)) : [];
  const shownTenor = tenorList.some(([, t]) => t === tenor) ? tenor : (tenorList.find(([d]) => d >= 365)?.[1] ?? tenorList[0]?.[1] ?? '');
  const hist = useFtpHistory(shownCcy, shownTenor, 120);

  if (!c) {
    return (
      <div className="page">
        {q.isError ? (
          <Empty tone="warn" title="The FTP curves could not be loaded">
            <button type="button" className="btn btn-sm" onClick={() => void q.refetch()}>
              Try again
            </button>
          </Empty>
        ) : (
          <SkeletonLines n={8} />
        )}
      </div>
    );
  }
  return (
    <div className="page" data-ready="1" data-testid="ftp-page">
      <div className="page-title">
        <h1 className="t-title">FTP curves</h1>
        {cur && <span className="t-small">in force on {fmtDateLong(c.date)}</span>}
        {cur && (c.stale ? <Badge tone="warn">published {fmtDateShort(cur.as_of)}, not yet for {fmtDateShort(c.date)}</Badge> : <Badge tone="good">today's</Badge>)}
        <span className="grow" />
        {cur && (
          <button type="button" className="btn btn-sm" onClick={() => endpoints.ftpTemplate().catch((e: Error) => toast({ tone: 'bad', title: 'The download failed', detail: e.message }))}>
            Download as Excel
          </button>
        )}
        {c.can_publish && canSeeAlm && (
          <Link to="/alm/ftp" className="btn btn-sm btn-primary">
            Publish new curves
          </Link>
        )}
      </div>
      {!cur ? (
        <Empty title="ALM has not published FTP curves yet">Funding is charged to the desks once ALM publishes its first workbook.</Empty>
      ) : (
        <>
          <p className="t-secondary prose">
            The rate each desk is charged, or credited, for funds of a given term in each currency. A bond bought today is charged the rate for its term from
            today's curve for as long as it is held.
          </p>
          <div className="row" style={{ gap: 6 }}>
            <PublishedBy p={cur} />
            {cur.note && <span className="t-small">· {cur.note}</span>}
          </div>
          <div className="panel-row two" style={{ marginTop: 10 }}>
            <div className="panel">
              <div className="t-heading">Rates, percent{c.previous ? `, with the move since ${fmtDateShort(c.previous.as_of)} in bp` : ''}</div>
              <FtpTable curves={cur.curves} previous={c.previous?.curves} testId="ftp-table" />
            </div>
            <div className="panel">
              <div className="t-heading">The curves</div>
              <FtpCurvesChart curves={cur.curves} />
            </div>
          </div>

          <Section title="History" meta="one rate over the last four months, from each day's publication" />
          <div className="panel">
            <div className="row" style={{ gap: 4, flexWrap: 'wrap' }}>
              {ccys.map((x) => (
                <Chip key={x} small selected={x === shownCcy} onClick={() => setCcy(x)}>
                  {x}
                </Chip>
              ))}
              <span className="vsep" />
              <select className="input input-sm" value={shownTenor} onChange={(e) => setTenor(e.target.value)} aria-label="Tenor">
                {tenorList.map(([d, t]) => (
                  <option key={d} value={t}>
                    {t}
                  </option>
                ))}
              </select>
            </div>
            {hist.data && hist.data.points.length > 0 ? (
              <LineChart
                height={170}
                series={[{ name: `${shownCcy} ${shownTenor}`, color: ccyColor(shownCcy, ccys), points: hist.data.points.map((p, i) => [i, p.rate] as [number, number]) }]}
                yFmt={(y) => `${y.toFixed(2)}%`}
                xFmt={(x) => (hist.data.points[Math.round(x)] ? fmtDateShort(hist.data.points[Math.round(x)].as_of) : '')}
              />
            ) : (
              <div className="t-muted" style={{ padding: 12 }}>
                {hist.isLoading ? 'loading…' : 'no history for this rate'}
              </div>
            )}
          </div>

          <Section title="Publications" meta="the latest fifteen; a corrected workbook replaces the day's earlier one, which is kept" />
          <table className="table" data-testid="ftp-recent">
            <thead>
              <tr>
                <th>For</th>
                <th>Published</th>
                <th>By</th>
                <th>Currencies</th>
                <th>File</th>
                <th>Note</th>
              </tr>
            </thead>
            <tbody>
              {c.recent.map((r) => (
                <tr key={r.id} className={r.is_current ? '' : 't-muted'}>
                  <td>
                    {fmtDateShort(r.as_of)} {!r.is_current && <Badge>replaced</Badge>}
                  </td>
                  <td>{fmtLocal(r.published_at)}</td>
                  <td>{r.published_by ?? 'application'}</td>
                  <td>{r.currencies.join(', ')}</td>
                  <td className="t-small">{r.file_name ?? '—'}</td>
                  <td className="t-small ellipsis" style={{ maxWidth: 380 }} title={r.note ?? ''}>
                    {r.note ?? ''}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </div>
  );
}
