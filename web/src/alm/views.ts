// Per-tool views: everything a page draws, formatted once from the server's result. Ported from
// crates/client/src/alm_state.rs so the two clients print the same.
import type { AttributionResult, FundingResult, GapResult, LcrLine, LcrResult, NiiResult, NsfrLine, NsfrResult, RateScenario, StressResult } from '../api/types';
import { fmtDateLong, fmtMonth, fmtMoneyShort, fmtPct, nice, signed } from '../lib/fmt';
import { SCENARIO_LABEL, pctOrDash, standing, type Standing } from '../lib/alm';

export const basisLabel = (v: number | null) => (v !== null ? `bank parameters v${v}` : 'unsaved what-if');
const minus = (v: number) => (v > 0.5 ? `-${fmtMoneyShort(v)}` : '0');

export const SHORT_LABEL: Record<RateScenario, string> = { BASE: 'Base', PARALLEL_UP: 'Par +', PARALLEL_DOWN: 'Par −', STEEPENER: 'Steep', FLATTENER: 'Flat', SHORT_UP: 'Short +', SHORT_DOWN: 'Short −', CUSTOM: 'Custom' };

export function lcrView(raw: LcrResult) {
  const t = raw.total;
  const lines = (rows: LcrLine[]) => rows.map((l) => [nice(l.class), fmtMoneyShort(l.amount), fmtPct(l.rate, 0), fmtMoneyShort(l.weighted)]);
  return {
    headline: pctOrDash(t.ratio),
    standing: standing(t.ratio, raw.minimum, raw.internal_minimum, raw.early_warning),
    basis: basisLabel(raw.params_version),
    hqlaRows: [
      ['Level 1', fmtMoneyShort(t.hqla_l1)],
      ['Level 2A after haircut', fmtMoneyShort(t.hqla_l2a)],
      ['Level 2B after haircut', fmtMoneyShort(t.hqla_l2b)],
      ['Level 2B cap adjustment', minus(t.adjustment_l2b_cap)],
      ['Level 2 cap adjustment', minus(t.adjustment_l2_cap)],
      ['Stock of liquid assets', fmtMoneyShort(t.stock)],
    ] as [string, string][],
    totals: [
      ['Total outflows', fmtMoneyShort(t.total_outflows)],
      ['Total inflows', fmtMoneyShort(t.total_inflows)],
      ['Inflows after cap', fmtMoneyShort(t.capped_inflows)],
      ['Net cash outflows', fmtMoneyShort(t.net_outflows)],
    ] as [string, string][],
    ccyRows: raw.by_ccy.map((c) => [c.ccy, fmtMoneyShort(c.stock), fmtMoneyShort(c.net_outflows), pctOrDash(c.ratio)]),
    outflowRows: lines(t.outflows),
    inflowRows: lines(t.inflows),
  };
}

export function nsfrView(raw: NsfrResult) {
  const rows = (l: NsfrLine[]) => l.map((x) => [`${nice(x.class)} · ${x.bucket}`, fmtMoneyShort(x.amount), fmtPct(x.factor, 0), fmtMoneyShort(x.weighted)]);
  return {
    headline: pctOrDash(raw.ratio),
    standing: standing(raw.ratio, raw.minimum, raw.internal_minimum, raw.internal_minimum),
    basis: basisLabel(raw.params_version),
    asfRows: rows(raw.asf_lines),
    rsfRows: rows(raw.rsf_lines),
    totals: [
      ['Available stable funding', fmtMoneyShort(raw.asf)],
      ['Required stable funding', fmtMoneyShort(raw.rsf)],
    ] as [string, string][],
  };
}

export function gapView(raw: GapResult) {
  return {
    basis: basisLabel(raw.params_version),
    rows: raw.buckets.map((b) => [b.label, fmtMoneyShort(b.assets), fmtMoneyShort(b.liabilities), signed(b.gap), signed(b.cumulative), b.limit_pct !== null ? `${b.cumulative_pct_assets.toFixed(1)}% of ${b.limit_pct.toFixed(0)}%` : `${b.cumulative_pct_assets.toFixed(1)}%`]),
    gapBars: raw.buckets.map((b) => [b.label, b.gap] as [string, number]),
    cumulativeBars: raw.buckets.map((b) => [b.label, b.cumulative] as [string, number]),
    breaches: raw.buckets.filter((b) => b.breach).map((b) => `${b.label}: ${b.cumulative_pct_assets.toFixed(1)}% against ${(b.limit_pct ?? 0).toFixed(0)}%`),
    ccyRows: raw.by_ccy.map((c) => {
      const y1 = c.buckets.find((b) => b.label === '1Y');
      return [c.ccy, y1 ? signed(y1.cumulative) : '', y1 ? `${y1.cumulative_pct_assets.toFixed(1)}%` : ''];
    }),
    y1: raw.buckets.find((b) => b.label === '1Y') ?? null,
  };
}

export function niiView(raw: NiiResult) {
  return {
    headline: fmtMoneyShort(raw.nii),
    nim: raw.nim === null ? '—' : `${raw.nim.toFixed(2)}%`,
    basis: `${SCENARIO_LABEL[raw.scenario]} · ${basisLabel(raw.params_version)}`,
    delta: raw.base_nii === null ? null : `${signed(raw.nii - raw.base_nii)} against the base (${fmtMoneyShort(raw.base_nii)})`,
    monthBars: raw.months.map((m) => [fmtMonth(m.period), m.nii] as [string, number]),
    rows: raw.months.map((m) => [fmtMonth(m.period, true), fmtMoneyShort(m.income), fmtMoneyShort(m.expense), fmtMoneyShort(m.nii)]),
    byBook: raw.by_book.map((b) => [nice(b.key), signed(b.nii)]),
    byCcy: raw.by_ccy.map((b) => [b.key, signed(b.nii)]),
  };
}

