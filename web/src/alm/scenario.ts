// Saved scenarios: how one reads in a line, and whether the simulation on screen still matches it.
import type { ScenarioBody } from '../api/types';
import { SCENARIO_LABEL } from '../lib/alm';
import { hedgesSummary } from './hedge';
import { customLabel } from './shock';

/** `Parallel up · 2 trial hedges · 3 assumptions changed`. */
export function bodySummary(b: ScenarioBody): string {
  const shock = b.scenario === 'BASE' ? 'Today’s curve' : b.scenario === 'CUSTOM' ? `Custom ${customLabel(b.custom_shock_bp ?? 0, b.custom_shock_buckets)}` : SCENARIO_LABEL[b.scenario];
  const parts = [shock];
  if (b.hedges.length) parts.push(hedgesSummary(b.hedges.length));
  if (b.changes.length) parts.push(b.changes.length === 1 ? '1 assumption changed' : `${b.changes.length} assumptions changed`);
  return parts.join(' · ');
}

/** The body with everything in a fixed order, for comparing two of them. */
function canonical(b: ScenarioBody): string {
  return JSON.stringify({
    scenario: b.scenario,
    bp: b.scenario === 'CUSTOM' ? (b.custom_shock_bp ?? 0) : null,
    buckets: b.scenario === 'CUSTOM' ? [...b.custom_shock_buckets].sort() : [],
    hedges: b.hedges.map((h) => [h.kind, h.ccy, h.notional, h.tenor_months, h.rate ?? null, h.reset_months ?? null]),
    changes: b.changes.map((c) => [c.path.join('\u0000'), c.value]).sort((x, y) => String(x[0]).localeCompare(String(y[0]))),
  });
}

export const sameBody = (a: ScenarioBody, b: ScenarioBody) => canonical(a) === canonical(b);
