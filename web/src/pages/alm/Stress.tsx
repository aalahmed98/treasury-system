import { useMemo } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { endpoints } from '../../api/client';
import type { CompareRow, StressResult } from '../../api/types';
import { Card } from '../../components/ui';
import { Bars, LineChart } from '../../components/charts';
import { useAlm, useTool, useUnhedged } from '../../alm/state';
import { AssumptionsHeader, Group, Headline, HedgeEffect, KV, Num, Pct, ResultsHeader, Row, SimpleTable, ToolLayout } from '../../alm/tool';
import { stressView } from '../../alm/views';
import { fmtDateLong, fmtMoneyShort, signed } from '../../lib/fmt';
import { ShockGrid } from './Nii';

function Assumptions() {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const s = working.stress;
  const b = bank.stress;
  return (
    <>
      <AssumptionsHeader />
      <RateLimits />
      <ShockSizes />
      <Group title="Liquidity stress" help="A harsher run-off, a deeper haircut on liquid assets and fewer inflows than the LCR assumes.">
        <Row label="Run-off multiplier">
          <Num value={s.liquidity.outflow_multiplier} base={b.liquidity.outflow_multiplier} min={1} max={5} step={0.05} decimals={2} onChange={(v) => edit((p) => void (p.stress.liquidity.outflow_multiplier = v))} />
        </Row>
        <Row label="Extra HQLA haircut">
          <Pct value={s.liquidity.extra_hqla_haircut_pct} base={b.liquidity.extra_hqla_haircut_pct} onChange={(v) => edit((p) => void (p.stress.liquidity.extra_hqla_haircut_pct = v))} />
        </Row>
        <Row label="Inflow multiplier">
          <Num value={s.liquidity.inflow_multiplier} base={b.liquidity.inflow_multiplier} min={0} max={1} step={0.05} decimals={2} onChange={(v) => edit((p) => void (p.stress.liquidity.inflow_multiplier = v))} />
        </Row>
      </Group>
      <Group title="FX shocks (% fall against BHD)" help="How far each currency is assumed to fall, applied to the net open position." open={false} count={Object.keys(s.fx_shock_pct).length}>
        {Object.entries(s.fx_shock_pct).map(([ccy, v]) => (
          <Row key={ccy} label={ccy}>
            <Pct value={v} base={b.fx_shock_pct[ccy]} onChange={(x) => edit((p) => void (p.stress.fx_shock_pct[ccy] = x))} />
          </Row>
        ))}
      </Group>
    </>
  );
}

const pctSigned = (v: number | null) => (v === null ? '—' : `${v >= 0 ? '+' : ''}${v.toFixed(1)}%`);

/** The current simulation and every saved scenario the user can see, each run on this snapshot:
 * the rate effect under the scenario's own assumptions and hedges, as the table above gives it. */
function SavedScenarios() {
  const { request, simulating, asOf, loaded } = useAlm();
  const req = request();
  const q = useQuery({
    queryKey: ['alm', 'compare', simulating ? req : asOf],
    queryFn: () => endpoints.almCompare({ as_of: asOf, current: simulating ? req : null }),
    placeholderData: keepPreviousData,
    staleTime: 10_000,
  });
  const rows = q.data ?? [];
  const cells = (r: CompareRow): string[] => {
    const who = r.id === null ? '' : (r.author ?? '');
    if (!r.row) return [r.name, who, r.error ?? 'could not be run', '', '', '', '', ''];
    const s = r.row;
    return [
      r.name,
      who,
      r.summary,
      `${signed(s.nii_delta)} (${pctSigned(s.nii_delta_pct)})`,
      signed(s.eve_delta),
      s.eve_loss_pct_tier1 === null ? '—' : `${s.eve_loss_pct_tier1.toFixed(1)}%`,
      r.gap_nii_effect === null ? '—' : signed(r.gap_nii_effect),
      s.nii_breach && s.eve_breach ? 'outside NII and EVE limits' : s.nii_breach ? 'outside NII limit' : s.eve_breach ? 'outside EVE limit' : 'within limits',
    ];
  };
  const mark = loaded ? rows.findIndex((r) => r.id === loaded.id) : -1;
  return (
    <Card title="Saved scenarios, side by side" width={1000} style={{ marginTop: 10 }}>
      {rows.length > 0 ? (
        <SimpleTable headers={['Scenario', 'By', 'What it is', 'ΔNII 12m', 'ΔEVE', '% Tier 1', 'Gap NII est.', 'Standing']} rows={rows.map(cells)} mark={mark >= 0 ? mark : undefined} text={3} testId="saved-scenarios" />
      ) : (
        <div className="t-muted" data-testid="saved-scenarios-empty">
          {q.isLoading ? 'loading…' : 'No saved scenarios yet. Choose a shock, try a hedge or change an assumption, then use Scenarios on the bar above to keep it.'}
        </div>
      )}
      {q.error && <div className="t-bad t-small">{q.error.message}</div>}
      <div className="t-small" style={{ marginTop: 4 }}>
        Each row is the change the scenario's shock makes under its own assumptions and hedges, on this snapshot. Open one from Scenarios on the bar to see it on every tab.
      </div>
    </Card>
  );
}

