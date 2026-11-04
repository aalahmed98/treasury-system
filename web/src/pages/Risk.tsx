// FX risk: what the dealing book is worth, how much it made, how much it could lose, and every
// figure against the bank's limits. The page reads one server view (/risk/fx) computed from the
// deals the user may see and the market data; the history and breaches come alongside.
import { useState } from 'react';
import { Link, Navigate, Route, Routes, useNavigate } from 'react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ApiError, endpoints } from '../api/client';
import { useBreaches, useFxHistory, useFxRisk } from '../api/queries';
import { useSession } from '../api/session';
import type { Breach, FxRiskResult, LimitStatus, LimitUse } from '../api/types';
import { fmtAmount, fmtDate, fmtDateLong, fmtMoneyShort, signed } from '../lib/fmt';
import { Badge, Dot, Gauge, Section, StatStrip, type StatCell, type Tone } from '../components/ui';
import { Bars, LineChart } from '../components/charts';
import { Empty, SkeletonLines, SkeletonStrip, useToast } from '../components/feedback';
import { exportCsv, type Col } from '../components/table';
import { LimitsPage } from './RiskLimits';

export const statusTone = (s: LimitStatus): Tone => (s === 'BREACH' ? 'bad' : s === 'WATCH' ? 'warn' : 'good');
const STATUS_WORD: Record<LimitStatus, string> = { OK: 'within', WATCH: 'watch', BREACH: 'breach' };

export function Risk() {
  const { canSeeRisk } = useSession();
  if (!canSeeRisk) return <Navigate to="/" replace />;
  return (
    <Routes>
      <Route index element={<RiskPage />} />
      <Route path="limits" element={<LimitsPage />} />
      <Route path="*" element={<Navigate to="/risk" replace />} />
    </Routes>
  );
}

function use(r: FxRiskResult, key: string): LimitUse | undefined {
  return r.limits.find((l) => l.key === key);
}

/** "93% of 120m" with the standing in words. */
function against(l: LimitUse | undefined): string {
  if (!l) return 'no limit set';
  return `${STATUS_WORD[l.status]} · ${l.pct.toFixed(0)}% of ${fmtMoneyShort(l.limit)}`;
}

