// ALM view logic ported from crates/client/src/alm_state.rs: standings, labels and the overview
// block's cells and limit rows.
import type { AlmOverview, LimitRow, RateScenario } from '../api/types';
import type { StatCell, Tone } from '../components/ui';
import { fmtMoneyShort, fmtPct } from './fmt';

export type Standing = 'comfortable' | 'watch' | 'breach' | 'unknown';

export const standingTone = (s: Standing): Tone => (s === 'comfortable' ? 'good' : s === 'watch' ? 'warn' : s === 'breach' ? 'bad' : 'neutral');
export const standingLabel = (s: Standing): string => (s === 'comfortable' ? 'within limits' : s === 'watch' ? 'watch' : s === 'breach' ? 'breach' : 'no data');

/** A ratio against its floors: below the higher of the minima is a breach, below the warning a watch. */
export function standing(ratio: number | null, minimum: number, internal: number, warning: number): Standing {
  if (ratio === null) return 'unknown';
  if (ratio < Math.max(minimum, internal)) return 'breach';
  if (ratio < warning) return 'watch';
  return 'comfortable';
}

export const SCENARIO_LABEL: Record<RateScenario, string> = {
  BASE: 'Base',
  PARALLEL_UP: 'Parallel up',
  PARALLEL_DOWN: 'Parallel down',
  STEEPENER: 'Steepener',
  FLATTENER: 'Flattener',
  SHORT_UP: 'Short rates up',
  SHORT_DOWN: 'Short rates down',
  CUSTOM: 'Custom parallel',
};

export const pctOrDash = (v: number | null) => (v === null ? '—' : fmtPct(v, 1));
export const basisLabel = (version: number) => `bank parameters v${version}`;

const flags = (b: boolean, w: boolean): Standing => (b ? 'breach' : w ? 'watch' : 'comfortable');

export interface OverviewCell extends StatCell {
  tool: string;
}

/** The eight balance-sheet cells, each pointing at its tool. */
export function overviewCells(raw: AlmOverview): OverviewCell[] {
  const lim = (m: string) => raw.limits.find((l) => l.metric.startsWith(m));
  const st = (l: LimitRow | undefined): Standing => flags(l?.breach ?? false, l?.warn ?? false);
  const budget = (a: number, b: number) => (Math.abs(b) > 1e-9 ? `${((a - b) / Math.abs(b)) * 100 >= 0 ? '+' : ''}${(((a - b) / Math.abs(b)) * 100).toFixed(1)}% vs budget ${fmtMoneyShort(b)}` : 'no budget');
  const lcr = lim('LCR');
  const nsfr = lim('NSFR');
  const cells: OverviewCell[] = [
    { tool: 'lcr', label: 'LCR', value: pctOrDash(raw.lcr), sub: `internal minimum ${lcr ? `${lcr.limit.toFixed(0)}%` : ''}`, tone: standingTone(st(lcr)) },
    { tool: 'nsfr', label: 'NSFR', value: pctOrDash(raw.nsfr), sub: `internal minimum ${nsfr ? `${nsfr.limit.toFixed(0)}%` : ''}`, tone: standingTone(st(nsfr)) },
    { tool: 'nii', label: 'NII next 12 months', value: fmtMoneyShort(raw.nii_12m), sub: `NIM ${raw.nim === null ? '—' : `${raw.nim.toFixed(2)}%`}`, tone: 'good' },
    { tool: 'attribution', label: 'NII month to date', value: fmtMoneyShort(raw.nii_mtd), sub: budget(raw.nii_mtd, raw.budget_mtd), tone: raw.nii_mtd < raw.budget_mtd * 0.95 ? 'warn' : 'good' },
    { tool: 'attribution', label: 'NII year to date', value: fmtMoneyShort(raw.nii_ytd), sub: budget(raw.nii_ytd, raw.budget_ytd), tone: raw.nii_ytd < raw.budget_ytd * 0.95 ? 'warn' : 'good' },
    { tool: 'gap', label: 'Cumulative gap 1Y', value: `${raw.gap_1y_pct >= 0 ? '+' : ''}${raw.gap_1y_pct.toFixed(1)}%`, sub: 'of total assets', tone: standingTone(st(lim('Cumulative'))) },
    {
      tool: 'stress',
      label: 'Worst EVE scenario',
      value: raw.worst_eve_loss_pct_tier1 === null ? '—' : `${raw.worst_eve_loss_pct_tier1.toFixed(1)}%`,
      sub: raw.worst_eve_scenario ? `of Tier 1 · ${SCENARIO_LABEL[raw.worst_eve_scenario]}` : 'no loss in any scenario',
      tone: standingTone(st(lim('EVE'))),
    },
    { tool: 'funding', label: 'Ten largest depositors', value: `${raw.top10_pct.toFixed(1)}%`, sub: `wholesale funding ${raw.wholesale_pct.toFixed(1)}%`, tone: standingTone(st(lim('Ten'))) },
  ];
  return cells;
}

export interface LimitView {
  metric: string;
  value: string;
  limit: string;
  gaugeValue: number;
  gaugeLimit: number;
  gaugeTrigger: number | null;
  gaugeMax: number;
  floor: boolean;
  standing: Standing;
}

export function limitRows(raw: AlmOverview): LimitView[] {
  return raw.limits.map((l) => {
    const v = l.value ?? 0;
    const max = Math.max(v, l.limit, l.trigger ?? 0) * 1.2;
    return {
      metric: l.metric,
      value: l.value === null ? '—' : `${l.value.toFixed(1)}${l.unit}`,
      limit: `${l.floor ? '≥ ' : '≤ '}${l.limit.toFixed(0)}${l.unit}`,
      gaugeValue: v,
      gaugeLimit: l.limit,
      gaugeTrigger: l.trigger,
      gaugeMax: Math.max(max, 1),
      floor: l.floor,
      standing: flags(l.breach, l.warn),
    };
  });
}