/** The floor rates cannot fall below after a shock: shared with the EVE tab. */
export function ShockSizes() {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const s = working.stress;
  const b = bank.stress;
  return (
    <Group title="Rate shocks (bp)" help="The size of each Basel shock per currency, and the floor rates cannot fall below after a shock." open={false} count={Object.keys(s.shocks).length + 2}>
      <ShockGrid />
      <Row label="Post-shock floor (bp)">
        <Num value={s.floor_bp} base={b.floor_bp} min={-500} max={0} step={5} onChange={(v) => edit((p) => void (p.stress.floor_bp = v))} />
      </Row>
      <Row label="Floor rises per year (bp)">
        <Num value={s.floor_slope_bp_per_year} base={b.floor_slope_bp_per_year} min={0} max={50} step={1} onChange={(v) => edit((p) => void (p.stress.floor_slope_bp_per_year = v))} />
      </Row>
    </Group>
  );
}

/** The EVE and NII limits and the capital they are measured against: shared with the EVE tab. */
export function RateLimits({ nii = true }: { nii?: boolean }) {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  return (
    <Group title="Limits" help="How much economic value (EVE) and income (NII) the bank is prepared to lose under the worst scenario.">
      <Row label="EVE loss (% of Tier 1)">
        <Pct value={working.gap.eve_limit_pct_tier1} base={bank.gap.eve_limit_pct_tier1} onChange={(v) => edit((p) => void (p.gap.eve_limit_pct_tier1 = v))} />
      </Row>
      {nii && (
        <Row label="NII sensitivity (%)">
          <Pct value={working.gap.nii_sensitivity_limit_pct} base={bank.gap.nii_sensitivity_limit_pct} onChange={(v) => edit((p) => void (p.gap.nii_sensitivity_limit_pct = v))} />
        </Row>
      )}
      <Row label="Tier 1 capital">
        <Num value={working.tier1_capital} base={bank.tier1_capital} min={1} max={1e12} step={1e6} width={120} onChange={(v) => edit((p) => void (p.tier1_capital = v))} />
      </Row>
    </Group>
  );
}

