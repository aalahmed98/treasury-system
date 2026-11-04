import { describe, expect, it } from 'vitest';
import { basis, carryPerDay, curveRate, forward, impliedBaseRate, impliedQuoteRate, pipFactor, quote } from './ird';

const close = (a: number, b: number, eps: number) => Math.abs(a - b) <= eps;

describe('ird follows covered interest parity (the alm crate cases)', () => {
  it('forward, points, implied rates and carry', () => {
    const f = forward(1.1, 90, 3.0, 360, 5.0, 360);
    expect(close(f, (1.1 * 1.0125) / 1.0075, 1e-12)).toBe(true);
    const q = quote('3M', 90, 1.1, 3.0, 'EUR', 5.0, 'USD');
    expect(close(q.points, (f - 1.1) * 10_000, 1e-9)).toBe(true);
    expect(q.points).toBeGreaterThan(54);
    expect(q.points).toBeLessThan(55);
    expect(close(impliedQuoteRate(1.1, f, 90, 3.0, 360, 360)!, 5.0, 1e-9)).toBe(true);
    expect(close(impliedBaseRate(1.1, f, 90, 5.0, 360, 360)!, 3.0, 1e-9)).toBe(true);
    expect(close(carryPerDay(1_000_000, 1.1, 3.0, 360, 5.0, 360), (-1_000_000 * 1.1 * 0.02) / 360, 1e-9)).toBe(true);
    expect(pipFactor('JPY')).toBe(100);
    expect(basis('GBP')).toBe(365);
    expect(impliedQuoteRate(1.1, f, 0, 3.0, 360, 360)).toBeNull();
  });
  it('interpolates a curve by years and clamps at the ends', () => {
    const pts = [
      { tenor: '1M', tenor_days: 30, rate: 4.0 },
      { tenor: '1Y', tenor_days: 365, rate: 5.0 },
    ];
    expect(curveRate(pts, 30 / 365)).toBe(4.0);
    expect(curveRate(pts, 2)).toBe(5.0);
    expect(close(curveRate(pts, (30 + (365 - 30) / 2) / 365)!, 4.5, 1e-9)).toBe(true);
  });
});
