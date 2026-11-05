// Funding and repo: how the portfolio is funded and what cash it will bring in. The repo book by
// what is left to run, what was raised and repaid, the collateral pledged against what is free,
// the latest quotes over SOFR, the coupons and redemptions of the next year, and the swaps'
// settlement cash kept apart from income.
import { useMemo } from 'react';
import { useSearchParams } from 'react-router';
import { useSession } from '../../api/session';
import { usePmuFunding, usePmuOverview } from '../../api/queries';
import type { PmuFunding } from '../../api/types';
import { fmtDateLong, fmtDateShort, fmtMonth } from '../../lib/fmt';
import { Badge, Section, StatStrip } from '../../components/ui';
import { Bars, BarsPair, StackedBars } from '../../components/charts';
import { BookYearTable, Loading, bhd, pct } from './common';

export function FundingPage() {
  const q = usePmuFunding();
  const ov = usePmuOverview();
  const names = useMemo(() => Object.fromEntries((ov.data?.by_book ?? []).map((b) => [b.book, b.name])), [ov.data]);
  return (
    <div className="page" data-ready={q.data ? '1' : undefined} data-testid="pmu-funding">
      <Loading q={q}>{(f) => <Body f={f} names={names} />}</Loading>
    </div>
  );
}

const mon = (m: string) => fmtMonth(m);

