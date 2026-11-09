import { describe, expect, it } from 'vitest';
import { niceTicks, trimZeros } from './charts';

describe('niceTicks', () => {
  it('covers a positive range from zero on a round step', () => {
    expect(niceTicks(0, 14.7e6)).toEqual([0, 5e6, 10e6, 15e6]);
  });
  it('keeps zero when the range crosses it, with the least slack', () => {
    const t = niceTicks(-1134e6, 929e6);
    expect(t).toContain(0);
    expect(t[0]).toBeLessThanOrEqual(-1134e6);
    expect(t[t.length - 1]).toBeGreaterThanOrEqual(929e6);
    expect(t.length).toBeGreaterThanOrEqual(3);
    expect(t.length).toBeLessThanOrEqual(6);
  });
  it('handles a flat range', () => {
    expect(niceTicks(0, 0)).toEqual([0]);
  });
});

describe('trimZeros', () => {
  it('drops padding zeros and keeps the unit and sign', () => {
    expect(trimZeros('5.00m')).toBe('5m');
    expect(trimZeros('15.0m')).toBe('15m');
    expect(trimZeros('-1.50bn')).toBe('-1.5bn');
    expect(trimZeros('+500m')).toBe('+500m');
    expect(trimZeros('2.50k')).toBe('2.5k');
    expect(trimZeros('0')).toBe('0');
    expect(trimZeros('3.25%')).toBe('3.25%');
  });
});
