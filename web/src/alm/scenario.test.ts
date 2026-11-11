import { describe, expect, it } from 'vitest';
import type { ScenarioBody } from '../api/types';
import { applyChanges, toParamChanges } from './diff';
import { bodySummary, sameBody } from './scenario';

const body = (over: Partial<ScenarioBody> = {}): ScenarioBody => ({ scenario: 'BASE', custom_shock_bp: null, custom_shock_buckets: [], hedges: [], changes: [], ...over });

describe('saved scenarios', () => {
  it('reads in one line', () => {
    expect(bodySummary(body())).toBe('Today’s curve');
    expect(bodySummary(body({ scenario: 'PARALLEL_UP' }))).toBe('Parallel up');
    const full = body({
      scenario: 'CUSTOM',
      custom_shock_bp: 100,
      custom_shock_buckets: ['3M', '6M'],
      hedges: [{ kind: 'PAY_FIXED_SWAP', ccy: 'BHD', notional: 1e6, tenor_months: 12 }],
      changes: [{ path: ['nii', 'default_beta'], value: 0.9 }],
    });
    expect(bodySummary(full)).toBe('Custom +100 bp at 3M to 6M · 1 trial hedge · 1 assumption changed');
  });

  it('knows when the simulation on screen has moved on from the saved one', () => {
    const a = body({ scenario: 'CUSTOM', custom_shock_bp: 100, custom_shock_buckets: ['6M', '3M'] });
    expect(sameBody(a, body({ scenario: 'CUSTOM', custom_shock_bp: 100, custom_shock_buckets: ['3M', '6M'] }))).toBe(true);
    expect(sameBody(a, body({ scenario: 'CUSTOM', custom_shock_bp: 150, custom_shock_buckets: ['3M', '6M'] }))).toBe(false);
    // A size left over from an earlier custom shock does not count once the scenario is a Basel one.
    expect(sameBody(body({ scenario: 'PARALLEL_UP', custom_shock_bp: 100 }), body({ scenario: 'PARALLEL_UP' }))).toBe(true);
  });

  it('carries a what-if as changes and puts it back on another parameter set', () => {
    const bank = { nii: { default_beta: 0.5, betas: { SAVINGS: 0.4 } }, lcr: { significant_ccys: ['USD'] }, gap: { nmd_core_pct: {} } };
    const working = structuredClone(bank) as typeof bank & { gap: { nmd_core_pct: Record<string, number> } };
    working.nii.default_beta = 0.9;
    working.lcr.significant_ccys = ['USD', 'EUR'];
    working.gap.nmd_core_pct.SAVINGS = 60;
    const changes = toParamChanges(bank, working);
    expect(changes).toContainEqual({ path: ['nii', 'default_beta'], value: 0.9 });
    expect(changes).toContainEqual({ path: ['lcr', 'significant_ccys'], value: ['USD', 'EUR'] });
    expect(changes).toContainEqual({ path: ['gap', 'nmd_core_pct', 'SAVINGS'], value: 60 });
    // On a later bank version with a different savings beta: only the changed leaves move.
    const later = { nii: { default_beta: 0.5, betas: { SAVINGS: 0.3 } }, lcr: { significant_ccys: ['USD'] }, gap: {} as Record<string, unknown> };
    applyChanges(later, changes);
    expect(later).toEqual({ nii: { default_beta: 0.9, betas: { SAVINGS: 0.3 } }, lcr: { significant_ccys: ['USD', 'EUR'] }, gap: { nmd_core_pct: { SAVINGS: 60 } } });
    applyChanges(later, [{ path: ['gap', 'nmd_core_pct', 'SAVINGS'], value: null }]);
    expect(later.gap).toEqual({ nmd_core_pct: {} });
  });
});