export function Stress() {
  const { scenario } = useAlm();
  const q = useTool<StressResult>('stress');
  const v = useMemo(() => (q.data ? stressView(q.data) : null), [q.data]);
  const raw = q.data;
  const mark = raw ? raw.rate_scenarios.findIndex((s) => s.scenario === scenario) : -1;
  const plain = useUnhedged<StressResult>('stress');
  const plainWorst = plain ? Math.max(0, ...plain.rate_scenarios.map((s) => s.eve_loss_pct_tier1 ?? 0)) : null;
  return (
    <ToolLayout assumptions={<Assumptions />} ready={!!v} busy={q.isFetching && !!v}>
      <ResultsHeader title="Stress testing" loading={q.isFetching} error={q.error?.message} onRetry={() => void q.refetch()} rates />
      {v && raw && (
        <>
          <Headline
            value={v.worst?.eve_loss_pct_tier1 != null ? `${v.worst.eve_loss_pct_tier1.toFixed(1)}%` : '0%'}
            standing={v.standing}
            line1={`worst EVE loss as a share of Tier 1 (${fmtMoneyShort(raw.tier1_capital)})${v.worst ? ` · ${v.worst.label}` : ''}`}
            line2={`${fmtDateLong(raw.as_of)} · ${v.basis} · base NII ${fmtMoneyShort(raw.base_nii)} · base EVE ${fmtMoneyShort(raw.base_eve)}`}
            testId="headline"
          />
          {v.breaches.length > 0 && <div className="t-bad" style={{ marginBottom: 8 }}>outside a limit: {v.breaches.join('; ')}</div>}
          {plainWorst !== null && (
            <HedgeEffect>
              Without the trial hedges the worst EVE loss would be {plainWorst.toFixed(1)}% of Tier 1; with them it is <span className="t-medium">{(v.worst?.eve_loss_pct_tier1 ?? 0).toFixed(1)}%</span>. The rate scenarios below include the hedges; the liquidity and FX stresses are not affected by them.
            </HedgeEffect>
          )}
          <Card title="Rate scenarios" width={900}>
            <SimpleTable headers={['Scenario', 'ΔNII 12m', 'ΔNII %', 'ΔEVE', 'EVE loss', '% Tier 1', 'Standing']} rows={v.rows} mark={mark >= 0 ? mark : undefined} testId="rate-scenarios" />
            <div className="t-small" style={{ marginTop: 4 }}>
              {mark >= 0 ? 'The highlighted row is the rate shock chosen above; the Gap, NII and EVE tabs show it in full.' : 'Every Basel scenario side by side. Choose a rate shock above to pick one out, or a custom one to add it as a row.'}
            </div>
          </Card>
          <SavedScenarios />
          {raw.uniform_200.length > 0 && (
            <Card title="Annual-report test: every curve ±200 bp" width={900} style={{ marginTop: 10 }}>
              <SimpleTable headers={['Scenario', 'ΔNII 12m', 'ΔNII %', 'ΔEVE', 'EVE loss', '% Tier 1']} rows={raw.uniform_200.map((s) => [s.label, signed(s.nii_delta), s.nii_delta_pct === null ? '—' : `${s.nii_delta_pct >= 0 ? '+' : ''}${s.nii_delta_pct.toFixed(1)}%`, signed(s.eve_delta), fmtMoneyShort(s.eve_loss), s.eve_loss_pct_tier1 === null ? '—' : `${s.eve_loss_pct_tier1.toFixed(1)}%`])} testId="uniform-200" />
              <div className="t-small" style={{ marginTop: 4 }}>
                The supplementary test the bank discloses in its annual report, alongside the six Basel shapes above.
              </div>
            </Card>
          )}
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="ΔNII by scenario" width={440}>
              <Bars data={v.niiBars} color="var(--s1)" height={160} fmt={fmtMoneyShort} />
            </Card>
            <Card title="ΔEVE by scenario" width={440}>
              <Bars data={v.eveBars} color="var(--s2)" height={160} fmt={fmtMoneyShort} />
            </Card>
          </div>
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="Liquidity stress" width={320}>
              <KV rows={v.liquidityRows} />
            </Card>
            <Card title="Cumulative stressed net outflow against the stock (90 days)" width={560}>
              <LineChart
                series={[
                  { name: 'net outflow', points: v.survival },
                  {
                    name: 'stock',
                    points: [
                      [1, v.stock],
                      [90, v.stock],
                    ],
                  },
                ]}
                height={170}
                yFmt={fmtMoneyShort}
                xFmt={(x) => `d${x.toFixed(0)}`}
              />
            </Card>
          </div>
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="FX shocks" width={440}>
              <SimpleTable headers={['Currency', 'Shock', 'Net exposure', 'Revaluation']} rows={v.fxRows} />
            </Card>
          </div>
        </>
      )}
    </ToolLayout>
  );
}
