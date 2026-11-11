import { describe, expect, it } from 'vitest';
import { curveRate, hedgeLabel, hedgesSummary, tenorLabel } from './hedge';

describe('trial hedges', () => {
  it('reads in one line', () => {
    expect(tenorLabel(60)).toBe('5Y');
    expect(tenorLabel(18)).toBe('18M');
    expect(hedgeLabel({ kind: 'PAY_FIXED_SWAP', ccy: 'BHD', notional: 50_000_000, tenor_months: 60, rate: null })).toBe('Pay-fixed swap · BHD 50.0m · 5Y · at market');
    expect(hedgeLabel({ kind: 'FIXED_FUNDING', ccy: 'USD', notional: 1_000_000, tenor_months: 6, rate: 4.5 })).toContain('6M · 4.50%');
    expect([hedgesSummary(0), hedgesSummary(1), hedgesSummary(3)]).toEqual(['none', '1 trial hedge', '3 trial hedges']);
  });

  it('reads the market rate off a curve as the server does', () => {
    const curve = { ccy: 'BHD', index_code: 'BHIBOR', as_of: '2026-11-08', points: [{ tenor: '1Y', tenor_days: 365, rate: 6 }, { tenor: 'ON', tenor_days: 0, rate: 5 }] };
    expect(curveRate(curve, 0.5)).toBeCloseTo(5.5, 12);
    expect(curveRate(curve, 3)).toBe(6);
    expect(curveRate(curve, -1)).toBe(5);
    expect(curveRate({ ...curve, points: [] }, 1)).toBe(0);
  });
});
