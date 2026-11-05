// Positions: one tree of what the desk holds, book to bond to purchase lot, seen through three
// column sets (holding, income for the period, risk). It replaces the Power BI position register,
// income by position, the hedge pairs and the concentration list. Choosing a bond opens its lots,
// its swaps, the repos it is pledged to and its cash flows.
import { useMemo, useState, type ReactNode } from 'react';
import { useSearchParams } from 'react-router';
import { usePmuBond, usePmuPositions } from '../../api/queries';
import type { PositionRow } from '../../api/types';
import { fmtDateLong, fmtDateShort } from '../../lib/fmt';
import { Badge, Chip, Section, StatStrip } from '../../components/ui';
import { SkeletonLines } from '../../components/feedback';
import { exportCsv, type Col } from '../../components/table';
import { Loading, PeriodPicker, bhd, dv01, pct, share, usePeriod } from './common';

type View = 'holding' | 'income' | 'risk';

interface ColDef {
  id: string;
  header: string;
  num?: boolean;
  cell: (r: PositionRow) => ReactNode;
  csv: (r: PositionRow) => string;
  title?: string;
}

const n2 = (v: number | null | undefined, dp = 2) => (v === null || v === undefined ? '' : v.toFixed(dp));

const COLS: Record<View, ColDef[]> = {
  holding: [
    { id: 'kondor', header: 'Kondor ID', cell: (r) => (r.level === 'LOT' ? (r.kondor_id ?? <Badge tone="warn">missing</Badge>) : r.level === 'BOND' ? `${r.lots} ${r.lots === 1 ? 'lot' : 'lots'}` : ''), csv: (r) => r.kondor_id ?? '' },
    { id: 'maturity', header: 'Maturity', cell: (r) => (r.maturity && r.level !== 'BOOK' ? fmtDateShort(r.maturity) : ''), csv: (r) => r.maturity ?? '' },
    { id: 'trade', header: 'Trade date', cell: (r) => (r.trade_date && r.level === 'LOT' ? fmtDateShort(r.trade_date) : ''), csv: (r) => r.trade_date ?? '' },
    { id: 'face_local', header: 'Face, local', num: true, cell: (r) => (r.face_local !== null && r.level !== 'BOOK' ? `${bhd(r.face_local)} ${r.ccy ?? ''}` : ''), csv: (r) => n2(r.face_local, 0) },
    { id: 'face', header: 'Face BHD', num: true, cell: (r) => bhd(r.face_bhd), csv: (r) => n2(r.face_bhd, 0) },
    { id: 'share', header: 'Share', num: true, cell: (r) => share(r.share), csv: (r) => n2(r.share * 100) },
    { id: 'price', header: 'Avg price', num: true, cell: (r) => r.price.toFixed(3), csv: (r) => n2(r.price, 4), title: 'purchase price per 100, weighted by face' },
    { id: 'mkt', header: 'Market', num: true, cell: (r) => (r.market_price !== null ? r.market_price.toFixed(3) : '—'), csv: (r) => n2(r.market_price, 4) },
    { id: 'mtm', header: 'Net MTM', num: true, cell: (r) => <span className={r.mtm_bhd < 0 ? 't-bad' : ''}>{bhd(r.mtm_bhd)}</span>, csv: (r) => n2(r.mtm_bhd, 0), title: 'market value against amortised cost, clean' },
    { id: 'eir', header: 'EIR', num: true, cell: (r) => pct(r.eir, 3), csv: (r) => n2(r.eir, 4), title: 'effective interest rate at purchase' },
    { id: 'ftp', header: 'FTP', num: true, cell: (r) => (r.ftp_rate === null ? <Badge tone="bad">no curve</Badge> : pct(r.ftp_rate, 3)), csv: (r) => n2(r.ftp_rate, 4), title: 'FTP rate locked at purchase' },
  ],
  income: [
    { id: 'coupon', header: 'Coupon', num: true, cell: (r) => bhd(r.income.coupon), csv: (r) => n2(r.income.coupon) },
    { id: 'amort', header: 'Amortisation', num: true, cell: (r) => bhd(r.income.amortisation), csv: (r) => n2(r.income.amortisation) },
    { id: 'ftp', header: 'FTP', num: true, cell: (r) => bhd(-r.income.ftp), csv: (r) => n2(-r.income.ftp) },
    { id: 'carry', header: 'Bond carry', num: true, cell: (r) => <span className={`t-medium ${r.income.carry < 0 ? 't-bad' : ''}`}>{bhd(r.income.carry)}</span>, csv: (r) => n2(r.income.carry), title: 'coupon + amortisation - FTP' },
    { id: 'irs', header: 'IRS', num: true, cell: (r) => bhd(r.income.irs), csv: (r) => n2(r.income.irs) },
    { id: 'nii', header: 'Net NII', num: true, cell: (r) => bhd(r.income.net_nii), csv: (r) => n2(r.income.net_nii) },
    { id: 'daily', header: 'Today after FTP', num: true, cell: (r) => bhd(r.daily_carry_bhd), csv: (r) => n2(r.daily_carry_bhd) },
  ],
  risk: [
    { id: 'face', header: 'Face BHD', num: true, cell: (r) => bhd(r.face_bhd), csv: (r) => n2(r.face_bhd, 0) },
    { id: 'hedged', header: 'Hedged', num: true, cell: (r) => (r.hedged_face_bhd > 0 ? share(r.hedged_face_bhd / Math.max(r.face_bhd, 1)) : '—'), csv: (r) => n2(r.hedged_face_bhd, 0) },
    { id: 'bdv01', header: 'Bond DV01', num: true, cell: (r) => dv01(r.dv01_bhd), csv: (r) => n2(r.dv01_bhd) },
    { id: 'sdv01', header: 'Swap DV01', num: true, cell: (r) => (r.swap_dv01_bhd ? dv01(r.swap_dv01_bhd) : '—'), csv: (r) => n2(r.swap_dv01_bhd) },
    { id: 'ndv01', header: 'Net DV01', num: true, cell: (r) => <span className="t-medium">{dv01(r.dv01_bhd + r.swap_dv01_bhd)}</span>, csv: (r) => n2(r.dv01_bhd + r.swap_dv01_bhd) },
    { id: 'md', header: 'Mod. duration', num: true, cell: (r) => r.mod_duration.toFixed(2), csv: (r) => n2(r.mod_duration) },
    { id: 'ytm', header: 'Market yield', num: true, cell: (r) => pct(r.ytm, 3), csv: (r) => n2(r.ytm, 4) },
    { id: 'mv', header: 'Market value', num: true, cell: (r) => bhd(r.market_value_bhd), csv: (r) => n2(r.market_value_bhd, 0) },
    { id: 'pledged', header: 'Pledged', num: true, cell: (r) => (r.pledged_face_bhd > 0 ? bhd(r.pledged_face_bhd) : '—'), csv: (r) => n2(r.pledged_face_bhd, 0) },
  ],
};

