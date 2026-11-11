// The rate shock every ALM rates tool runs under: which tools it reaches, the shape it has across
// the gap buckets, and how it reads in words. The shapes mirror crates/alm/src/curve.rs.
import type { AlmParams, RateScenario } from '../api/types';

export const BUCKETS = ['ON', '1W', '1M', '3M', '6M', '1Y', '2Y', '5Y', '10Y+'] as const;
/** The tenor in years each bucket's bar is drawn at: its far edge, ten years for the open one. */
export const BUCKET_YEARS = [1 / 365, 7 / 365, 1 / 12, 0.25, 0.5, 1, 2, 5, 10];

export const SHORTCUTS: [string, string[]][] = [
  ['All tenors', []],
  ['Up to 1Y', ['ON', '1W', '1M', '3M', '6M', '1Y']],
  ['Over 1Y', ['2Y', '5Y', '10Y+']],
];

/** The tools whose figures a rate shock changes. */
export const RATE_TOOLS = new Set(['gap', 'nii', 'eve', 'stress']);

/** Buckets in ladder order; every bucket chosen is the same as none, which means all. */
export function normaliseBuckets(buckets: string[]): string[] {
  const on = BUCKETS.filter((b) => buckets.includes(b));
  return on.length === BUCKETS.length ? [] : [...on];
}

/** Add or drop a bucket; the last one cannot be dropped. */
export function toggleBucket(buckets: string[], label: string): string[] {
  const current = buckets.length === 0 ? [...BUCKETS] : buckets;
  const next = current.includes(label) ? current.filter((b) => b !== label) : [...current, label];
  return next.length === 0 ? buckets : normaliseBuckets(next);
}

/** The buckets as a run (`3M to 1Y`) when they are neighbours, else a list. */
export function tenorsLabel(buckets: string[]): string {
  const on = normaliseBuckets(buckets);
  if (on.length === 0) return 'all tenors';
  const idx = on.map((b) => BUCKETS.indexOf(b as (typeof BUCKETS)[number]));
  const run = idx[idx.length - 1] - idx[0] + 1 === idx.length;
  if (on.length === 1) return on[0];
  return run ? `${on[0]} to ${on[on.length - 1]}` : on.join(', ');
}

const bp = (v: number) => `${v > 0 ? '+' : v < 0 ? '−' : ''}${Math.abs(Math.round(v))} bp`;

/** The Basel shock sizes the shape is drawn with: the reporting currency's, else USD's. */
export function shockSizes(params: AlmParams | null): { ccy: string; parallel: number; short: number; long: number } {
  const ccy = params?.reporting_ccy ?? '';
  const own = params?.stress.shocks[ccy];
  if (own) return { ccy, ...own };
  const usd = params?.stress.shocks.USD;
  return usd ? { ccy: 'USD', ...usd } : { ccy: 'USD', parallel: 200, short: 300, long: 150 };
}

/** Basis points applied at each bucket, before the post-shock floor. */
export function shockShape(scenario: RateScenario, customBp: number, buckets: string[], params: AlmParams | null): number[] {
  const s = shockSizes(params);
  const chosen = normaliseBuckets(buckets);
  return BUCKET_YEARS.map((t, i) => {
    const short = Math.exp(-t / 4);
    const long = 1 - short;
    switch (scenario) {
      case 'BASE':
        return 0;
      case 'PARALLEL_UP':
        return s.parallel;
      case 'PARALLEL_DOWN':
        return -s.parallel;
      case 'SHORT_UP':
        return s.short * short;
      case 'SHORT_DOWN':
        return -s.short * short;
      case 'STEEPENER':
        return -0.65 * s.short * short + 0.9 * s.long * long;
      case 'FLATTENER':
        return 0.8 * s.short * short - 0.6 * s.long * long;
      case 'CUSTOM':
        return chosen.length === 0 || chosen.includes(BUCKETS[i]) ? customBp : 0;
    }
  });
}

/** One line per bucket, for the shape's tooltip. */
export function shapeTitle(shape: number[]): string {
  return shape.map((v, i) => `${BUCKETS[i]} ${bp(v)}`).join(' · ');
}

/** The custom shock in words, as the server labels it. */
export function customLabel(customBp: number, buckets: string[]): string {
  const sign = customBp >= 0 ? '+' : '−';
  const size = `${sign}${Math.abs(Math.round(customBp))} bp`;
  return normaliseBuckets(buckets).length === 0 ? `${size}, all tenors` : `${size} at ${tenorsLabel(buckets)}`;
}
