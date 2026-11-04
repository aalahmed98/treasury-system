// Interest rate differential calculator: covered interest parity on the latest curves, run here
// without a server round trip.
import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { endpoints } from '../../api/client';
import { Card } from '../../components/ui';
import { Headline, KV, Num, ResultsHeader, Row, SimpleTable, ToolLayout } from '../../alm/tool';
import { basis, carryPerDay, forward, impliedBaseRate, impliedQuoteRate, marketRate, marketSpot, pipFactor, quote } from '../../lib/ird';
import { fmtAmount, fmtDateLong } from '../../lib/fmt';

const CCYS = ['BHD', 'USD', 'EUR', 'GBP', 'SAR', 'AED'];
const TENORS: [string, number][] = [
  ['1W', 7],
  ['1M', 30],
  ['3M', 91],
  ['6M', 182],
  ['1Y', 365],
  ['2Y', 730],
];

interface Ird {
  base: string;
  quote: string;
  spot: number;
  days: number;
  baseRate: number;
  quoteRate: number;
  notional: number;
  forwardInput: number;
  follow: boolean;
}

export function IrdPage() {
  const market = useQuery({ queryKey: ['market', 'rates'], queryFn: endpoints.marketRates, staleTime: 60_000 });
  const m = market.data;
  const [s, setS] = useState<Ird>({ base: 'USD', quote: 'BHD', spot: 0.376, days: 90, baseRate: 4.3, quoteRate: 5.9, notional: 10_000_000, forwardInput: 0, follow: true });

  // Pull spot and rates for the tenor from the market while following it.
  useEffect(() => {
    if (!m || !s.follow) return;
    const spot = marketSpot(m, s.base, s.quote);
    const br = marketRate(m, s.base, s.days);
    const qr = marketRate(m, s.base === s.quote ? s.base : s.quote, s.days);
    setS((prev) => ({ ...prev, spot: spot ?? prev.spot, baseRate: br ?? prev.baseRate, quoteRate: qr ?? prev.quoteRate }));
  }, [m, s.follow, s.base, s.quote, s.days]);

  const bb = basis(s.base);
  const qb = basis(s.quote);
  const pip = pipFactor(s.quote);
  const f = forward(s.spot, s.days, s.baseRate, bb, s.quoteRate, qb);
  const points = (f - s.spot) * pip;
  const carry = carryPerDay(s.notional, s.spot, s.baseRate, bb, s.quoteRate, qb);
  const rows: [string, string][] = [
    ['Spot', s.spot.toFixed(5)],
    ['Forward points', `${points >= 0 ? '+' : ''}${points.toFixed(1)}`],
    [`Carry per day (${s.quote})`, fmtAmount(carry.toFixed(2))],
    [`Carry over the tenor (${s.quote})`, fmtAmount((carry * s.days).toFixed(2))],
    ['Break-even spot move', `${points >= 0 ? '+' : ''}${points.toFixed(1)} pips`],
  ];
  if (s.forwardInput > 0) {
    const iq = impliedQuoteRate(s.spot, s.forwardInput, s.days, s.baseRate, bb, qb);
    const ib = impliedBaseRate(s.spot, s.forwardInput, s.days, s.quoteRate, qb, bb);
    if (iq !== null) rows.push([`Implied ${s.quote} rate for forward ${s.forwardInput.toFixed(5)}`, `${iq.toFixed(3)}%`]);
    if (ib !== null) rows.push([`Implied ${s.base} rate for forward ${s.forwardInput.toFixed(5)}`, `${ib.toFixed(3)}%`]);
  }
  const tenorRows = TENORS.map(([t, d]) => {
    const br = m && s.follow ? (marketRate(m, s.base, d) ?? s.baseRate) : s.baseRate;
    const qr = m && s.follow ? (marketRate(m, s.quote, d) ?? s.quoteRate) : s.quoteRate;
    const q = quote(t, d, s.spot, br, s.base, qr, s.quote);
    return [t, `${br.toFixed(3)}%`, `${qr.toFixed(3)}%`, q.forward.toFixed(5), `${q.points >= 0 ? '+' : ''}${q.points.toFixed(1)}`];
  });
  const asOf = m?.curves[0]?.as_of;
  const typed = (patch: Partial<Ird>) => setS((prev) => ({ ...prev, ...patch, follow: false }));

  const inputs = (
    <>
      <div className="t-heading" style={{ marginBottom: 6 }}>
        Pair and tenor
      </div>
      <div className="row" style={{ marginBottom: 6 }}>
        <select className="input input-sm" value={s.base} onChange={(e) => setS({ ...s, base: e.target.value })} aria-label="base currency">
          {CCYS.map((c) => (
            <option key={c}>{c}</option>
          ))}
        </select>
        <span>/</span>
        <select className="input input-sm" value={s.quote} onChange={(e) => setS({ ...s, quote: e.target.value })} aria-label="quote currency">
          {CCYS.map((c) => (
            <option key={c}>{c}</option>
          ))}
        </select>
        <span className="t-small">quote per base</span>
      </div>
      <Row label="Days">
        <Num value={s.days} min={1} max={3650} step={1} onChange={(v) => setS({ ...s, days: Math.round(v) })} />
      </Row>
      <Row label="Follow the market">
        <input type="checkbox" checked={s.follow} onChange={(e) => setS({ ...s, follow: e.target.checked })} />
      </Row>
      <Row label="Spot">
        <Num value={s.spot} min={0.0001} max={10000} step={0.0001} decimals={5} width={100} onChange={(v) => typed({ spot: v })} />
      </Row>
      <Row label={`${s.base} rate`}>
        <Num value={s.baseRate} min={-5} max={50} step={0.01} decimals={3} suffix="%" onChange={(v) => typed({ baseRate: v })} />
      </Row>
      <Row label={`${s.quote} rate`}>
        <Num value={s.quoteRate} min={-5} max={50} step={0.01} decimals={3} suffix="%" onChange={(v) => typed({ quoteRate: v })} />
      </Row>
      <Row label={`Notional (${s.base})`}>
        <Num value={s.notional} min={0} max={1e12} step={100_000} width={120} onChange={(v) => setS({ ...s, notional: v })} />
      </Row>
      <Row label="Forward to solve for">
        <Num value={s.forwardInput} min={0} max={10000} step={0.0001} decimals={5} width={100} onChange={(v) => setS({ ...s, forwardInput: v })} />
      </Row>
      <p className="t-small" style={{ marginTop: 10 }}>{asOf ? `curves and spots as of ${fmtDateLong(asOf)}` : 'no market curves loaded'}</p>
      <p className="t-small">Simple interest on each currency's basis (ACT/365 for GBP, ACT/360 otherwise). Typing a spot or rate stops following the market.</p>
    </>
  );

  return (
    <ToolLayout assumptions={inputs} ready>
      <ResultsHeader title={`${s.base}/${s.quote} ${s.days} days`} loading={market.isLoading} error={market.error?.message} />
      <Headline value={f.toFixed(5)} standing={points >= 0 ? 'comfortable' : 'watch'} line1={`forward outright · ${points >= 0 ? '+' : ''}${points.toFixed(1)} pips`} line2={`differential ${s.quoteRate - s.baseRate >= 0 ? '+' : ''}${(s.quoteRate - s.baseRate).toFixed(3)}% (${s.quote} over ${s.base})`} testId="headline" />
      <div className="row-wrap">
        <Card title="This tenor" width={380}>
          <KV rows={rows} />
        </Card>
        <Card title="Standard tenors from the curves" width={480}>
          <SimpleTable headers={['Tenor', `${s.base} rate`, `${s.quote} rate`, 'Forward', 'Points']} rows={tenorRows} />
        </Card>
      </div>
    </ToolLayout>
  );
}