export function stressView(raw: StressResult) {
  const l = raw.liquidity;
  const breaches = raw.rate_scenarios.filter((s) => s.nii_breach || s.eve_breach).map((s) => s.label);
  if (l.survival_days !== null) breaches.push(`liquid assets exhausted on day ${l.survival_days} of the liquidity stress`);
  const worst = raw.worst_eve ? raw.rate_scenarios.find((r) => r.scenario === raw.worst_eve) ?? null : null;
  return {
    basis: basisLabel(raw.params_version),
    worst,
    rows: raw.rate_scenarios.map((s) => [
      s.label,
      signed(s.nii_delta),
      s.nii_delta_pct === null ? '—' : `${s.nii_delta_pct >= 0 ? '+' : ''}${s.nii_delta_pct.toFixed(1)}%`,
      signed(s.eve_delta),
      fmtMoneyShort(s.eve_loss),
      s.eve_loss_pct_tier1 === null ? '—' : `${s.eve_loss_pct_tier1.toFixed(1)}%`,
      s.nii_breach && s.eve_breach ? 'NII and EVE limits' : s.nii_breach ? 'NII limit' : s.eve_breach ? 'EVE limit' : '',
    ]),
    niiBars: raw.rate_scenarios.map((s) => [SHORT_LABEL[s.scenario], s.nii_delta] as [string, number]),
    eveBars: raw.rate_scenarios.map((s) => [SHORT_LABEL[s.scenario], s.eve_delta] as [string, number]),
    liquidityRows: [
      ['LCR today', pctOrDash(l.lcr_base)],
      ['LCR under stress', pctOrDash(l.lcr_stressed)],
      ['Stock after extra haircut', fmtMoneyShort(l.stock_stressed)],
      ['Stressed net outflows', fmtMoneyShort(l.net_outflows_stressed)],
      ['Survival', l.survival_days !== null ? `${l.survival_days} days` : 'beyond 90 days'],
    ] as [string, string][],
    survival: l.horizon.map((p) => [p.day, p.cumulative_net_outflow] as [number, number]),
    stock: l.stock_stressed,
    fxRows: raw.fx.map((f) => [f.ccy, `-${f.shock_pct.toFixed(0)}%`, signed(f.exposure), signed(f.pnl)]),
    breaches,
    standing: (breaches.length ? 'breach' : 'comfortable') as Standing,
  };
}

export function attributionView(raw: AttributionResult) {
  const n = raw.steps.length;
  return {
    headline: signed(raw.delta),
    standing: (raw.delta >= 0 ? 'comfortable' : 'watch') as Standing,
    subtitle: `${fmtMonth(raw.period_b, true)} against ${raw.compare_to === 'PRIOR' ? 'the prior month,' : 'the budget for'} ${fmtMonth(raw.period_a, true)} · in ${raw.reporting_ccy}`,
    range: `from ${fmtMoneyShort(raw.nii_a)} to ${fmtMoneyShort(raw.nii_b)}`,
    steps: raw.steps.map((s, i) => [s.label, s.value, i === 0 || i + 1 === n] as [string, number, boolean]),
    effectRows: [
      ['Days', signed(raw.days_effect)],
      ['Pure volume', signed(raw.pure_volume_effect)],
      ['Mix', signed(raw.mix_effect)],
      ['Rate', signed(raw.rate_effect)],
      ['Residual', signed(raw.residual)],
      ['Change', signed(raw.delta)],
    ] as [string, string][],
    rows: raw.lines.map((l) => [nice(l.book), l.side.toLowerCase(), l.ccy, signed(l.nii_a), signed(l.nii_b), `${l.rate_a.toFixed(2)}%`, `${l.rate_b.toFixed(2)}%`, signed(l.nii_b - l.nii_a)]),
  };
}

export function fundingView(raw: FundingResult) {
  const short = (n: string) => n.replace('Government entity', 'Govt').replace('Large corporate', 'Corp').replace('Financial institution', 'FI').replace('High net worth group', 'HNW');
  return {
    basis: basisLabel(raw.params_version),
    standing: (raw.breaches.length ? 'breach' : 'comfortable') as Standing,
    headlineRows: [
      ['Largest depositor', `${raw.top1_pct.toFixed(1)}% (limit ${raw.top1_limit_pct.toFixed(0)}%)`],
      ['Ten largest depositors', `${raw.top10_pct.toFixed(1)}% (limit ${raw.top10_limit_pct.toFixed(0)}%)`],
      ['Wholesale funding', `${raw.wholesale_pct.toFixed(1)}% (limit ${raw.wholesale_limit_pct.toFixed(0)}%)`],
      ['Deposits', fmtMoneyShort(raw.total_deposits)],
      ['Total funding', fmtMoneyShort(raw.total_funding)],
    ] as [string, string][],
    groupRows: raw.groups.map((g) => [g.name, nice(g.segment), fmtMoneyShort(g.balance), `${g.share_pct.toFixed(2)}%`]),
    groupBars: raw.groups.slice(0, 10).map((g) => [short(g.name), g.share_pct] as [string, number]),
    segmentRows: raw.by_segment.map((s) => [nice(s.key), fmtMoneyShort(s.nii)]),
  };
}

export const whenLine = (asOf: string, basis: string, ccy: string) => `${fmtDateLong(asOf)} · ${basis} · in ${ccy}`;
