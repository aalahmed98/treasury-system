import { useMemo } from 'react';
import type { Factors3, NsfrResult } from '../../api/types';
import { Card, Gauge } from '../../components/ui';
import { useAlm, useTool } from '../../alm/state';
import { AssumptionsHeader, Group, Headline, KV, Num, Pct, ResultsHeader, Row, SimpleTable, ToolLayout } from '../../alm/tool';
import { nsfrView, whenLine } from '../../alm/views';
import { nice } from '../../lib/fmt';

function FactorGrid({ map, base, onChange }: { map: Record<string, Factors3>; base: Record<string, Factors3>; onChange: (k: string, i: number, v: number) => void }) {
  return (
    <div className="factor-grid">
      <span />
      {['< 6M', '6M-1Y', '≥ 1Y'].map((h) => (
        <span key={h} className="h">
          {h}
        </span>
      ))}
      {Object.entries(map).map(([k, f]) => (
        <div key={k} style={{ display: 'contents' }}>
          <span className="k">{nice(k)}</span>
          {f.map((v, i) => (
            <Num key={i} value={v} base={base[k]?.[i]} min={0} max={100} step={0.5} decimals={1} width={64} onChange={(x) => onChange(k, i, x)} />
          ))}
        </div>
      ))}
    </div>
  );
}

function Assumptions() {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const n = working.nsfr;
  const b = bank.nsfr;
  return (
    <>
      <AssumptionsHeader />
      <Group title="Thresholds" help="Stable funding available must cover the stable funding required, so the ratio stays above the minimum.">
        <Row label="Regulatory minimum">
          <Pct value={n.minimum} base={b.minimum} max={500} onChange={(v) => edit((p) => void (p.nsfr.minimum = v))} />
        </Row>
        <Row label="Internal minimum">
          <Pct value={n.internal_minimum} base={b.internal_minimum} max={500} onChange={(v) => edit((p) => void (p.nsfr.internal_minimum = v))} />
        </Row>
      </Group>
      <Group title="Available stable funding factors" help="How much of each liability counts as stable funding, by the time left to maturity." open={false} count={Object.keys(n.asf).length}>
        <FactorGrid map={n.asf} base={b.asf} onChange={(k, i, v) => edit((p) => void (p.nsfr.asf[k][i] = v))} />
      </Group>
      <Group title="Required stable funding factors" help="How much stable funding each asset needs, by the time left to maturity." open={false} count={Object.keys(n.rsf).length}>
        <FactorGrid map={n.rsf} base={b.rsf} onChange={(k, i, v) => edit((p) => void (p.nsfr.rsf[k][i] = v))} />
      </Group>
      <Group title="Other required funding" help="Liquid assets, undrawn commitments and encumbered assets carry their own factors.">
        {['L1', 'L2A', 'L2B'].map((level) => (
          <Row key={level} label={`HQLA ${level}`}>
            <Pct value={n.hqla_rsf[level] ?? 0} base={b.hqla_rsf[level] ?? 0} onChange={(v) => edit((p) => void (p.nsfr.hqla_rsf[level] = v))} />
          </Row>
        ))}
        <Row label="Off-balance commitments">
          <Pct value={n.off_balance_rsf_pct} base={b.off_balance_rsf_pct} onChange={(v) => edit((p) => void (p.nsfr.off_balance_rsf_pct = v))} />
        </Row>
        <Row label="Encumbered a year or more">
          <Pct value={n.encumbered_rsf_pct} base={b.encumbered_rsf_pct} onChange={(v) => edit((p) => void (p.nsfr.encumbered_rsf_pct = v))} />
        </Row>
      </Group>
    </>
  );
}

export function Nsfr() {
  const q = useTool<NsfrResult>('nsfr');
  const v = useMemo(() => (q.data ? nsfrView(q.data) : null), [q.data]);
  const raw = q.data;
  return (
    <ToolLayout assumptions={<Assumptions />} ready={!!v}>
      <ResultsHeader title="Net stable funding ratio (NSFR)" loading={q.isFetching} error={q.error?.message} />
      {v && raw && (
        <>
          <Headline value={v.headline} standing={v.standing} line1={whenLine(raw.as_of, v.basis, raw.reporting_ccy)} line2={`minimum ${raw.minimum.toFixed(0)}% · internal ${raw.internal_minimum.toFixed(0)}%`} testId="headline" />
          <Gauge value={raw.ratio ?? 0} limit={Math.max(raw.internal_minimum, raw.minimum)} trigger={null} max={Math.max(raw.ratio ?? 0, raw.internal_minimum) * 1.15} floor width={320} />
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="Totals" width={320}>
              <KV rows={v.totals} />
            </Card>
          </div>
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="Available stable funding" width={480}>
              <SimpleTable headers={['Class · band', 'Amount', 'Factor', 'Weighted']} rows={v.asfRows} />
            </Card>
            <Card title="Required stable funding" width={480}>
              <SimpleTable headers={['Class · band', 'Amount', 'Factor', 'Weighted']} rows={v.rsfRows} />
            </Card>
          </div>
        </>
      )}
    </ToolLayout>
  );
}
