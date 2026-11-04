// Interest rate differential: covered interest parity on two money-market rates, ported from
// crates/alm/src/ird.rs. Rates in percent, simple interest on the currency's day basis.
import type { CurvePoint, MarketResponse } from '../api/types';

export const pipFactor = (quoteCcy: string) => (quoteCcy === 'JPY' ? 100 : 10_000);
export const basis = (ccy: string) => (ccy === 'GBP' ? 365 : 360);

/** Outright forward for a pair quoted as units of the quote currency per one base unit. */
export function forward(spot: number, days: number, baseRate: number, baseBasis: number, quoteRate: number, quoteBasis: number): number {
  const d = Math.max(0, days);
  return (spot * (1 + (quoteRate / 100) * (d / quoteBasis))) / (1 + (baseRate / 100) * (d / baseBasis));
}

/** The quote-currency rate that makes `fwd` fair, given the base rate. */
export function impliedQuoteRate(spot: number, fwd: number, days: number, baseRate: number, baseBasis: number, quoteBasis: number): number | null {
  if (spot <= 0 || days <= 0) return null;
  return (((fwd / spot) * (1 + (baseRate / 100) * (days / baseBasis)) - 1) * quoteBasis * 100) / days;
}

/** The base-currency rate that makes `fwd` fair, given the quote rate. */
export function impliedBaseRate(spot: number, fwd: number, days: number, quoteRate: number, quoteBasis: number, baseBasis: number): number | null {
  if (fwd <= 0 || days <= 0) return null;
  return (((spot / fwd) * (1 + (quoteRate / 100) * (days / quoteBasis)) - 1) * baseBasis * 100) / days;
}

/** Daily carry in the quote currency of holding `notional` of the base currency funded in the quote currency. */
export function carryPerDay(notional: number, spot: number, baseRate: number, baseBasis: number, quoteRate: number, quoteBasis: number): number {
  return notional * spot * (baseRate / 100 / baseBasis - quoteRate / 100 / quoteBasis);
}

export interface IrdQuote {
  tenor: string;
  days: number;
  baseRate: number;
  quoteRate: number;
  forward: number;
  points: number;
}

export function quote(tenor: string, days: number, spot: number, baseRate: number, baseCcy: string, quoteRate: number, quoteCcy: string): IrdQuote {
  const f = forward(spot, days, baseRate, basis(baseCcy), quoteRate, basis(quoteCcy));
  return { tenor, days, baseRate, quoteRate, forward: f, points: (f - spot) * pipFactor(quoteCcy) };
}

/** Linear interpolation on a curve by years, clamped at the ends, as crates/alm/src/curve.rs. */
export function curveRate(points: CurvePoint[], years: number): number | null {
  if (!points.length) return null;
  const pts = [...points].sort((a, b) => a.tenor_days - b.tenor_days).map((p) => [p.tenor_days / 365, p.rate] as const);
  if (years <= pts[0][0]) return pts[0][1];
  const last = pts[pts.length - 1];
  if (years >= last[0]) return last[1];
  for (let i = 1; i < pts.length; i++) {
    const [t0, r0] = pts[i - 1];
    const [t1, r1] = pts[i];
    if (years <= t1) return r0 + ((r1 - r0) * (years - t0)) / (t1 - t0);
  }
  return last[1];
}

export function marketRate(m: MarketResponse, ccy: string, days: number): number | null {
  const c = m.curves.find((x) => x.ccy === ccy);
  return c ? curveRate(c.points, days / 365) : null;
}

/** The spot for base/quote from the market's pairs, inverting when only the reverse is quoted. */
export function marketSpot(m: MarketResponse, baseCcy: string, quoteCcy: string): number | null {
  if (baseCcy === quoteCcy) return 1;
  const direct = m.spots.find((s) => s.pair === `${baseCcy}/${quoteCcy}` || s.pair === `${baseCcy}${quoteCcy}`);
  if (direct) return direct.mid;
  const inverse = m.spots.find((s) => s.pair === `${quoteCcy}/${baseCcy}` || s.pair === `${quoteCcy}${baseCcy}`);
  if (inverse && inverse.mid > 0) return 1 / inverse.mid;
  // Through a common currency, when both legs are quoted against it.
  for (const via of ['USD', 'BHD']) {
    const a = marketSpotDirect(m, baseCcy, via);
    const b = marketSpotDirect(m, via, quoteCcy);
    if (a !== null && b !== null) return a * b;
  }
  return null;
}

function marketSpotDirect(m: MarketResponse, base: string, quote: string): number | null {
  if (base === quote) return 1;
  const d = m.spots.find((s) => s.pair === `${base}/${quote}` || s.pair === `${base}${quote}`);
  if (d) return d.mid;
  const i = m.spots.find((s) => s.pair === `${quote}/${base}` || s.pair === `${quote}${base}`);
  return i && i.mid > 0 ? 1 / i.mid : null;
}

/** The market outright for a pair and value date: spot crossed through the dinar, carried forward
 * on the two currencies' curves. Units of ccy2 per ccy1; null when either currency has no spot. */
export function marketOutright(m: MarketResponse, ccy1: string, ccy2: string, days: number): number | null {
  const spot = marketSpot(m, ccy1, ccy2);
  if (spot === null) return null;
  if (days <= 0) return spot;
  return forward(spot, days, marketRate(m, ccy1, days) ?? 0, basis(ccy1), marketRate(m, ccy2, days) ?? 0, basis(ccy2));
}
