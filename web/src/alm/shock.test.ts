import { describe, expect, it } from 'vitest';
import { customLabel, normaliseBuckets, shockShape, tenorsLabel, toggleBucket } from './shock';

describe('the rate shock', () => {
  it('treats every bucket chosen as none, which means all', () => {
    expect(normaliseBuckets(['1Y', '3M'])).toEqual(['3M', '1Y']);
    expect(normaliseBuckets(['ON', '1W', '1M', '3M', '6M', '1Y', '2Y', '5Y', '10Y+'])).toEqual([]);
    expect(normaliseBuckets(['4M'])).toEqual([]);
  });

  it('adds and drops buckets, but never the last one', () => {
    expect(toggleBucket([], '10Y+')).toEqual(['ON', '1W', '1M', '3M', '6M', '1Y', '2Y', '5Y']);
    expect(toggleBucket(['3M'], '6M')).toEqual(['3M', '6M']);
    expect(toggleBucket(['3M', '6M'], '3M')).toEqual(['6M']);
    expect(toggleBucket(['6M'], '6M')).toEqual(['6M']);
  });

  it('names the tenors as a run when they are neighbours', () => {
    expect(tenorsLabel([])).toBe('all tenors');
    expect(tenorsLabel(['1Y'])).toBe('1Y');
    expect(tenorsLabel(['3M', '6M', '1Y'])).toBe('3M to 1Y');
    expect(tenorsLabel(['ON', '1Y'])).toBe('ON, 1Y');
    expect(customLabel(100, ['3M', '6M'])).toBe('+100 bp at 3M to 6M');
    expect(customLabel(-50, [])).toBe('−50 bp, all tenors');
  });

  it('draws each scenario across the buckets', () => {
    expect(shockShape('BASE', 100, [], null)).toEqual(Array(9).fill(0));
    expect(shockShape('CUSTOM', 100, [], null)).toEqual(Array(9).fill(100));
    expect(shockShape('CUSTOM', -25, ['3M', '6M'], null)).toEqual([0, 0, 0, -25, -25, 0, 0, 0, 0]);
    expect(shockShape('PARALLEL_DOWN', 0, [], null)).toEqual(Array(9).fill(-200));
    const steep = shockShape('STEEPENER', 0, [], null);
    expect(steep[0]).toBeLessThan(0);
    expect(steep[8]).toBeGreaterThan(0);
    const short = shockShape('SHORT_UP', 0, [], null);
    expect(short[0]).toBeCloseTo(300, 0);
    expect(short[8]).toBeLessThan(short[5]);
  });
});
