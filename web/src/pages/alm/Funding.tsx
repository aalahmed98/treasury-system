import { useMemo } from 'react';
import type { FundingResult } from '../../api/types';
import { Card } from '../../components/ui';
import { Bars } from '../../components/charts';
import { useAlm, useTool } from '../../alm/state';
import { AssumptionsHeader, Group, Headline, KV, Pct, ResultsHeader, Row, SimpleTable, ToolLayout } from '../../alm/tool';
import { fundingView, whenLine } from '../../alm/views';

function Assumptions() {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const f = working.funding;
  const b = bank.funding;
  return (
    <>
      <AssumptionsHeader />
      <Group title="Limits" help="How concentrated the deposit base may be before the bank wants to know.">
        <Row label="Largest depositor (% of deposits)">
          <Pct value={f.top1_limit_pct} base={b.top1_limit_pct} onChange={(v) => edit((p) => void (p.funding.top1_limit_pct = v))} />
        </Row>
        <Row label="Ten largest depositors">
          <Pct value={f.top10_limit_pct} base={b.top10_limit_pct} onChange={(v) => edit((p) => void (p.funding.top10_limit_pct = v))} />
        </Row>
        <Row label="Wholesale funding (% of funding)">
          <Pct value={f.wholesale_share_limit_pct} base={b.wholesale_share_limit_pct} onChange={(v) => edit((p) => void (p.funding.wholesale_share_limit_pct = v))} />
        </Row>
      </Group>
    </>
  );
}

export function Funding() {
  const q = useTool<FundingResult>('funding');
  const v = useMemo(() => (q.data ? fundingView(q.data) : null), [q.data]);
  const raw = q.data;
  return (
    <ToolLayout assumptions={<Assumptions />} ready={!!v}>
      <ResultsHeader title="Funding concentration" loading={q.isFetching} error={q.error?.message} />
      {v && raw && (
        <>
          <Headline value={`${raw.top10_pct.toFixed(1)}%`} standing={v.standing} line1={`ten largest depositor groups as a share of deposits (limit ${raw.top10_limit_pct.toFixed(0)}%)`} line2={whenLine(raw.as_of, v.basis, raw.reporting_ccy)} testId="headline" />
          {raw.breaches.length > 0 && <div className="t-bad" style={{ marginBottom: 8 }}>{raw.breaches.join('; ')}</div>}
          <div className="row-wrap">
            <Card title="Measures" width={340}>
              <KV rows={v.headlineRows} />
            </Card>
            <Card title="Largest groups (% of deposits)" width={560}>
              <Bars data={v.groupBars} color="var(--s4)" height={170} fmt={(x) => `${x.toFixed(1)}%`} />
            </Card>
          </div>
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="Depositor groups" width={560}>
              <SimpleTable headers={['Group', 'Segment', 'Balance', 'Share']} rows={v.groupRows} />
            </Card>
            <Card title="Deposits by segment" width={300}>
              <SimpleTable headers={['Segment', 'Deposits']} rows={v.segmentRows} />
            </Card>
          </div>
        </>
      )}
    </ToolLayout>
  );
}