function RiskPage() {
  const navigate = useNavigate();
  const risk = useFxRisk();
  const history = useFxHistory(30);
  const r = risk.data;
  if (!r) {
    return (
      <div className="page">
        <div className="page-title">
          <h1 className="t-title">FX risk</h1>
        </div>
        {risk.isError ? (
          <Empty tone="warn" title="The risk view could not be loaded" detail={risk.error instanceof Error ? risk.error.message : 'the server did not answer'}>
            <button type="button" className="btn btn-sm" onClick={() => void risk.refetch()}>
              Try again
            </button>
          </Empty>
        ) : (
          <SkeletonStrip cells={4} />
        )}
      </div>
    );
  }
  const stopDay = use(r, 'stop.daily');
  const stopMonth = use(r, 'stop.monthly');
  const nop = use(r, 'nop.total');
  const intraday = use(r, 'nop.intraday');
  const varUse = use(r, 'var');
  const tone = (l?: LimitUse): Tone | null => (l ? statusTone(l.status) : null);
  const hero: StatCell[] = [
    { label: 'P&L today', value: signed(r.pnl.today), sub: `new deals ${signed(r.pnl.new_deals)} · market ${signed(r.pnl.market_move)}`, tone: r.pnl.today < 0 ? tone(stopDay) : 'good' },
    { label: 'P&L month to date', value: signed(r.pnl.mtd), sub: r.pnl.mtd < 0 ? `stop-loss ${against(stopMonth)}` : `stop-loss ${fmtMoneyShort(stopMonth?.limit ?? 0)} untouched`, tone: r.pnl.mtd < 0 ? tone(stopMonth) : 'good' },
    { label: 'Net open position', value: fmtMoneyShort(r.nop_total), sub: against(nop), tone: tone(nop) },
    { label: 'Value at risk, 1 day 99%', value: fmtMoneyShort(r.var.var99), sub: against(varUse), tone: tone(varUse) },
  ];
  const strip: StatCell[] = [
    { label: 'P&L year to date', value: signed(r.pnl.ytd), sub: `${signed(r.pnl.total)} on everything traded` },
    { label: 'Open contracts', value: signed(r.pnl.open_mtm), sub: 'mark-to-market of unsettled deals' },
    { label: 'Floating currencies', value: fmtMoneyShort(r.nop_floating), sub: 'open position outside the pegs' },
    { label: 'Pegged currencies', value: fmtMoneyShort(r.nop_pegged), sub: 'USD, SAR, AED: peg risk only' },
    { label: 'Intraday peak', value: fmtMoneyShort(r.nop_intraday_peak), sub: against(intraday), tone: tone(intraday) },
    { label: 'Off-market deals', value: String(r.off_market.length), sub: `${r.cancelled_30d} cancelled in 30 days`, tone: r.off_market.length ? 'warn' : 'good' },
  ];
  return (
    <div className="page" data-ready="1" data-testid="risk-page">
      <div className="page-title">
        <h1 className="t-title">FX risk</h1>
        <span className="t-small">
          as of {fmtDateLong(r.as_of)} · limits v{r.limits_version ?? '—'} · in {r.base_ccy}
          {risk.isFetching && ' · updating…'}
        </span>
        <span className="grow" />
        <Link className="btn btn-sm" to="/risk/limits">
          Limits
        </Link>
      </div>
      <StatStrip cells={hero} hero testId="risk-hero" />
      <StatStrip cells={strip} testId="risk-strip" />
      {r.unpriced.length > 0 && <div className="notice warn">No market rate for {r.unpriced.join(', ')}: their deals are left out of the valuation and the position totals.</div>}

      <LimitsSection r={r} />

      <Section title="Positions" meta="net amount per currency from every live FX deal, and what is still to settle by tenor" />
      <div className="panel-row two">
        <div className="panel">
          <PositionsTable r={r} />
        </div>
        <div className="panel">
          <div className="t-heading">Forward ladder, BHD</div>
          {r.ladder.rows.length === 0 ? (
            <div className="t-muted">nothing left to settle</div>
          ) : (
            <table className="table" data-testid="fwd-ladder">
              <thead>
                <tr>
                  <th>Ccy</th>
                  {r.ladder.buckets.map((b) => (
                    <th key={b} className="num">
                      {b}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {r.ladder.rows.map((row) => (
                  <tr key={row.ccy}>
                    <td className="t-medium">{row.ccy}</td>
                    {row.bhd.map((v, i) => (
                      <td key={i} className={`num ${v < 0 ? 't-bad' : v > 0 ? '' : 't-muted'}`}>
                        {v === 0 ? '·' : signed(v)}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>

      <Section title="P&L" meta="daily change in the book's value over 30 days, and who and what made it" />
      <div className="panel-row two">
        <div className="panel">
          <div className="t-heading">Daily P&L</div>
          {history.data ? (
            <Bars data={history.data.points.map((p) => [fmtDate(p.date).slice(4, 6), p.pnl_day])} color="var(--s1)" fmt={signed} labels="extremes" height={140} />
          ) : (
            <SkeletonLines n={4} />
          )}
          <div className="t-heading" style={{ marginTop: 12 }}>
            Cumulative
          </div>
          {history.data && <LineChart series={[{ name: 'P&L', points: history.data.points.map((p, i) => [i, p.pnl_total]) }]} height={110} yFmt={signed} xFmt={(x) => fmtDate(history.data!.points[Math.round(x)]?.date ?? r.as_of).slice(4)} refY={0} />}
        </div>
        <div className="panel">
          <ByTable title="By dealer" rows={r.by_trader} testId="pnl-dealer" />
          <div style={{ height: 14 }} />
          <ByTable title="By pair" rows={r.by_pair} />
        </div>
      </div>

      <Section title="Counterparties" meta="pre-settlement: what is owed on unsettled deals plus a potential-exposure add-on; settlement: the most paid away on one value date" />
      <div className="panel">
        {r.counterparties.length === 0 ? (
          <div className="t-muted">no unsettled deals</div>
        ) : (
          <table className="table" data-testid="counterparties">
            <thead>
              <tr>
                <th>Counterparty</th>
                <th className="num">Deals</th>
                <th className="num">MTM</th>
                <th className="num">Pre-settlement</th>
                <th>of line</th>
                <th className="num">Settlement peak</th>
                <th>of line</th>
                <th>Standing</th>
              </tr>
            </thead>
            <tbody>
              {r.counterparties.map((c) => (
                <tr key={c.name}>
                  <td className="t-medium">{c.name}</td>
                  <td className="num">{c.deals}</td>
                  <td className={`num ${c.mtm < 0 ? 't-bad' : ''}`}>{signed(c.mtm)}</td>
                  <td className="num">{fmtMoneyShort(c.pre_settlement)}</td>
                  <td>
                    <span className="row" style={{ gap: 6 }}>
                      <Gauge value={c.pre_settlement} limit={c.pre_settlement_line} trigger={null} max={Math.max(c.pre_settlement, c.pre_settlement_line) * 1.1} floor={false} width={70} />
                      <span className="t-small">{c.pre_settlement_pct.toFixed(0)}% of {fmtMoneyShort(c.pre_settlement_line)}</span>
                    </span>
                  </td>
                  <td className="num" title={c.settlement_peak_date ? `on ${fmtDateLong(c.settlement_peak_date)}` : undefined}>
                    {fmtMoneyShort(c.settlement_peak)}
                  </td>
                  <td>
                    <span className="row" style={{ gap: 6 }}>
                      <Gauge value={c.settlement_peak} limit={c.settlement_line} trigger={null} max={Math.max(c.settlement_peak, c.settlement_line) * 1.1} floor={false} width={70} />
                      <span className="t-small">{c.settlement_pct.toFixed(0)}% of {fmtMoneyShort(c.settlement_line)}</span>
                    </span>
                  </td>
                  <td>
                    <Badge tone={statusTone(c.status)} dot>
                      {STATUS_WORD[c.status]}
                    </Badge>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      <Section title="Value at risk" meta={`one day, 99%, historical over the last ${r.var.observations} daily moves of each currency against the dinar`} />
      <div className="panel-row two">
        <div className="panel">
          <div className="t-heading">VaR over 30 days</div>
          {history.data ? (
            <LineChart series={[{ name: 'VaR', points: history.data.points.map((p, i) => [i, p.var99]) }]} height={140} yFmt={fmtMoneyShort} xFmt={(x) => fmtDate(history.data!.points[Math.round(x)]?.date ?? r.as_of).slice(4)} refY={varUse?.limit} />
          ) : (
            <SkeletonLines n={4} />
          )}
          <div className="t-small" style={{ marginTop: 4 }}>
            The dashed line is the limit of {fmtMoneyShort(varUse?.limit ?? 0)}.
          </div>
        </div>
        <div className="panel">
          <div className="kv">
            <span className="k">Value at risk</span>
            <span className="v t-medium">{fmtMoneyShort(r.var.var99)}</span>
            <span className="k">Expected shortfall (average beyond the VaR)</span>
            <span className="v">{fmtMoneyShort(r.var.es99)}</span>
            <span className="k">Stressed VaR (worst 250-day window held)</span>
            <span className="v">
              {fmtMoneyShort(r.var.stressed_var99)}
              {r.var.stressed_window_end && <span className="t-small"> to {fmtDate(r.var.stressed_window_end)}</span>}
            </span>
          </div>
          <table className="table" style={{ marginTop: 10 }}>
            <thead>
              <tr>
                <th>Ccy</th>
                <th className="num">Position, BHD</th>
                <th className="num">VaR on its own</th>
              </tr>
            </thead>
            <tbody>
              {r.var.by_ccy.map((c) => (
                <tr key={c.ccy}>
                  <td className="t-medium">{c.ccy}</td>
                  <td className={`num ${c.position < 0 ? 't-bad' : ''}`}>{signed(c.position)}</td>
                  <td className="num">{c.var99 > 0 ? fmtMoneyShort(c.var99) : <span className="t-muted">pegged</span>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      <Section title="Off-market deals" meta="dealt further from the mid for their value date on the trade date than the tolerance allows" />
      <div className="panel">
        {r.off_market.length === 0 ? (
          <Empty tone="good" title="Every deal was done near the market" detail="Each FX deal is checked against the mid for its value date on the day it was traded." />
        ) : (
          <table className="table" data-testid="off-market">
            <thead>
              <tr>
                <th>Deal</th>
                <th>Traded</th>
                <th>Dealer</th>
                <th>Pair</th>
                <th className="num">Rate</th>
                <th className="num">Mid then</th>
                <th className="num">Off by</th>
                <th className="num">Day-one value</th>
              </tr>
            </thead>
            <tbody>
              {r.off_market.map((d) => (
                <tr key={d.deal_id} className="selectable" onClick={() => navigate(`/blotter?deal=${d.deal_id}`)}>
                  <td className="t-medium">{d.deal_ref}</td>
                  <td>{fmtDate(d.trade_date)}</td>
                  <td>{d.trader}</td>
                  <td>
                    {d.side.toLowerCase()} {d.pair}
                  </td>
                  <td className="num">{d.rate.toFixed(5)}</td>
                  <td className="num">{d.trade_mid?.toFixed(5) ?? '—'}</td>
                  <td className="num t-bad">{d.off_market_bp !== null ? `${d.off_market_bp > 0 ? '+' : ''}${d.off_market_bp.toFixed(0)} bp` : '—'}</td>
                  <td className={`num ${(d.day_one ?? 0) < 0 ? 't-bad' : ''}`}>{d.day_one !== null ? signed(d.day_one) : '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      <Breaches />
      <div style={{ height: 16 }} />
    </div>
  );
}

function LimitsSection({ r }: { r: FxRiskResult }) {
  const [all, setAll] = useState(false);
  const order: Record<LimitStatus, number> = { BREACH: 0, WATCH: 1, OK: 2 };
  const sorted = [...r.limits].sort((a, b) => order[a.status] - order[b.status] || b.pct - a.pct);
  const flagged = sorted.filter((l) => l.status !== 'OK');
  const shown = all ? sorted : flagged;
  return (
    <>
      <Section title="Limits" meta={flagged.length ? `${flagged.length} outside or near their level` : 'every limit within its level'} />
      <div className="panel" data-testid="risk-limits">
        {shown.length === 0 && <div className="t-muted">Every one of the {r.limits.length} limits is within its level.</div>}
        {shown.length > 0 && (
          <table className="table limits">
            <tbody>
              {shown.map((l) => (
                <tr key={l.key}>
                  <td style={{ width: 18 }}>
                    <Dot tone={statusTone(l.status)} />
                  </td>
                  <td>{l.label}</td>
                  <td className="num t-medium">{fmtMoneyShort(l.value)}</td>
                  <td className="t-small">of {fmtMoneyShort(l.limit)}</td>
                  <td>
                    <Gauge value={l.value} limit={l.limit} trigger={null} max={Math.max(l.value, l.limit) * 1.1} floor={false} width={110} />
                  </td>
                  <td className="t-small">
                    {l.pct.toFixed(0)}% · {STATUS_WORD[l.status]}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <button type="button" className="btn btn-ghost btn-sm" onClick={() => setAll((a) => !a)} style={{ marginTop: 4 }}>
          {all ? 'Show only those outside or near' : `Show all ${r.limits.length} limits`}
        </button>
      </div>
    </>
  );
}

function PositionsTable({ r }: { r: FxRiskResult }) {
  const cols: Col<FxRiskResult['positions'][number]>[] = [
    { id: 'ccy', header: 'Ccy', width: 60, cell: (p) => p.ccy },
    { id: 'amount', header: 'Amount', width: 120, cell: (p) => String(p.amount) },
    { id: 'bhd', header: 'BHD', width: 120, cell: (p) => String(p.bhd ?? '') },
    { id: 'limit', header: 'Limit', width: 120, cell: (p) => String(p.limit) },
  ];
  return (
    <>
      <div className="row" style={{ marginBottom: 6 }}>
        <span className="t-heading">By currency</span>
        <span className="grow" />
        <button type="button" className="btn btn-ghost btn-sm" onClick={() => exportCsv('fx-positions', cols, r.positions)}>
          Export CSV
        </button>
      </div>
      <table className="table" data-testid="positions">
        <thead>
          <tr>
            <th>Ccy</th>
            <th className="num">Amount</th>
            <th className="num">BHD</th>
            <th className="num">Mid</th>
            <th>Against its limit</th>
          </tr>
        </thead>
        <tbody>
          {r.positions.map((p) => (
            <tr key={p.ccy}>
              <td>
                <span className="t-medium">{p.ccy}</span> {p.pegged && <span className="t-small">pegged</span>}
              </td>
              <td className={`num ${p.amount < 0 ? 't-bad' : ''}`}>{fmtAmount(p.amount.toFixed(0))}</td>
              <td className={`num ${(p.bhd ?? 0) < 0 ? 't-bad' : ''}`}>{p.bhd !== null ? signed(p.bhd) : '—'}</td>
              <td className="num">{p.spot?.toFixed(5) ?? '—'}</td>
              <td>
                <span className="row" style={{ gap: 6 }}>
                  <Gauge value={Math.abs(p.bhd ?? 0)} limit={p.limit} trigger={null} max={Math.max(Math.abs(p.bhd ?? 0), p.limit) * 1.1} floor={false} width={80} />
                  <span className="t-small">
                    {p.pct.toFixed(0)}% of {fmtMoneyShort(p.limit)} · {STATUS_WORD[p.status]}
                  </span>
                </span>
              </td>
            </tr>
          ))}
          {r.positions.length === 0 && (
            <tr>
              <td colSpan={5} className="t-muted">
                no open positions
              </td>
            </tr>
          )}
        </tbody>
        <tfoot>
          <tr>
            <td className="t-medium">Total</td>
            <td />
            <td className="num t-medium">{fmtMoneyShort(r.nop_total)}</td>
            <td colSpan={2} className="t-small">
              longs {fmtMoneyShort(r.nop_long)} · shorts {fmtMoneyShort(r.nop_short)}; the larger is the open position
            </td>
          </tr>
        </tfoot>
      </table>
    </>
  );
}

function ByTable({ title, rows, testId }: { title: string; rows: FxRiskResult['by_trader']; testId?: string }) {
  return (
    <>
      <div className="t-heading" style={{ marginBottom: 6 }}>
        {title}
      </div>
      <table className="table" data-testid={testId}>
        <thead>
          <tr>
            <th />
            <th className="num">Today</th>
            <th className="num">Month</th>
            <th className="num">Year</th>
            <th className="num">Open position</th>
            <th className="num">Deals</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((t) => (
            <tr key={t.key}>
              <td className="t-medium">{t.key}</td>
              <td className={`num ${t.today < 0 ? 't-bad' : ''}`}>{signed(t.today)}</td>
              <td className={`num ${t.mtd < 0 ? 't-bad' : ''}`}>{signed(t.mtd)}</td>
              <td className={`num ${t.ytd < 0 ? 't-bad' : ''}`}>{signed(t.ytd)}</td>
              <td className="num">{fmtMoneyShort(t.nop)}</td>
              <td className="num">{t.deals}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}

interface Run {
  key: string;
  label: string;
  from: string;
  to: string;
  days: string[];
  unsigned: string[];
  worst: number;
  limit: number;
  notes: string[];
  signedBy: string | null;
}

/** Consecutive days the same limit stood breached become one run: one line, one sign-off. */
function runs(rows: Breach[]): Run[] {
  const byKey = new Map<string, Breach[]>();
  for (const b of rows) byKey.set(b.key, [...(byKey.get(b.key) ?? []), b]);
  const out: Run[] = [];
  for (const [key, list] of byKey) {
    const sorted = [...list].sort((a, b) => a.date.localeCompare(b.date));
    let cur: Breach[] = [];
    const flush = () => {
      if (!cur.length) return;
      out.push({
        key,
        label: cur[0].label,
        from: cur[0].date,
        to: cur[cur.length - 1].date,
        days: cur.map((b) => b.date),
        unsigned: cur.filter((b) => !b.signed_by).map((b) => b.date),
        worst: Math.max(...cur.map((b) => b.value)),
        limit: cur[cur.length - 1].limit,
        notes: [...new Set(cur.map((b) => b.note).filter((n): n is string => !!n))],
        signedBy: cur.find((b) => b.signed_by)?.signed_by ?? null,
      });
      cur = [];
    };
    for (const b of sorted) {
      const prev = cur[cur.length - 1];
      if (prev && Date.parse(b.date) - Date.parse(prev.date) > 86_400_000) flush();
      cur.push(b);
    }
    flush();
  }
  return out.sort((a, b) => b.to.localeCompare(a.to) || b.days.length - a.days.length);
}

const SHOW_RUNS = 8;

/** Runs of days in the last month a limit stood breached at the close; the head signs each off. */
function Breaches() {
  const { isHead } = useSession();
  const breaches = useBreaches(30);
  const qc = useQueryClient();
  const toast = useToast();
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [all, setAll] = useState(false);
  const sign = useMutation({
    mutationFn: async ({ run, note }: { run: Run; note: string }) => {
      for (const d of run.unsigned) await endpoints.signBreach(d, run.key, note);
    },
    onSuccess: (_, v) => {
      void qc.invalidateQueries({ queryKey: ['risk', 'breaches'] });
      toast({ tone: 'good', title: 'Breach signed off', detail: `${v.run.label}, ${v.run.unsigned.length} ${v.run.unsigned.length === 1 ? 'day' : 'days'}` });
    },
    onError: (e) => toast({ tone: 'bad', title: 'The sign-off failed', detail: e instanceof ApiError ? e.message : undefined }),
  });
  const list = runs(breaches.data ?? []);
  const open = list.filter((r) => r.unsigned.length > 0).length;
  const shown = all ? list : list.slice(0, SHOW_RUNS);
  const span = (r: Run) => (r.from === r.to ? fmtDate(r.from) : `${fmtDate(r.from)} to ${fmtDate(r.to)}`);
  return (
    <>
      <Section title="Breaches" meta={`limits that stood outside their level at the close in the last 30 days, a run of days as one line${list.length ? `; ${open} not yet signed off` : ''}`} />
      <div className="panel" data-testid="breaches-log">
        {breaches.isLoading && <SkeletonLines n={3} />}
        {breaches.data && list.length === 0 && <Empty tone="good" title="No breaches in the last 30 days" />}
        {list.length > 0 && (
          <table className="table">
            <thead>
              <tr>
                <th>Limit</th>
                <th>When</th>
                <th className="num">Days</th>
                <th className="num">Worst</th>
                <th className="num">Limit</th>
                <th>Sign-off</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((r) => {
                const id = `${r.key}|${r.from}`;
                return (
                  <tr key={id}>
                    <td className="t-medium">{r.label}</td>
                    <td>{span(r)}</td>
                    <td className="num">{r.days.length}</td>
                    <td className="num t-bad">{fmtMoneyShort(r.worst)}</td>
                    <td className="num">{fmtMoneyShort(r.limit)}</td>
                    <td style={{ whiteSpace: 'normal' }}>
                      {r.unsigned.length === 0 ? (
                        <span>
                          <Badge tone="good" dot>
                            signed
                          </Badge>{' '}
                          <span className="t-small">
                            {r.signedBy}: {r.notes.join('; ')}
                          </span>
                        </span>
                      ) : isHead ? (
                        <span className="row" style={{ gap: 6 }}>
                          <input className="input input-sm" style={{ width: 240 }} placeholder="why it happened, what was done" value={notes[id] ?? ''} onChange={(e) => setNotes({ ...notes, [id]: e.target.value })} aria-label={`Sign-off note for ${r.label}`} />
                          <button type="button" className="btn btn-sm" disabled={!(notes[id] ?? '').trim() || sign.isPending} onClick={() => sign.mutate({ run: r, note: (notes[id] ?? '').trim() })}>
                            Sign off {r.unsigned.length > 1 ? `${r.unsigned.length} days` : ''}
                          </button>
                        </span>
                      ) : (
                        <span className="t-small">awaiting the treasury head</span>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
        {list.length > SHOW_RUNS && (
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => setAll((a) => !a)} style={{ marginTop: 4 }}>
            {all ? 'Show the latest' : `Show all ${list.length}`}
          </button>
        )}
      </div>
    </>
  );
}
