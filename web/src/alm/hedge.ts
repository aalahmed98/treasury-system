// Trial hedges: what they are called, how one reads in a line, and the market rate a new one
// would start at. The legs themselves are built by crates/alm/src/hedge.rs.
import type { HedgeKind, RateCurve, TrialHedge } from '../api/types';
import { fmtMoneyShort } from '../lib/fmt';

export const HEDGE_KINDS: [HedgeKind, string][] = [
  ['PAY_FIXED_SWAP', 'Pay-fixed swap'],
  ['RECEIVE_FIXED_SWAP', 'Receive-fixed swap'],
  ['FIXED_FUNDING', 'Fixed-rate funding'],
  ['FIXED_INVESTMENT', 'Fixed-rate investment'],
];
export const HEDGE_LABEL = Object.fromEntries(HEDGE_KINDS) as Record<HedgeKind, string>;

export const HEDGE_HELP: Record<HedgeKind, string> = {
  PAY_FIXED_SWAP: 'Pay a fixed rate, receive floating. Gains when rates rise: use it when assets are longer than liabilities.',
  RECEIVE_FIXED_SWAP: 'Receive a fixed rate, pay floating. Gains when rates fall: use it when liabilities are longer than assets.',
  FIXED_FUNDING: 'Borrow at a fixed rate to the tenor and hold the cash in one-month money. Lengthens liabilities, like paying fixed.',
  FIXED_INVESTMENT: 'Invest at a fixed rate to the tenor, funded with one-month money. Lengthens assets, like receiving fixed.',
};

export const TENORS: [string, number][] = [
  ['3M', 3],
  ['6M', 6],
  ['1Y', 12],
  ['2Y', 24],
  ['3Y', 36],
  ['5Y', 60],
  ['7Y', 84],
  ['10Y', 120],
];

export const tenorLabel = (months: number) => (months % 12 === 0 ? `${months / 12}Y` : `${months}M`);

/** `Pay-fixed swap · BHD 50.0m · 5Y · at market`. */
export function hedgeLabel(h: TrialHedge): string {
  const rate = h.rate === null || h.rate === undefined ? 'at market' : `${h.rate.toFixed(2)}%`;
  return `${HEDGE_LABEL[h.kind]} · ${h.ccy} ${fmtMoneyShort(h.notional)} · ${tenorLabel(h.tenor_months)} · ${rate}`;
}

export const hedgesSummary = (n: number) => (n === 0 ? 'none' : n === 1 ? '1 trial hedge' : `${n} trial hedges`);

/** Percent at `years` on a curve: linear between tenors, flat beyond the ends. */
export function curveRate(curve: RateCurve, years: number): number {
  const pts = curve.points.map((p) => [p.tenor_days / 365, p.rate] as [number, number]).sort((a, b) => a[0] - b[0]);
  if (!pts.length) return 0;
  if (years <= pts[0][0]) return pts[0][1];
  for (let i = 1; i < pts.length; i++) {
    const [t0, r0] = pts[i - 1];
    const [t1, r1] = pts[i];
    if (years <= t1) return t1 - t0 < 1e-12 ? r1 : r0 + ((r1 - r0) * (years - t0)) / (t1 - t0);
  }
  return pts[pts.length - 1][1];
}
