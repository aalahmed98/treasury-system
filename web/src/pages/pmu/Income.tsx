// Income: what the portfolio earned over a period, broken into its parts, and what it will earn.
// The six figures and the waterfall are one sum (coupon + amortisation − FTP = carry, + swap carry
// = net NII), so the chart and the numbers can never disagree. Below, the forecast: the rest of
// the year and the next three, running off or reinvesting what matures.
import { useMemo, useState } from 'react';
import { usePmuForecast, usePmuIncome, usePmuOverview } from '../../api/queries';
import type { PmuForecast, PmuIncome, PmuIncomeParts } from '../../api/types';
import { fmtDateShort, fmtMonth } from '../../lib/fmt';
import { Chip, Section, StatStrip } from '../../components/ui';
import { Bars, SERIES, Waterfall } from '../../components/charts';
import { SkeletonLines } from '../../components/feedback';
import { BookYearTable, INCOME_COLS, Loading, PeriodPicker, bhd, usePeriod } from './common';

export function IncomePage() {
  const first = usePmuIncome();
  const ov = usePmuOverview();
  const today = ov.data?.as_of ?? first.data?.to;
  const period = usePeriod(today);
  const q = usePmuIncome(period.from, period.to);
  const shown = q.data ?? first.data;
  const names = useMemo(() => Object.fromEntries((ov.data?.by_book ?? []).map((b) => [b.book, b.name])), [ov.data]);
  return (
    <div className="page" data-ready={shown ? '1' : undefined} data-testid="pmu-income">
      <Loading q={q.data ? q : first}>
        {(d) => (
          <>
            <Section title="Accrual and income" meta={`${fmtDateShort(d.from)} to ${fmtDateShort(d.to)} · in BHD`} />
            {today && <PeriodPicker today={today} period={period} />}
            <Period d={d} busy={q.isFetching} />
            <Forecast names={names} />
          </>
        )}
      </Loading>
    </div>
  );
}

