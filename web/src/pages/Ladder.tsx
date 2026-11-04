// Liquidity ladder: expected cash per value date for one currency, the cumulative balance as a
// line so a dip below zero is seen before it is read, nostro actuals where the user may see them,
// and a drill-down into the flows and movements behind a date.
import { useEffect, useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { endpoints } from '../api/client';
import { useFeeds, useLadder } from '../api/queries';
import type { LadderRow } from '../api/types';
import { fmtAmount, fmtDate, fmtDateShort, fmtLocal, fmtMoneyShort } from '../lib/fmt';
import { matchClass, matchLabel } from '../lib/deals';
import { usePref } from '../lib/prefs';
import { Chip } from '../components/ui';
import { LineChart } from '../components/charts';
import { referenceNow } from '../components/shell';
import { exportCsv, type Col } from '../components/table';
import { SideToggle, WithSide } from '../components/positions';
import { Empty, Kbd, SkeletonLines } from '../components/feedback';

const LEG: Record<string, string> = { FX_CCY1: 'fx leg 1', FX_CCY2: 'fx leg 2', MM_START: 'principal', MM_END: 'principal + interest' };

type Horizon = 14 | 35;

export function Ladder() {
  const [params, setParams] = useSearchParams();
  const ccy = params.get('ccy') ?? undefined;
  const ladder = useLadder(ccy);
  const feeds = useFeeds();
  const qc = useQueryClient();
  const [date, setDate] = useState<string | null>(null);
  const [horizon, setHorizon] = usePref<Horizon>('ladder-horizon', 35);
  const [quietToo, setQuietToo] = usePref<boolean>('ladder-quiet', true);
  const l = ladder.data;
  const today = referenceNow(feeds.data).slice(0, 10);
  const actuals = l?.liquidity_access ?? false;
  const setCcy = (c: string) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        n.set('ccy', c);
        return n;
      },
      { replace: true },
    );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setDate(null);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const rows = useMemo(() => {
    const all = l?.rows ?? [];
    const from = all.findIndex((r) => r.date >= today);
    const limit = from < 0 ? all.length : from + horizon;
    return all.filter((r, i) => i < limit && (quietToo || r.deals > 0 || r.actual_net !== null || r.date === today));
  }, [l, today, horizon, quietToo]);
  const maxAbs = Math.max(1, ...rows.map((r) => Math.abs(Number(r.net_expected))));
  const chart = useMemo(() => {
    const all = (l?.rows ?? []).slice(0, (l?.rows ?? []).findIndex((r) => r.date >= today) + horizon);
    return { points: all.map((r, i) => [i, Number(r.cumulative_expected)] as [number, number]), labels: all.map((r) => fmtDate(r.date)), todayIndex: all.findIndex((r) => r.date === today) };
  }, [l, today, horizon]);
  const lowest = rows.reduce<LadderRow | null>((m, r) => (m === null || Number(r.cumulative_expected) < Number(m.cumulative_expected) ? r : m), null);

  const csvCols: Col<LadderRow>[] = [
    { id: 'date', header: 'Value date', width: 100, cell: (r) => r.date },
    { id: 'cin', header: 'Confirmed in', width: 100, cell: (r) => r.confirmed_in },
    { id: 'cout', header: 'Confirmed out', width: 100, cell: (r) => r.confirmed_out },
    { id: 'pin', header: 'Provisional in', width: 100, cell: (r) => r.provisional_in },
    { id: 'pout', header: 'Provisional out', width: 100, cell: (r) => r.provisional_out },
    { id: 'net', header: 'Net expected', width: 100, cell: (r) => r.net_expected },
    { id: 'cum', header: 'Cumulative', width: 100, cell: (r) => r.cumulative_expected },
    { id: 'act', header: 'Actual', width: 100, cell: (r) => r.actual_net ?? '' },
    { id: 'diff', header: 'Difference', width: 100, cell: (r) => r.difference ?? '' },
    { id: 'deals', header: 'Deals', width: 60, cell: (r) => String(r.deals) },
  ];

  return (
    <WithSide page="liquidity" ready={l !== undefined}>
      <div className="toolbar">
        <h1 className="t-title" style={{ marginRight: 4 }}>
          Liquidity ladder
        </h1>
        <div className="chips" role="radiogroup" aria-label="Currency">
          {(l?.currencies ?? []).map((c) => (
            <Chip key={c} selected={(ccy ?? l?.ccy) === c} onClick={() => setCcy(c)}>
              {c}
            </Chip>
          ))}
        </div>
        <span className="vsep" />
        <div className="chips" role="radiogroup" aria-label="Horizon">
          <Chip small outlined selected={horizon === 14} onClick={() => setHorizon(14)}>
            2 weeks
          </Chip>
          <Chip small outlined selected={horizon === 35} onClick={() => setHorizon(35)}>
            5 weeks
          </Chip>
        </div>
        <Chip small outlined selected={!quietToo} onClick={() => setQuietToo((q) => !q)} title="hide days with no expected flow and no nostro movement">
          days with flows only
        </Chip>
        <span className="spacer" />
        {l && (
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => exportCsv(`ladder-${l.ccy}`, csvCols, l.rows)}>
            Export CSV
          </button>
        )}
        <SideToggle />
        {l && <span className="t-small">as of {fmtLocal(l.generated_at)}</span>}
        <button type="button" className="btn btn-sm" onClick={() => void qc.invalidateQueries({ queryKey: ['ladder'] })}>
          Refresh
        </button>
      </div>
      {!l ? (
        <div style={{ padding: 8 }}>
          <SkeletonLines n={8} />
        </div>
      ) : (
        <>
          <div className="row-wrap" style={{ alignItems: 'baseline', marginBottom: 6 }}>
            {l.opening ? (
              <>
                <span className="t-muted">Opening</span>
                <span className="t-medium">{fmtAmount(l.opening.balance)}</span>
                <span className="t-small">
                  {l.ccy} {l.opening.kind} of {fmtDateShort(l.opening.statement_date)} ({l.opening.bank}, {l.opening.account_no})
                </span>
              </>
            ) : l.liquidity_access ? (
              <span className="t-small">No nostro statement on file for this currency; cumulative starts at zero.</span>
            ) : (
              <span className="t-small">Expected flows from your desks' deals. Nostro balances and actual movements need ALM or treasury-head access.</span>
            )}
            {lowest && (
              <>
                <span className="t-muted" style={{ marginLeft: 12 }}>
                  Lowest point
                </span>
                <span className={`t-medium ${Number(lowest.cumulative_expected) < 0 ? 't-bad' : ''}`}>{fmtAmount(lowest.cumulative_expected)}</span>
                <span className="t-small">on {fmtDate(lowest.date)}</span>
              </>
            )}
          </div>
          <div className="ladder-chart">
            <LineChart series={[{ name: `${l.ccy} cumulative expected`, points: chart.points }]} height={120} yFmt={fmtMoneyShort} xFmt={(x) => chart.labels[Math.round(x)] ?? ''} refY={0} markX={chart.todayIndex >= 0 ? chart.todayIndex : undefined} markLabel="today" />
          </div>
          <div className="tablewrap" data-testid="ladder">
            {rows.length === 0 ? (
              <Empty title="No days with flows in this horizon" detail="Turn the filter off to see every value date.">
                <button type="button" className="btn btn-sm" onClick={() => setQuietToo(true)}>
                  Show all days
                </button>
              </Empty>
            ) : (
              <table className="table grid" style={{ width: actuals ? 1306 : 1056 }}>
                <thead>
                  <tr>
                    <th style={{ width: 120 }}>Value date</th>
                    <th className="num" style={{ width: 120 }}>
                      Confirmed in
                    </th>
                    <th className="num" style={{ width: 120 }}>
                      Confirmed out
                    </th>
                    <th className="num" style={{ width: 120 }}>
                      Provisional in
                    </th>
                    <th className="num" style={{ width: 120 }}>
                      Provisional out
                    </th>
                    <th className="num" style={{ width: 130 }}>
                      Net expected
                    </th>
                    <th style={{ width: 110 }} />
                    <th className="num" style={{ width: 150 }}>
                      Cumulative
                    </th>
                    {actuals && (
                      <th className="num" style={{ width: 130 }}>
                        Actual (nostro)
                      </th>
                    )}
                    {actuals && (
                      <th className="num" style={{ width: 120 }}>
                        Difference
                      </th>
                    )}
                    <th className="num" style={{ width: 66 }}>
                      Deals
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((r, i) => {
                    const quiet = r.deals === 0 && r.actual_net === null;
                    const dim = (s: string) => <span className={quiet ? 't-muted' : ''}>{s}</span>;
                    const net = Number(r.net_expected);
                    const cum = Number(r.cumulative_expected);
                    const frac = Math.max(-1, Math.min(1, net / maxAbs));
                    return (
                      <tr key={r.date} className={`selectable ${i % 2 ? 'odd' : ''} ${date === r.date ? 'selected' : ''}`} onClick={() => setDate((d) => (d === r.date ? null : r.date))}>
                        <td className={r.date === today ? 't-medium t-warn' : r.date < today ? 't-muted' : ''} title={r.date === today ? 'today' : undefined}>
                          {fmtDate(r.date)}
                          {r.date === today && <span className="t-small"> today</span>}
                        </td>
                        <td className="num">{dim(fmtAmount(r.confirmed_in))}</td>
                        <td className="num">{dim(fmtAmount(r.confirmed_out))}</td>
                        <td className="num">{Number(r.provisional_in) === 0 ? dim('0') : <span className="t-warn">{fmtAmount(r.provisional_in)}</span>}</td>
                        <td className="num">{Number(r.provisional_out) === 0 ? dim('0') : <span className="t-warn">{fmtAmount(r.provisional_out)}</span>}</td>
                        <td className={`num t-medium ${net < 0 ? 't-bad' : net === 0 ? 't-muted' : 't-good'}`}>{fmtAmount(r.net_expected)}</td>
                        <td>
                          <div className="bar-cell">
                            <div className="mid" />
                            {frac !== 0 && <div className="bar" style={{ left: frac > 0 ? '50%' : `${50 - Math.abs(frac) * 48}%`, width: `${Math.abs(frac) * 48}%`, background: frac > 0 ? 'var(--good)' : 'var(--bad)' }} />}
                          </div>
                        </td>
                        <td className={`num ${cum < 0 ? 't-bad' : ''}`}>{fmtAmount(r.cumulative_expected)}</td>
                        {actuals && <td className="num">{r.actual_net === null ? <span className="t-muted">—</span> : fmtAmount(r.actual_net)}</td>}
                        {actuals && (
                          <td className="num">
                            {r.difference === null ? <span className="t-muted">—</span> : Number(r.difference) === 0 ? <span className="t-good">0</span> : <span className={Math.abs(Number(r.difference)) > 1000 ? 't-bad' : 't-warn'}>{fmtAmount(r.difference)}</span>}
                          </td>
                        )}
                        <td className="num">{dim(String(r.deals))}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
          </div>
          {date ? (
            <DateDetail ccy={l.ccy} date={date} access={l.liquidity_access} onClose={() => setDate(null)} />
          ) : (
            <div className="hint t-small">
              click a value date for the deals and nostro movements behind it · <Kbd>Esc</Kbd> closes it
            </div>
          )}
        </>
      )}
    </WithSide>
  );
}

function DateDetail({ ccy, date, access, onClose }: { ccy: string; date: string; access: boolean; onClose: () => void }) {
  const navigate = useNavigate();
  const flows = useQuery({ queryKey: ['flows', ccy, date], queryFn: () => endpoints.flows(ccy, date) });
  const moves = useQuery({ queryKey: ['movements', ccy, date], queryFn: () => endpoints.movements(ccy, date), enabled: access });
  const f = flows.data ?? [];
  const m = moves.data ?? [];
  const [y, mo, d] = date.split('-').map(Number);
  const long = new Date(Date.UTC(y, mo - 1, d)).toLocaleDateString('en-GB', { weekday: 'long', day: '2-digit', month: 'long', year: 'numeric', timeZone: 'UTC' });
  return (
    <div className="detail" style={{ maxHeight: 220 }}>
      <div className="head">
        <span className="t-heading">
          {ccy} on {long}
        </span>
        <span className="t-small">
          {f.length} expected flows · {m.length} nostro movements
        </span>
        <span style={{ flex: 1 }} />
        <button type="button" className="btn btn-ghost btn-sm" onClick={onClose} aria-label="close" title="close (Esc)">
          ✕
        </button>
      </div>
      <div className="two-up">
        <div className="stack">
          <span className="t-label">Expected flows</span>
          {f.map((x) => (
            <div key={`${x.deal_id}-${x.leg}`} className="row" style={{ fontSize: 12 }}>
              <button type="button" className="btn btn-ghost btn-sm" style={{ color: 'var(--info)', padding: 0 }} onClick={() => navigate(`/blotter?deal=${x.deal_id}`)}>
                {x.deal_ref}
              </button>
              <span className="t-small">{x.desk_code}</span>
              <span className="t-small">{LEG[x.leg] ?? 'flow'}</span>
              <span className={Number(x.amount) < 0 ? 't-bad' : 't-good'}>{fmtAmount(x.amount)}</span>
              <span className={`t-small ${x.confirmed ? 't-good' : 't-warn'}`}>{x.confirmed ? 'confirmed' : 'provisional'}</span>
              <span className={`t-small ${matchClass(x.match_status)}`}>{matchLabel(x.match_status)}</span>
            </div>
          ))}
          {flows.isLoading && <SkeletonLines n={3} />}
          {flows.data && f.length === 0 && <span className="t-small">none</span>}
        </div>
        <div className="stack">
          <span className="t-label">Nostro movements</span>
          {!access && <span className="t-small">not visible to this user</span>}
          {access && moves.data && m.length === 0 && <span className="t-small">none reported</span>}
          {m.map((x) => (
            <div key={x.id} className="row" style={{ fontSize: 12 }}>
              <span className="t-small">{x.reference}</span>
              <span className={Number(x.amount) < 0 ? 't-bad' : 't-good'}>{fmtAmount(x.amount)}</span>
              {x.deal_ref_hint ? <span className="t-small">for {x.deal_ref_hint}</span> : <span className="t-small t-warn">{x.source}</span>}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
