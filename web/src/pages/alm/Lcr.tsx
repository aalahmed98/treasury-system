import { useMemo } from 'react';
import type { LcrResult } from '../../api/types';
import { Card, Gauge } from '../../components/ui';
import { useAlm, useTool } from '../../alm/state';
import { AssumptionsHeader, Group, Headline, KV, Pct, ResultsHeader, Row, SimpleTable, ToolLayout } from '../../alm/tool';
import { lcrView, whenLine } from '../../alm/views';
import { nice } from '../../lib/fmt';

function Assumptions() {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const l = working.lcr;
  const b = bank.lcr;
  return (
    <>
      <AssumptionsHeader />
      <Group title="Thresholds" help="The ratio must stay above the regulatory minimum. The internal minimum and the early warning are the bank's own triggers, set in Setup.">
        <Row label="Regulatory minimum">
          <Pct value={l.minimum} base={b.minimum} max={500} onChange={(v) => edit((p) => void (p.lcr.minimum = v))} />
        </Row>
        <Row label="Internal minimum">
          <Pct value={l.internal_minimum} base={b.internal_minimum} max={500} onChange={(v) => edit((p) => void (p.lcr.internal_minimum = v))} />
        </Row>
        <Row label="Early warning">
          <Pct value={l.early_warning} base={b.early_warning} max={500} onChange={(v) => edit((p) => void (p.lcr.early_warning = v))} />
        </Row>
      </Group>
      <Group title="Liquid assets" help="Haircuts on each level of liquid assets, and the caps on how much level 2 may count towards the stock.">
        {['L1', 'L2A', 'L2B'].map((level) => (
          <Row key={level} label={`${level} haircut`} cover={`lcr.haircuts.${level}`}>
            <Pct value={l.haircuts[level] ?? 0} base={b.haircuts[level] ?? 0} onChange={(v) => edit((p) => void (p.lcr.haircuts[level] = v))} />
          </Row>
        ))}
        <Row label="Level 2 cap" cover="lcr.cap_l2_total_pct">
          <Pct value={l.cap_l2_total_pct} base={b.cap_l2_total_pct} max={99} onChange={(v) => edit((p) => void (p.lcr.cap_l2_total_pct = v))} />
        </Row>
        <Row label="Level 2B cap" cover="lcr.cap_l2b_pct">
          <Pct value={l.cap_l2b_pct} base={b.cap_l2b_pct} max={99} onChange={(v) => edit((p) => void (p.lcr.cap_l2b_pct = v))} />
        </Row>
        <Row label="Inflow cap" cover="lcr.inflow_cap_pct">
          <Pct value={l.inflow_cap_pct} base={b.inflow_cap_pct} onChange={(v) => edit((p) => void (p.lcr.inflow_cap_pct = v))} />
        </Row>
      </Group>
      <Group title="Run-off rates" help="The share of each liability class assumed to leave within 30 days of stress." open={false} count={Object.keys(l.outflow_rates).length}>
        {Object.entries(l.outflow_rates).map(([k, v]) => (
          <Row key={k} label={nice(k)} cover={`lcr.outflow_rates.${k}`}>
            <Pct value={v} base={b.outflow_rates[k]} onChange={(x) => edit((p) => void (p.lcr.outflow_rates[k] = x))} />
          </Row>
        ))}
      </Group>
      <Group title="Inflow rates" help="The share of each receivable class assumed to arrive within 30 days, counted up to the inflow cap." open={false} count={Object.keys(l.inflow_rates).length}>
        {Object.entries(l.inflow_rates).map(([k, v]) => (
          <Row key={k} label={nice(k)} cover={`lcr.inflow_rates.${k}`}>
            <Pct value={v} base={b.inflow_rates[k]} onChange={(x) => edit((p) => void (p.lcr.inflow_rates[k] = x))} />
          </Row>
        ))}
      </Group>
      <p className="t-small prose" style={{ marginTop: 10 }}>
        Basel III defaults until the CBB LM-11 calibration and the bank's limits are confirmed. Edits here are a what-if; Setup saves a new version.
      </p>
    </>
  );
}

export function Lcr() {
  const q = useTool<LcrResult>('lcr');
  const v = useMemo(() => (q.data ? lcrView(q.data) : null), [q.data]);
  const raw = q.data;
  return (
    <ToolLayout assumptions={<Assumptions />} ready={!!v} busy={q.isFetching && !!v}>
      <ResultsHeader title="Liquidity coverage ratio (LCR)" loading={q.isFetching} error={q.error?.message} onRetry={() => void q.refetch()} />
      {v && raw && (
        <>
          <Headline value={v.headline} standing={v.standing} line1={whenLine(raw.as_of, v.basis, raw.reporting_ccy)} line2={`minimum ${raw.minimum.toFixed(0)}% · internal ${raw.internal_minimum.toFixed(0)}% · early warning ${raw.early_warning.toFixed(0)}%`} testId="headline" />
          <Gauge value={raw.total.ratio ?? 0} limit={Math.max(raw.internal_minimum, raw.minimum)} trigger={raw.early_warning} max={Math.max(raw.total.ratio ?? 0, raw.early_warning) * 1.15} floor width={320} />
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="Stock of liquid assets" width={320}>
              <KV rows={v.hqlaRows} boldLast />
            </Card>
            <Card title="Net cash outflows (30 days)" width={320}>
              <KV rows={v.totals} boldLast />
            </Card>
            <Card title="By currency" width={320}>
              <SimpleTable headers={['Currency', 'Stock', 'Net outflows', 'LCR']} rows={v.ccyRows} />
            </Card>
          </div>
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="Outflows" width={480}>
              <SimpleTable headers={['Class', 'Amount', 'Run-off', 'Outflow']} rows={v.outflowRows} />
            </Card>
            <Card title="Inflows" width={480}>
              <SimpleTable headers={['Class', 'Due in 30 days', 'Rate', 'Inflow']} rows={v.inflowRows} />
            </Card>
          </div>
        </>
      )}
    </ToolLayout>
  );
}
