// PMU overview: the desk at a glance. Six figures with their move since yesterday, each opening
// the page that explains it; the portfolio by book and its rate risk; the year's NII, booked and
// forecast; the maturity profile hedged and unhedged; the largest holdings; the repo book.
import { useState } from 'react';
import { useNavigate } from 'react-router';
import { usePmuOverview } from '../../api/queries';
import type { PmuOverview } from '../../api/types';
import { fmtDateLong, fmtMonth } from '../../lib/fmt';
import { Chip, Section, StatStrip, type StatCell } from '../../components/ui';
import { Bars, SERIES, StackedBars } from '../../components/charts';
import { Loading, bhd, dv01, pct, share, since } from './common';

export function PmuOverviewPage() {
  const q = usePmuOverview();
  return (
    <div className="page" data-ready={q.data ? '1' : undefined} data-testid="pmu-overview">
      <Loading q={q}>{(o) => <Body o={o} />}</Loading>
    </div>
  );
}

function Body({ o }: { o: PmuOverview }) {
  const navigate = useNavigate();
  const [yearEnd, setYearEnd] = useState(false);
  const h = o.headline;
  const p = o.headline_prev ?? undefined;
  const cells: StatCell[] = [
    { label: 'Portfolio size', value: bhd(h.portfolio_face_bhd), sub: `face held · market value ${bhd(h.market_value_bhd)}`, onClick: () => navigate('/pmu/positions') },
    { label: 'Net MTM', value: bhd(h.net_mtm_bhd), sub: `bonds and swaps · ${since(h.net_mtm_bhd, p?.net_mtm_bhd)}`, tone: h.net_mtm_bhd < 0 ? 'warn' : 'good', onClick: () => navigate('/pmu/risk') },
    { label: 'Bond carry, month to date', value: bhd(h.mtd_carry_bhd), sub: `after FTP · ${bhd(h.daily_carry_bhd)} today`, tone: h.mtd_carry_bhd < 0 ? 'bad' : 'good', onClick: () => navigate('/pmu/income') },
    { label: 'Net DV01', value: dv01(h.net_dv01_bhd), sub: `BHD per bp, after hedges · ${since(h.net_dv01_bhd, p?.net_dv01_bhd)}`, onClick: () => navigate('/pmu/risk') },
    { label: 'NII, month to date', value: bhd(h.mtd_nii_bhd), sub: 'carry after FTP plus swap carry', tone: h.mtd_nii_bhd < 0 ? 'bad' : 'good', onClick: () => navigate('/pmu/income') },
    { label: 'Portfolio yield', value: pct(h.portfolio_yield), sub: `effective rate on book value · duration ${h.mod_duration.toFixed(2)}`, onClick: () => navigate('/pmu/risk') },
  ];
  const maxFace = Math.max(1, ...o.by_book.map((b) => b.face_bhd));
  const buckets = yearEnd ? o.maturity_year_end : o.maturity_now;
  const months = o.monthly;
  const r = o.repo;
  return (
    <>
      <Section title="Bond portfolio" meta={`as of ${fmtDateLong(o.as_of)} · in BHD`} />
      <StatStrip cells={cells} testId="pmu-headline" hero />
      {o.warnings.length > 0 && (
        <div className="row" style={{ gap: 12, marginTop: 6 }}>
          {o.warnings.map((w) => (
            <span key={w.key} className="t-small t-warn">
              {w.message}
            </span>
          ))}
        </div>
      )}

      <div className="panel-row pmu-ov">
        <div className="panel">
          <div className="t-heading">Portfolio by book</div>
          <table className="table limits fit" data-testid="pmu-books">
            <colgroup>
              <col />
              <col style={{ width: 64 }} />
              <col style={{ width: 64 }} />
              <col style={{ width: 58 }} />
              <col style={{ width: '26%' }} />
            </colgroup>
            <thead>
              <tr>
                <th>Book</th>
                <th className="num">Face</th>
                <th className="num">MTM</th>
                <th className="num">Yield</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {o.by_book.map((b) => (
                <tr key={b.book} className="clickable" onClick={() => navigate(`/pmu/positions?book=${encodeURIComponent(b.book)}`)} title={`${b.name}: open its positions`}>
                  <td className="metric">
                    <span className="t-medium">{b.book}</span> <span className="t-small">{b.accounting}</span>
                  </td>
                  <td className="num">{bhd(b.face_bhd)}</td>
                  <td className={`num ${b.mtm_bhd < 0 ? 't-bad' : ''}`}>{bhd(b.mtm_bhd)}</td>
                  <td className="num">{pct(b.yield_pct)}</td>
                  <td>
                    <span className="mini-bar">
                      <span style={{ width: `${(100 * b.face_bhd) / maxFace}%` }} />
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="panel">
          <div className="t-heading">Net DV01 by book, BHD per bp</div>
          <Bars data={o.by_book.map((b) => [b.book, b.net_dv01_bhd])} color="var(--s2)" fmt={dv01} labels="all" />
        </div>
        <div className="panel">
          <div className="t-heading">NII by month, {months[0]?.month.slice(0, 4)}</div>
          <Bars
            data={months.map((m) => [fmtMonth(m.month), m.income.net_nii])}
            dim={months.map((m) => !m.actual)}
            dimLegend={['booked', 'forecast (runs off)']}
            fmt={bhd}
            labels="extremes"
            color={SERIES[0]}
          />
        </div>
        <div className="panel">
          <div className="row" style={{ gap: 6, marginBottom: 8 }}>
            <span className="t-heading">Maturity profile</span>
            <span className="grow" />
            <Chip small selected={!yearEnd} onClick={() => setYearEnd(false)}>
              Now
            </Chip>
            <Chip small selected={yearEnd} onClick={() => setYearEnd(true)}>
              Year end
            </Chip>
          </div>
          <StackedBars testId="pmu-maturity" data={buckets.map((b) => [b.label, b.hedged_bhd, b.unhedged_bhd])} names={['hedged', 'unhedged']} colors={['var(--s3)', 'var(--s4)']} fmt={bhd} />
        </div>
      </div>

      <Section title="Repo book" meta={`${r.active} active · ${bhd(r.due_7d_bhd)} due within seven days`} />
      <StatStrip
        testId="pmu-repo"
        cells={[
          { label: 'Repo amount', value: bhd(r.amount_bhd), sub: `${r.active} active`, onClick: () => navigate('/pmu/funding') },
          { label: 'Available face', value: bhd(r.available_face_bhd), sub: `${bhd(r.pledged_face_bhd)} pledged`, onClick: () => navigate('/pmu/funding') },
          { label: 'Average repo rate', value: pct(r.avg_rate), sub: 'weighted by cash' },
          { label: 'Average haircut', value: pct(r.avg_haircut, 1), sub: 'weighted by cash' },
          { label: 'Weighted maturity', value: `${Math.round(r.wtd_maturity_days)} days`, sub: 'still to run' },
          { label: 'Receipts, next 12 months', value: bhd(r.receipts_12m_bhd), sub: 'coupons and redemptions', onClick: () => navigate('/pmu/funding') },
        ]}
      />

      <Section title="Largest holdings" meta="by face, across books; share of the whole portfolio" />
      <table className="table" data-testid="pmu-concentration">
        <thead>
          <tr>
            <th>Bond</th>
            <th>Book</th>
            <th>ISIN</th>
            <th className="num">Face</th>
            <th className="num">Share</th>
            <th className="num">Yield</th>
            <th className="num">Hedged</th>
            <th className="num">DV01</th>
          </tr>
        </thead>
        <tbody>
          {o.concentration.map((c) => (
            <tr key={c.key} className="clickable" onClick={() => navigate(`/pmu/positions?bond=${encodeURIComponent(c.isin ?? '')}`)}>
              <td>{c.label}</td>
              <td className="t-secondary">{c.book}</td>
              <td className="t-small">{c.isin}</td>
              <td className="num">{bhd(c.face_bhd)}</td>
              <td className="num">{share(c.share)}</td>
              <td className="num">{pct(c.eir)}</td>
              <td className="num">{c.hedged_face_bhd > 0 ? share(c.hedged_face_bhd / c.face_bhd) : '—'}</td>
              <td className="num">{dv01(c.dv01_bhd)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