function Period({ d, busy }: { d: PmuIncome; busy: boolean }) {
  const t = d.total;
  const tone = (v: number) => (v < 0 ? ('bad' as const) : ('good' as const));
  const byBond = [...d.by_bond].sort((a, b) => a.income.carry - b.income.carry);
  const maxAbs = Math.max(1, ...byBond.map((b) => Math.abs(b.income.carry)));
  return (
    <div className={busy ? 'dimmed' : ''}>
      <StatStrip
        testId="pmu-income-strip"
        hero
        cells={[
          { label: 'Bond carry', value: bhd(t.carry), sub: 'after FTP', tone: tone(t.carry) },
          { label: 'Coupon', value: bhd(t.coupon), sub: 'accrued over the period' },
          { label: 'Amortisation', value: bhd(t.amortisation), sub: 'premium and discount at the effective rate' },
          { label: 'FTP funding', value: bhd(-t.ftp), sub: 'book value at the rate locked at purchase' },
          { label: 'IRS carry', value: bhd(t.irs), sub: 'receive leg less pay leg of the hedges' },
          { label: 'Accounting NII', value: bhd(t.accounting_nii), sub: `with ${bhd(-t.repo)} of repo interest, no FTP` },
        ]}
      />
      <div className="panel-row two">
        <div className="panel">
          <div className="t-heading">What makes up bond carry, and net NII</div>
          <Waterfall
            height={210}
            fmt={bhd}
            steps={[
              ['Coupon', t.coupon, false],
              ['Amortisation', t.amortisation, false],
              ['FTP', -t.ftp, false],
              ['Bond carry', t.carry, true],
              ['IRS carry', t.irs, false],
              ['Net NII', t.net_nii, true],
            ]}
          />
        </div>
        <div className="panel">
          <div className="t-heading">Carry after FTP earned each day</div>
          <Bars data={d.daily.map((x) => [fmtDateShort(x.date).slice(0, 6), x.income.carry])} fmt={bhd} colorOf={(v) => (v < 0 ? 'var(--bad)' : SERIES[0])} />
        </div>
      </div>

      <div className="panel-row two">
        <div className="panel">
          <div className="t-heading">Bond carry after FTP, by bond (weakest first)</div>
          <table className="table" data-testid="pmu-carry-by-bond">
            <tbody>
              {byBond.slice(0, 14).map((b) => (
                <tr key={b.key}>
                  <td className="ellipsis" style={{ maxWidth: 260 }} title={b.label}>
                    {b.label}
                  </td>
                  <td className="num" style={{ width: 70 }}>
                    <span className={b.income.carry < 0 ? 't-bad' : ''}>{bhd(b.income.carry)}</span>
                  </td>
                  <td style={{ width: '38%' }}>
                    <span className="div-bar">
                      <span className={b.income.carry < 0 ? 'neg' : 'pos'} style={{ width: `${(50 * Math.abs(b.income.carry)) / maxAbs}%` }} />
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {byBond.length > 14 && <div className="t-small">the weakest 14 of {byBond.length}; every bond is on the Positions page under Income</div>}
        </div>
        <div className="panel">
          <div className="t-heading">By book</div>
          <IncomeTable rows={d.by_book.map((b) => ({ key: b.key, label: b.label, income: b.income }))} total={t} />
        </div>
      </div>
    </div>
  );
}

function IncomeTable({ rows, total }: { rows: { key: string; label: string; income: PmuIncomeParts }[]; total?: PmuIncomeParts }) {
  return (
    <table className="table">
      <thead>
        <tr>
          <th />
          {INCOME_COLS.map(([k, h]) => (
            <th key={k} className="num">
              {h}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.key}>
            <td>{r.label}</td>
            {INCOME_COLS.map(([k]) => (
              <td key={k} className="num">
                {bhd(k === 'ftp' ? -r.income[k] : r.income[k])}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
      {total && (
        <tfoot>
          <tr>
            <td>Total</td>
            {INCOME_COLS.map(([k]) => (
              <td key={k} className="num">
                {bhd(k === 'ftp' ? -total[k] : total[k])}
              </td>
            ))}
          </tr>
        </tfoot>
      )}
    </table>
  );
}

function Forecast({ names }: { names: Record<string, string> }) {
  const [reinvest, setReinvest] = useState(false);
  const q = usePmuForecast(reinvest);
  const f = q.data;
  return (
    <div data-testid="pmu-forecast">
      <div className="row" style={{ gap: 6, marginTop: 22 }}>
        <span className="t-eyebrow">Income forecast</span>
        <span className="t-small">{reinvest ? 'maturing face reinvested at today’s yield for the same tenor' : 'the portfolio runs off: nothing new is bought'}</span>
        <span className="grow" />
        <Chip small selected={!reinvest} onClick={() => setReinvest(false)}>
          Run off
        </Chip>
        <Chip small selected={reinvest} onClick={() => setReinvest(true)}>
          Reinvest
        </Chip>
      </div>
      {!f ? <SkeletonLines n={6} /> : <ForecastBody f={f} names={names} busy={q.isFetching} />}
    </div>
  );
}

function ForecastBody({ f, names, busy }: { f: PmuForecast; names: Record<string, string>; busy: boolean }) {
  const year = f.as_of.slice(0, 4);
  return (
    <div className={busy ? 'dimmed' : ''}>
      <StatStrip
        testId="pmu-forecast-strip"
        cells={[
          { label: `Full-year NII, ${year}`, value: bhd(f.full_year.net_nii), sub: `${bhd(f.year_to_date.net_nii)} booked so far` },
          { label: 'Rest of the year', value: bhd(f.rest_of_year.net_nii), sub: `from ${fmtDateShort(f.periods[0]?.from ?? f.as_of)}` },
          { label: 'Full-year carry after FTP', value: bhd(f.full_year.carry), sub: 'bond carry, no swaps' },
          { label: 'Year-end face', value: bhd(f.year_end_face_bhd), sub: `${bhd(f.year_end_hedged_face_bhd)} of it hedged` },
        ]}
      />
      <div className="panel-row two">
        <div className="panel">
          <div className="t-heading">Monthly NII, booked and forecast</div>
          <Bars
            data={f.monthly.map((m) => [fmtMonth(m.month), m.income.net_nii])}
            dim={f.monthly.map((m) => !m.actual)}
            dimLegend={['booked', 'forecast']}
            fmt={bhd}
           
          />
        </div>
        <div className="panel">
          <div className="t-heading">Forecast net income by period</div>
          <table className="table" data-testid="pmu-periods">
            <thead>
              <tr>
                <th>Period</th>
                <th>Dates</th>
                <th className="num">Carry after FTP</th>
                <th className="num">IRS</th>
                <th className="num">Net NII</th>
              </tr>
            </thead>
            <tbody>
              {f.periods.map((p) => (
                <tr key={p.label}>
                  <td>
                    {p.label} {p.partial && <span className="t-small">part year</span>}
                  </td>
                  <td className="t-small">
                    {fmtDateShort(p.from)} to {fmtDateShort(p.to)}
                  </td>
                  <td className="num">{bhd(p.income.carry)}</td>
                  <td className="num">{bhd(p.income.irs)}</td>
                  <td className="num t-medium">{bhd(p.income.net_nii)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="t-small">Floating coupons and swap legs use the forwards in today’s curves; swaps run to maturity and repos to their end date.</div>
        </div>
      </div>
      <Section title={`Interest earned by month, ${year}`} meta="income at the effective rate, its FTP funding, and what is left with the swaps; forecast months in grey" />
      <table className="table" data-testid="pmu-months">
        <thead>
          <tr>
            <th>Month</th>
            <th className="num">EIR income</th>
            <th className="num">FTP funding</th>
            <th className="num">Bond carry</th>
            <th className="num">IRS carry</th>
            <th className="num">Net NII</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {f.monthly.map((m) => (
            <tr key={m.month} className={m.actual ? '' : 't-muted'}>
              <td>{fmtMonth(m.month, true)}</td>
              <td className="num">{bhd(m.income.eir_income)}</td>
              <td className="num">{bhd(-m.income.ftp)}</td>
              <td className="num">{bhd(m.income.carry)}</td>
              <td className="num">{bhd(m.income.irs)}</td>
              <td className="num t-medium">{bhd(m.income.net_nii)}</td>
              <td className="t-small">{m.actual ? '' : 'forecast'}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <Section title={`Full-year income by portfolio, ${year}`} meta="booked to date, then forecast" />
      <BookYearTable rows={f.by_book} names={names} testId="pmu-book-year" />
    </div>
  );
}
