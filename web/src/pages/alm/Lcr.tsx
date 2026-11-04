import { useMemo } from 'react';
import type { LcrResult } from '../../api/types';
import { Card, Gauge } from '../../components/ui';
import { useAlm, useTool } from '../../alm/state';
import { AssumptionsHeader, Group, Headline, KV, Pct, ResultsHeader, Row, SimpleTable, ToolLayout } from '../../alm/tool';
import { lcrView, whenLine } from '../../alm/views';
import { nice } from '../../lib/fmt';

function Assumptions() {
  const { working, edit } = useAlm();
  if (!working) return null;
  const l = working.lcr;
  return (
    <>
      <AssumptionsHeader />
      <Group title="Thresholds" />
      <Row label="Regulatory minimum">
        <Pct value={l.minimum} max={500} onChange={(v) => edit((p) => void (p.lcr.minimum = v))} />
      </Row>
      <Row label="Internal minimum">
        <Pct value={l.internal_minimum} max={500} onChange={(v) => edit((p) => void (p.lcr.internal_minimum = v))} />
      </Row>
      <Row label="Early warning">
        <Pct value={l.early_warning} max={500} onChange={(v) => edit((p) => void (p.lcr.early_warning = v))} />
      </Row>
      <Group title="Liquid assets" />
      {['L1', 'L2A', 'L2B'].map((level) => (
        <Row key={level} label={`${level} haircut`}>
          <Pct value={l.haircuts[level] ?? 0} onChange={(v) => edit((p) => void (p.lcr.haircuts[level] = v))} />
        </Row>
      ))}
      <Row label="Level 2 cap">
        <Pct value={l.cap_l2_total_pct} max={99} onChange={(v) => edit((p) => void (p.lcr.cap_l2_total_pct = v))} />
      </Row>
      <Row label="Level 2B cap">
        <Pct value={l.cap_l2b_pct} max={99} onChange={(v) => edit((p) => void (p.lcr.cap_l2b_pct = v))} />
      </Row>
      <Row label="Inflow cap">
        <Pct value={l.inflow_cap_pct} onChange={(v) => edit((p) => void (p.lcr.inflow_cap_pct = v))} />
      </Row>
      <Group title="Run-off rates" />
      {Object.entries(l.outflow_rates).map(([k, v]) => (
        <Row key={k} label={nice(k)}>
          <Pct value={v} onChange={(x) => edit((p) => void (p.lcr.outflow_rates[k] = x))} />
        </Row>
      ))}
      <Group title="Inflow rates" />
      {Object.entries(l.inflow_rates).map(([k, v]) => (
        <Row key={k} label={nice(k)}>
          <Pct value={v} onChange={(x) => edit((p) => void (p.lcr.inflow_rates[k] = x))} />
        </Row>
      ))}
      <p className="t-small" style={{ marginTop: 10 }}>
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
    <ToolLayout assumptions={<Assumptions />} ready={!!v}>
      <ResultsHeader title="Liquidity coverage ratio" loading={q.isFetching} error={q.error?.message} />
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