const VIEWS: [View, string][] = [
  ['holding', 'Holding'],
  ['income', 'Income'],
  ['risk', 'Risk'],
];

export function PositionsPage() {
  const [params, setParams] = useSearchParams();
  const first = usePmuPositions();
  const today = first.data?.as_of;
  const period = usePeriod(today);
  const q = usePmuPositions(period.from, period.to);
  const view = (params.get('view') as View) || 'holding';
  const book = params.get('book');
  const bondSel = params.get('bond');
  const [ccy, setCcy] = useState<string>('');
  const [unhedged, setUnhedged] = useState(false);
  const [search, setSearch] = useState('');
  const [closedBooks, setClosedBooks] = useState<Set<string>>(new Set());
  const [openBonds, setOpenBonds] = useState<Set<string>>(new Set());
  const setParam = (k: string, v: string | null) => {
    const next = new URLSearchParams(params);
    if (v) next.set(k, v);
    else next.delete(k);
    setParams(next, { replace: true });
  };

  const data = q.data ?? first.data;
  const rows = data?.rows ?? [];
  const books = useMemo(() => rows.filter((r) => r.level === 'BOOK').map((r) => r.book), [rows]);
  const ccys = useMemo(() => [...new Set(rows.filter((r) => r.level === 'BOND').map((r) => r.ccy ?? ''))].filter(Boolean).sort(), [rows]);
  const needle = search.trim().toLowerCase();

  // Which bonds survive the filters; their books stay as headers and their lots stay attached.
  const visible = useMemo(() => {
    const bondOk = (r: PositionRow) =>
      (!ccy || r.ccy === ccy) &&
      (!unhedged || r.hedged_face_bhd < 0.5) &&
      (!needle || r.label.toLowerCase().includes(needle) || (r.isin ?? '').toLowerCase().includes(needle) || (r.issuer ?? '').toLowerCase().includes(needle)) &&
      (!bondSel || r.isin === bondSel);
    const keepBonds = new Set(rows.filter((r) => r.level === 'BOND' && (!book || r.book === book) && bondOk(r)).map((r) => r.key));
    const keepBooks = new Set(rows.filter((r) => r.level === 'BOND' && keepBonds.has(r.key)).map((r) => r.book));
    return rows.filter((r) => {
      if (r.level === 'BOOK') return keepBooks.has(r.book);
      if (closedBooks.has(r.book)) return false;
      if (r.level === 'BOND') return keepBonds.has(r.key);
      return keepBonds.has(r.parent ?? '') && openBonds.has(r.parent ?? '');
    });
  }, [rows, ccy, unhedged, needle, bondSel, book, closedBooks, openBonds]);

  const flip = (set: (f: (s: Set<string>) => Set<string>) => void, key: string) =>
    set((s) => {
      const n = new Set(s);
      if (n.has(key)) n.delete(key);
      else n.add(key);
      return n;
    });
  const cols = COLS[view];
  const csvCols: Col<PositionRow>[] = [
    { id: 'level', header: 'Level', width: 0, cell: (r) => r.level },
    { id: 'book', header: 'Book', width: 0, cell: (r) => r.book },
    { id: 'isin', header: 'ISIN', width: 0, cell: (r) => r.isin ?? '' },
    { id: 'name', header: 'Name', width: 0, cell: (r) => r.label },
    ...cols.map((c) => ({ id: c.id, header: c.header, width: 0, cell: () => '', csv: c.csv })),
  ];

  return (
    <div className="page" data-ready={data ? '1' : undefined} data-testid="pmu-positions">
      <Loading q={q.data ? q : first}>
        {(d) => (
          <div>
            <div style={{ minWidth: 0 }}>
              <Section title="Positions" meta={`held on ${fmtDateLong(d.as_of)} · income from ${fmtDateShort(d.from)} to ${fmtDateShort(d.to)}`} />
              <StatStrip
                testId="pmu-register-strip"
                cells={[
                  { label: 'Held face', value: bhd(d.headline.portfolio_face_bhd), sub: 'BHD' },
                  { label: 'Market value', value: bhd(d.headline.market_value_bhd), sub: `book value ${bhd(d.headline.book_value_bhd)}` },
                  { label: 'Net MTM', value: bhd(d.headline.net_mtm_bhd), sub: 'bonds and swaps', tone: d.headline.net_mtm_bhd < 0 ? 'warn' : 'good' },
                  { label: 'Purchase lots', value: String(d.headline.lots), sub: `${d.swaps.length} swaps hedging them` },
                  { label: 'With a Kondor ID', value: pct(d.headline.kondor_pct, 1), sub: 'of purchase lots', tone: d.headline.kondor_pct < 100 ? 'warn' : 'good' },
                  { label: 'Daily accrual after FTP', value: bhd(d.headline.daily_carry_bhd), sub: 'today' },
                ]}
              />
              <div className="row toolbar" style={{ gap: 6, flexWrap: 'wrap', marginTop: 10 }}>
                {VIEWS.map(([v, label]) => (
                  <Chip key={v} selected={view === v} onClick={() => setParam('view', v === 'holding' ? null : v)}>
                    {label}
                  </Chip>
                ))}
                <span className="vsep" />
                <Chip small selected={!book} onClick={() => setParam('book', null)}>
                  All books
                </Chip>
                {books.map((b) => (
                  <Chip key={b} small selected={book === b} onClick={() => setParam('book', book === b ? null : b)}>
                    {b}
                  </Chip>
                ))}
                <span className="vsep" />
                <select className="input input-sm" value={ccy} onChange={(e) => setCcy(e.target.value)} aria-label="Currency">
                  <option value="">every currency</option>
                  {ccys.map((c) => (
                    <option key={c} value={c}>
                      {c}
                    </option>
                  ))}
                </select>
                <Chip small selected={unhedged} onClick={() => setUnhedged((u) => !u)}>
                  Unhedged only
                </Chip>
                <input className="input input-sm" style={{ width: 180 }} placeholder="bond, issuer or ISIN" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Search positions" />
                {bondSel && (
                  <Chip small selected onClick={() => setParam('bond', null)} title="show every bond again">
                    Only {bondSel} ✕
                  </Chip>
                )}
                <span className="grow" />
                <button
                  type="button"
                  className="btn btn-ghost btn-sm"
                  onClick={() => {
                    setClosedBooks(new Set());
                    setOpenBonds(new Set(rows.filter((r) => r.level === 'BOND').map((r) => r.key)));
                  }}
                >
                  Show every lot
                </button>
                <button
                  type="button"
                  className="btn btn-ghost btn-sm"
                  onClick={() => {
                    setClosedBooks(new Set());
                    setOpenBonds(new Set());
                  }}
                >
                  Bonds only
                </button>
                <button type="button" className="btn btn-sm" onClick={() => exportCsv(`pmu-positions-${view}`, csvCols, visible)}>
                  Export CSV
                </button>
              </div>
              {view === 'income' && (
                <div style={{ marginTop: 8 }}>
                  <PeriodPicker today={d.as_of} period={period} />
                </div>
              )}
              <div className="tree-wrap">
                <table className="table tree" data-testid="pmu-tree">
                  <thead>
                    <tr>
                      <th>Book, bond, lot</th>
                      {cols.map((c) => (
                        <th key={c.id} className={c.num ? 'num' : ''} title={c.title}>
                          {c.header}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {visible.map((r) => {
                      const expandable = r.level !== 'LOT';
                      const isOpen = r.level === 'BOOK' ? !closedBooks.has(r.book) : openBonds.has(r.key);
                      const toggle = () => (r.level === 'BOOK' ? flip(setClosedBooks, r.book) : flip(setOpenBonds, r.key));
                      return (
                        <tr key={r.key} className={`lvl-${r.level.toLowerCase()} ${r.isin === bondSel && r.level === 'BOND' ? 'selected' : ''}`} data-level={r.level}>
                          <td className="name">
                            {expandable ? (
                              <button type="button" className="twisty" aria-expanded={isOpen} aria-label={`${isOpen ? 'collapse' : 'expand'} ${r.label}`} onClick={toggle}>
                                {isOpen ? '▾' : '▸'}
                              </button>
                            ) : (
                              <span className="twisty" />
                            )}
                            {r.level === 'BOND' ? (
                              <button type="button" className="link" onClick={() => setParam('bond', bondSel === r.isin ? null : r.isin)} title={`${r.isin}: open the bond`}>
                                {r.label}
                              </button>
                            ) : (
                              <span>{r.label}</span>
                            )}
                            {r.level === 'BOND' && <span className="t-small"> {r.isin}</span>}
                          </td>
                          {cols.map((c) => (
                            <td key={c.id} className={c.num ? 'num' : ''}>
                              {c.cell(r)}
                            </td>
                          ))}
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
                {visible.length === 0 && <div className="t-muted" style={{ padding: 12 }}>No position matches these filters.</div>}
              </div>
            </div>
            {bondSel && <BondDetail isin={bondSel} onClose={() => setParam('bond', null)} />}
          </div>
        )}
      </Loading>
    </div>
  );
}

function BondDetail({ isin, onClose }: { isin: string; onClose: () => void }) {
  const q = usePmuBond(isin);
  const d = q.data;
  return (
    <aside className="detail-pane" data-testid="pmu-bond">
      <div className="row" style={{ gap: 6 }}>
        <span className="t-heading">{d?.bond.name ?? isin}</span>
        <span className="grow" />
        <button type="button" className="btn btn-ghost btn-sm" onClick={onClose} aria-label="close the bond">
          ✕
        </button>
      </div>
      {!d ? (
        q.isError ? <div className="notice bad">{q.error?.message}</div> : <SkeletonLines n={6} />
      ) : (
        <>
          <div className="kv">
            <span>ISIN</span>
            <span>{d.bond.isin}</span>
            <span>Issuer</span>
            <span>
              {d.bond.issuer} <span className="t-small">{d.bond.issuer_type.toLowerCase().replace('_', ' ')}</span>
            </span>
            <span>Coupon</span>
            <span>
              {d.bond.rate_type === 'FLOAT' ? `${d.bond.float_index} + ${pct(d.bond.float_spread)}` : pct(d.bond.coupon, 3)}, {d.bond.coupon_freq}× a year, {d.bond.day_count}
            </span>
            <span>Issued, matures</span>
            <span>
              {fmtDateShort(d.bond.issue_date)}, {fmtDateShort(d.bond.maturity)}
            </span>
            <span>Currency, HQLA</span>
            <span>
              {d.bond.ccy}, {d.bond.hqla_level ?? 'not HQLA'}
            </span>
          </div>
          <div className="t-label" style={{ marginTop: 10 }}>
            Purchase lots
          </div>
          <table className="table">
            <tbody>
              {d.lots.map((l) => (
                <tr key={l.key}>
                  <td>{l.trade_date && fmtDateShort(l.trade_date)}</td>
                  <td className="t-small">{l.book}</td>
                  <td className="num">{bhd(l.face_local ?? 0)}</td>
                  <td className="num">{l.price.toFixed(3)}</td>
                  <td className="num">{pct(l.eir, 3)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {d.swaps.length > 0 && (
            <>
              <div className="t-label" style={{ marginTop: 10 }}>
                Swaps hedging it
              </div>
              <table className="table">
                <tbody>
                  {d.swaps.map((s) => (
                    <tr key={s.id}>
                      <td>
                        {s.pay_fixed ? 'pays' : 'receives'} {pct(s.fixed_rate, 3)} on {bhd(s.notional)}
                        <div className="t-small">
                          to {fmtDateShort(s.maturity)}
                          {s.closed_notional > 0 ? ` · ${bhd(s.closed_notional)} closed` : ''}
                        </div>
                      </td>
                      <td className="num">MTM {bhd(s.mtm_bhd)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}
          {d.repos.length > 0 && (
            <>
              <div className="t-label" style={{ marginTop: 10 }}>
                Pledged to
              </div>
              <table className="table">
                <tbody>
                  {d.repos.map((r) => (
                    <tr key={r.id}>
                      <td>{r.counterparty}</td>
                      <td className="num">{bhd(r.cash_bhd)}</td>
                      <td className="t-small">to {fmtDateShort(r.end)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}
          <div className="t-label" style={{ marginTop: 10 }}>
            Cash flows, next three years
          </div>
          <table className="table">
            <tbody>
              {d.cash_flows.slice(0, 14).map((c, i) => (
                <tr key={i}>
                  <td>{fmtDateShort(c.date)}</td>
                  <td className="t-secondary">{c.kind}</td>
                  <td className="num">{bhd(c.amount_bhd)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </aside>
  );
}
