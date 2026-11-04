import { useMemo } from 'react';
import type { StressResult } from '../../api/types';
import { Card } from '../../components/ui';
import { Bars, LineChart } from '../../components/charts';
import { useAlm, useTool } from '../../alm/state';
import { AssumptionsHeader, Bp, Group, Headline, KV, Num, Pct, ResultsHeader, Row, SimpleTable, ToolLayout } from '../../alm/tool';
import { stressView } from '../../alm/views';
import { fmtDateLong, fmtMoneyShort } from '../../lib/fmt';
import { ShockGrid } from './Nii';

function Assumptions() {
  const { working, edit, bank, customBp, setCustomBp } = useAlm();
  if (!working || !bank) return null;
  const s = working.stress;
  const b = bank.stress;
  return (
    <>
      <AssumptionsHeader />
      <Group title="Limits" help="How much economic value (EVE) and income (NII) the bank is prepared to lose under the worst scenario.">
        <Row label="EVE loss (% of Tier 1)">
          <Pct value={working.gap.eve_limit_pct_tier1} base={bank.gap.eve_limit_pct_tier1} onChange={(v) => edit((p) => void (p.gap.eve_limit_pct_tier1 = v))} />
        </Row>
        <Row label="NII sensitivity (%)">
          <Pct value={working.gap.nii_sensitivity_limit_pct} base={bank.gap.nii_sensitivity_limit_pct} onChange={(v) => edit((p) => void (p.gap.nii_sensitivity_limit_pct = v))} />
        </Row>
        <Row label="Tier 1 capital">
          <Num value={working.tier1_capital} base={bank.tier1_capital} min={1} max={1e12} step={1e6} width={120} onChange={(v) => edit((p) => void (p.tier1_capital = v))} />
        </Row>
        <Row label="Custom parallel shock">
          <Bp value={customBp} onChange={setCustomBp} />
        </Row>
      </Group>
      <Group title="Rate shocks (bp)" help="The size of each Basel shock per currency, and the floor rates cannot fall below after a shock." open={false} count={Object.keys(s.shocks).length + 2}>
        <ShockGrid />
        <Row label="Post-shock floor (bp)">
          <Num value={s.floor_bp} base={b.floor_bp} min={-500} max={0} step={5} onChange={(v) => edit((p) => void (p.stress.floor_bp = v))} />
        </Row>
        <Row label="Floor rises per year (bp)">
          <Num value={s.floor_slope_bp_per_year} base={b.floor_slope_bp_per_year} min={0} max={50} step={1} onChange={(v) => edit((p) => void (p.stress.floor_slope_bp_per_year = v))} />
        </Row>
      </Group>
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

export function Stress() {
  const q = useTool<StressResult>('stress');
  const v = useMemo(() => (q.data ? stressView(q.data) : null), [q.data]);
  const raw = q.data;
  return (
    <ToolLayout assumptions={<Assumptions />} ready={!!v}>
      <ResultsHeader title="Stress testing and EVE" loading={q.isFetching} error={q.error?.message} />
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
          <Card title="Rate scenarios" width={900}>
            <SimpleTable headers={['Scenario', 'ΔNII 12m', 'ΔNII %', 'ΔEVE', 'EVE loss', '% Tier 1', 'Standing']} rows={v.rows} />
          </Card>
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
