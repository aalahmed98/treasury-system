// Risk and hedges: what the portfolio loses when rates move, how much the swaps take off, where
// in the curve the risk sits, and how the hedged and unhedged face runs off to year end. Bonds are
// revalued at their yield moved; swaps on their curve moved; nothing is estimated from DV01 alone.
import { useState } from 'react';
import { useNavigate } from 'react-router';
import { usePmuRisk } from '../../api/queries';
import type { PmuRisk } from '../../api/types';
import { fmtDateLong } from '../../lib/fmt';
import { Chip, Section, StatStrip } from '../../components/ui';
import { Bars, BarsPair, StackedBars } from '../../components/charts';
import { Loading, bhd, dv01, pct, share } from './common';

export function RiskPage() {
  const q = usePmuRisk();
  return (
    <div className="page" data-ready={q.data ? '1' : undefined} data-testid="pmu-risk">
      <Loading q={q}>{(r) => <Body r={r} />}</Loading>
    </div>
  );
}

function Body({ r }: { r: PmuRisk }) {
  const navigate = useNavigate();
  const [yearEnd, setYearEnd] = useState(false);
  const h = r.headline;
  const bond = r.dv01_buckets.reduce((s, b) => s + b.bond, 0);
  const irs = r.dv01_buckets.reduce((s, b) => s + b.irs, 0);
  const yearsAll = r.yield_by_year.filter((y) => y.book === null);
  const yearsBook = r.yield_by_year.filter((y) => y.book !== null);
  const buckets = yearEnd ? r.maturity_year_end : r.maturity_now;
  return (
    <>
      <Section title="Risk and hedges" meta={`as of ${fmtDateLong(r.as_of)} · DV01 is the P&L of a 1 bp rise, in BHD`} />
      <StatStrip
        testId="pmu-risk-strip"
        hero
        cells={[
          { label: 'Net DV01', value: dv01(h.net_dv01_bhd), sub: 'bonds and swaps' },
          { label: 'Bond DV01', value: dv01(bond), sub: 'before hedges' },
          { label: 'Swap DV01', value: dv01(irs), sub: `takes off ${share(bond ? -irs / bond : 0)} of the bonds’ risk` },
          { label: 'Modified duration', value: h.mod_duration.toFixed(2), sub: 'weighted by market value' },
          { label: 'Weighted bond yield', value: pct(h.portfolio_yield), sub: 'effective rate on book value' },
          { label: 'Year-end hedged face', value: bhd(r.year_end_hedged_face_bhd), sub: `of ${bhd(r.year_end_face_bhd)} still held on 31 Dec` },
        ]}
      />

      <div className="panel-row two">
        <div className="panel">
          <div className="t-heading">DV01 by maturity bucket, BHD per bp</div>
          <BarsPair data={r.dv01_buckets.map((b) => [b.label, b.bond, b.irs])} names={['bonds', 'swaps']} fmt={dv01} />
          <table className="table compact">
            <tbody>
              <tr>
                <td className="t-secondary">Net</td>
                {r.dv01_buckets.map((b) => (
                  <td key={b.label} className="num">
                    {b.net ? dv01(b.net) : '—'}
                  </td>
                ))}
              </tr>
            </tbody>
          </table>
        </div>
        <div className="panel">
          <div className="t-heading">Parallel rate shock, estimated P&amp;L</div>
          <table className="table" data-testid="pmu-shocks">
            <thead>
              <tr>
                <th>Every rate</th>
                {r.books.map((b) => (
                  <th key={b} className="num">
                    {b}
                  </th>
                ))}
                <th className="num">Total</th>
              </tr>
            </thead>
            <tbody>
              {r.shocks.map((s) => (
                <tr key={s.shift_bp}>
                  <td>
                    {s.shift_bp > 0 ? '+' : '−'}
                    {Math.abs(s.shift_bp)} bp
                  </td>
                  {s.by_book.map((v, i) => (
                    <td key={i} className={`num ${v < 0 ? 't-bad' : ''}`}>
                      {bhd(v)}
                    </td>
                  ))}
                  <td className={`num t-medium ${s.total < 0 ? 't-bad' : ''}`}>{bhd(s.total)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="t-small">HTM bonds are shown for their economic value; their accounts do not move with rates.</div>
        </div>
      </div>

      <Section title="Hedge pairs, all books" meta={`${r.hedge_pairs.length} bonds with swaps against them; closed notional in the bond’s currency`} />
      <table className="table" data-testid="pmu-hedge-pairs">
        <thead>
          <tr>
            <th>Bond</th>
            <th>ISIN</th>
            <th className="num">Face</th>
            <th className="num">Hedged</th>
            <th className="num">Bond DV01</th>
            <th className="num">IRS DV01</th>
            <th className="num">Net DV01</th>
            <th className="num">Bond MTM</th>
            <th className="num">IRS MTM</th>
            <th className="num">Net MTM</th>
            <th className="num">Closed notional</th>
          </tr>
        </thead>
        <tbody>
          {r.hedge_pairs.map((p) => (
            <tr key={p.isin} className="clickable" onClick={() => navigate(`/pmu/positions?view=risk&bond=${p.isin}`)}>
              <td>{p.name}</td>
              <td className="t-small">{p.isin}</td>
              <td className="num">{bhd(p.face_bhd)}</td>
              <td className="num">{share(p.hedged_face_bhd / Math.max(1, p.face_bhd))}</td>
              <td className="num">{dv01(p.bond_dv01)}</td>
              <td className="num">{dv01(p.irs_dv01)}</td>
              <td className="num t-medium">{dv01(p.net_dv01)}</td>
              <td className="num">{bhd(p.bond_mtm)}</td>
              <td className="num">{bhd(p.irs_mtm)}</td>
              <td className={`num t-medium ${p.net_mtm < 0 ? 't-bad' : ''}`}>{bhd(p.net_mtm)}</td>
              <td className="num">{p.closed_notional > 0 ? `${bhd(p.closed_notional)} ${p.ccy}` : '—'}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <div className="panel-row two">
        <div className="panel">
          <div className="t-heading">By portfolio: duration, value, risk and today’s accrual</div>
          <table className="table" data-testid="pmu-duration">
            <thead>
              <tr>
                <th>Portfolio</th>
                <th className="num">Mod. duration</th>
                <th className="num">Market value</th>
                <th className="num">Net DV01</th>
                <th className="num" title="coupon and amortisation less FTP, today">Daily accrual after FTP</th>
              </tr>
            </thead>
            <tbody>
              {r.by_book.map((b) => (
                <tr key={b.book}>
                  <td>
                    {b.book} <span className="t-small">{b.accounting}</span>
                  </td>
                  <td className="num">{b.mod_duration.toFixed(2)}</td>
                  <td className="num">{bhd(b.market_value_bhd)}</td>
                  <td className="num">{dv01(b.net_dv01_bhd)}</td>
                  <td className={`num ${b.daily_carry_bhd < 0 ? 't-bad' : ''}`}>{bhd(b.daily_carry_bhd)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="t-heading" style={{ marginTop: 16 }}>
            Hedged and unhedged face by currency
          </div>
          <StackedBars data={r.hedged_by_ccy.map((c) => [c.ccy, c.hedged_bhd, c.unhedged_bhd])} names={['hedged', 'unhedged']} colors={['var(--s3)', 'var(--s4)']} fmt={bhd} testId="pmu-hedged-ccy" />
        </div>
        <div className="panel">
          <div className="row" style={{ gap: 6, marginBottom: 8 }}>
            <span className="t-heading">Maturity buckets, hedged and unhedged</span>
            <span className="grow" />
            <Chip small selected={!yearEnd} onClick={() => setYearEnd(false)}>
              Now
            </Chip>
            <Chip small selected={yearEnd} onClick={() => setYearEnd(true)}>
              Year end
            </Chip>
          </div>
          <StackedBars data={buckets.map((b) => [b.label, b.hedged_bhd, b.unhedged_bhd])} names={['hedged', 'unhedged']} colors={['var(--s3)', 'var(--s4)']} fmt={bhd} />
          <div className="t-small">{yearEnd ? 'What is still held on 31 December, by time left from then, hedged by the swaps still live then.' : 'By time left to maturity from today.'}</div>
        </div>
      </div>

      <Section title="Current weighted bond yield by maturity year" meta="effective rate at purchase, weighted by face; unhedged counts only the face no swap covers" />
      <div className="panel-row two">
        <div className="panel">
          <Bars data={yearsAll.map((y) => [String(y.year), y.yield_all])} fmt={(v) => pct(v)} labels="all" />
        </div>
        <div className="panel">
          <table className="table" data-testid="pmu-yield-years">
            <thead>
              <tr>
                <th>Portfolio</th>
                <th>Maturity year</th>
                <th className="num">Held face</th>
                <th className="num">All-bond yield</th>
                <th className="num">Unhedged yield</th>
              </tr>
            </thead>
            <tbody>
              {yearsBook.map((y) => (
                <tr key={`${y.book}${y.year}`}>
                  <td>{y.book}</td>
                  <td>{y.year}</td>
                  <td className="num">{bhd(y.face_bhd)}</td>
                  <td className="num">{pct(y.yield_all)}</td>
                  <td className="num">{pct(y.yield_unhedged)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      <Section title="Fully unhedged holdings" meta={`${r.unhedged.length} bonds with no swap against them`} />
      <table className="table" data-testid="pmu-unhedged">
        <thead>
          <tr>
            <th>Bond</th>
            <th>Book</th>
            <th className="num">Face</th>
            <th className="num">DV01</th>
            <th className="num">Mod. duration</th>
            <th className="num">Yield</th>
            <th className="num">MTM</th>
          </tr>
        </thead>
        <tbody>
          {[...r.unhedged]
            .sort((a, b) => a.dv01_bhd - b.dv01_bhd)
            .map((u) => (
              <tr key={u.key} className="clickable" onClick={() => navigate(`/pmu/positions?view=risk&bond=${u.isin}`)}>
                <td>{u.label}</td>
                <td className="t-secondary">{u.book}</td>
                <td className="num">{bhd(u.face_bhd)}</td>
                <td className="num">{dv01(u.dv01_bhd)}</td>
                <td className="num">{u.mod_duration.toFixed(2)}</td>
                <td className="num">{pct(u.eir)}</td>
                <td className={`num ${u.mtm_bhd < 0 ? 't-bad' : ''}`}>{bhd(u.mtm_bhd)}</td>
              </tr>
            ))}
        </tbody>
      </table>
    </>
  );
}