function Body({ f, names }: { f: PmuFunding; names: Record<string, string> }) {
  const s = f.summary;
  const { canBookPmu } = useSession();
  const [params, setParams] = useSearchParams();
  const act = (id: string) => {
    const next = new URLSearchParams(params);
    next.set('repo', id);
    next.delete('book');
    setParams(next, { replace: true });
  };
  const thisMonth = f.as_of.slice(0, 7);
  return (
    <>
      <Section title="Liquidity and repo" meta={`as of ${fmtDateLong(f.as_of)} · in BHD`} />
      <StatStrip
        testId="pmu-funding-strip"
        hero
        cells={[
          { label: 'Repo amount', value: bhd(s.amount_bhd), sub: `${s.active} active repos` },
          { label: 'Available face', value: bhd(s.available_face_bhd), sub: `free to pledge; ${bhd(s.pledged_face_bhd)} pledged` },
          { label: 'Due within 7 days', value: bhd(s.due_7d_bhd), sub: 'to repay or roll', tone: s.due_7d_bhd > 0 ? 'warn' : 'good' },
          { label: 'Average repo rate', value: pct(s.avg_rate), sub: `haircut ${pct(s.avg_haircut, 1)} · ${Math.round(s.wtd_maturity_days)} days to run` },
          { label: 'Receipts, next 12 months', value: bhd(s.receipts_12m_bhd), sub: 'coupons and redemptions' },
          { label: 'Cash lent', value: bhd(s.reverse_bhd), sub: `${s.reverse_active} reverse ${s.reverse_active === 1 ? 'repo' : 'repos'}` },
        ]}
      />

      <div className="panel-row pmu-three">
        <div className="panel">
          <div className="t-heading">Active repos by time left</div>
          <Bars data={f.by_remaining.map((b) => [b.label.replace(' months', 'm').replace(' days', 'd').replace('Over 1 year', '>1y'), b.amount])} fmt={bhd} labels="all" colorOf={(_, i) => (i === 0 ? 'var(--s2)' : 'var(--s1)')} />
        </div>
        <div className="panel">
          <div className="t-heading">Funding raised and repaid, by month</div>
          <BarsPair data={f.funding_flows.map((m) => [mon(m.month), m.a, m.b])} names={['raised', 'repaid']} fmt={bhd} />
        </div>
        <div className="panel">
          <div className="t-heading">Collateral by book, face</div>
          <StackedBars data={f.collateral_by_book.map((c) => [c.book, c.pledged_bhd, c.free_bhd])} names={['pledged', 'free']} colors={['var(--s2)', 'var(--s3)']} fmt={bhd} testId="pmu-collateral" />
        </div>
      </div>

      <Section title="Repo book" meta={`${f.repos.length} active, soonest first`} />
      <table className="table" data-testid="pmu-repos">
        <thead>
          <tr>
            <th>Counterparty</th>
            <th>Kondor ID</th>
            <th>Started</th>
            <th>Ends</th>
            <th className="num">Days left</th>
            <th className="num">Cash</th>
            <th className="num">Rate</th>
            <th className="num">Haircut</th>
            <th className="num">Collateral face</th>
            <th>Bonds</th>
            {canBookPmu && <th />}
          </tr>
        </thead>
        <tbody>
          {f.repos.map((r) => (
            <tr key={r.id}>
              <td>
                {r.counterparty} {r.direction === 'REVERSE' && <Badge tone="info">reverse</Badge>}
              </td>
              <td className="t-small">{r.kondor_id ?? (r.provisional ? <Badge tone="warn">provisional</Badge> : '—')}</td>
              <td>{fmtDateShort(r.start)}</td>
              <td>
                {fmtDateShort(r.end)} {r.closed_early && <Badge>closed early</Badge>}
              </td>
              <td className="num">{r.days_left <= 7 ? <Badge tone="warn">{r.days_left}</Badge> : r.days_left}</td>
              <td className="num">{bhd(r.cash_bhd)}</td>
              <td className="num">{pct(r.rate, 3)}</td>
              <td className="num">{pct(r.haircut_pct, 1)}</td>
              <td className="num">{bhd(r.collateral_face_bhd)}</td>
              <td className="t-small" title={r.collateral_note ?? ''}>
                {r.isins.join(', ')}
              </td>
              {canBookPmu && (
                <td>
                  <button
                    type="button"
                    className="btn btn-ghost btn-sm"
                    onClick={() => act(r.id)}
                    aria-label={r.closed_early ? `roll the repo with ${r.counterparty}, closing ${r.end}` : `close or roll the repo with ${r.counterparty} to ${r.end}`}
                  >
                    {r.closed_early ? 'Roll' : 'Close or roll'}
                  </button>
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>

      <div className="panel-row two">
        <div className="panel">
          <div className="t-heading">Latest repo quotes, spread over SOFR</div>
          <table className="table" data-testid="pmu-quotes">
            <thead>
              <tr>
                <th>Counterparty</th>
                <th>Tenor</th>
                <th className="num">Spread</th>
                <th className="num">SOFR</th>
                <th className="num">All-in</th>
                <th>Quoted</th>
              </tr>
            </thead>
            <tbody>
              {f.quotes.map((q) => (
                <tr key={`${q.counterparty}${q.tenor}`} className={q.age_days > 2 ? 't-muted' : ''}>
                  <td>{q.counterparty}</td>
                  <td>{q.tenor}</td>
                  <td className="num">{q.spread_bp.toFixed(1)} bp</td>
                  <td className="num">{pct(q.sofr, 3)}</td>
                  <td className="num">{q.sofr === null ? '—' : pct(q.sofr + q.spread_bp / 100, 3)}</td>
                  <td>
                    {fmtDateShort(q.as_of)} {q.age_days > 2 && <Badge tone="warn">{q.age_days} days old</Badge>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="panel">
          <div className="t-heading">Coupon and principal receipts, next 12 months</div>
          <StackedBars data={f.receipts.map((m) => [fmtMonth(m.month), m.a, m.b])} names={['coupons', 'principal']} fmt={bhd} testId="pmu-receipts" />
          <div className="t-heading" style={{ marginTop: 14 }}>
            Hedge settlement cash, separate from income
          </div>
          <Bars
            data={f.hedge_cash.map((m) => [mon(m.month), m.a + m.b])}
            dim={f.hedge_cash.map((m) => m.month.slice(0, 7) > thisMonth)}
            dimLegend={['settled', 'projected']}
            fmt={bhd}
            labels="extremes"
            colorOf={(v) => (v < 0 ? 'var(--bad)' : 'var(--s1)')}
          />
          <div className="t-small">Net periodic settlements of the swaps plus any unwind cash, by month; received above the line, paid below.</div>
        </div>
      </div>

      <Section title={`Full-year income by portfolio, ${f.as_of.slice(0, 4)}`} meta="booked to date, then forecast with the portfolio running off" />
      <BookYearTable rows={f.by_book_year} names={names} />
    </>
  );
}
