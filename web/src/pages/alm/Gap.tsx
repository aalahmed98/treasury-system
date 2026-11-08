import { useMemo } from 'react';
import type { GapResult } from '../../api/types';
import { Card } from '../../components/ui';
import { Bars } from '../../components/charts';
import { useAlm, useTool } from '../../alm/state';
import { AssumptionsHeader, Group, Headline, Num, Pct, ResultsHeader, Row, SimpleTable, ToolLayout } from '../../alm/tool';
import { gapView, whenLine } from '../../alm/views';
import { fmtMoneyShort, nice } from '../../lib/fmt';

const BUCKETS = ['ON', '1W', '1M', '3M', '6M', '1Y', '2Y', '5Y', '10Y+'];

function Assumptions() {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const g = working.gap;
  const b = bank.gap;
  const keys = Object.keys(g.cumulative_gap_limit_pct_assets).sort((a, b) => BUCKETS.indexOf(a) - BUCKETS.indexOf(b));
  return (
    <>
      <AssumptionsHeader />
      <Group title="Behavioural life (months)" help="How long deposits with no contractual maturity are assumed to stay, which decides the bucket they reprice in.">
        {Object.entries(g.nmd_behavioural_months).map(([k, v]) => (
          <Row key={k} label={nice(k)} cover={`gap.nmd_behavioural_months.${k}`}>
            <Num value={v} base={b.nmd_behavioural_months[k]} min={1} max={240} step={1} onChange={(x) => edit((p) => void (p.gap.nmd_behavioural_months[k] = x))} />
          </Row>
        ))}
      </Group>
      <Group title="Cumulative gap limits (% of assets)" help="The largest cumulative gap allowed at each horizon, as a share of total assets." count={keys.length}>
        {keys.map((k) => (
          <Row key={k} label={k}>
            <Pct value={g.cumulative_gap_limit_pct_assets[k]} base={b.cumulative_gap_limit_pct_assets[k]} onChange={(x) => edit((p) => void (p.gap.cumulative_gap_limit_pct_assets[k] = x))} />
          </Row>
        ))}
      </Group>
    </>
  );
}

export function Gap() {
  const q = useTool<GapResult>('gap');
  const v = useMemo(() => (q.data ? gapView(q.data) : null), [q.data]);
  const raw = q.data;
  return (
    <ToolLayout assumptions={<Assumptions />} ready={!!v} busy={q.isFetching && !!v}>
      <ResultsHeader title="Repricing gap" loading={q.isFetching} error={q.error?.message} onRetry={() => void q.refetch()} />
      {v && raw && (
        <>
          <Headline
            value={v.y1 ? `${v.y1.cumulative_pct_assets >= 0 ? '+' : ''}${v.y1.cumulative_pct_assets.toFixed(1)}%` : '—'}
            standing={v.breaches.length ? 'breach' : 'comfortable'}
            line1={`cumulative gap at one year, as a share of total assets of ${fmtMoneyShort(raw.total_assets)}`}
            line2={whenLine(raw.as_of, v.basis, raw.reporting_ccy)}
            testId="headline"
          />
          {v.breaches.length > 0 && <div className="t-bad" style={{ marginBottom: 8 }}>outside the limit: {v.breaches.join('; ')}</div>}
          <div className="row-wrap">
            <Card title="Gap per bucket (assets less liabilities)" width={460}>
              <Bars data={v.gapBars} color="var(--s1)" height={170} fmt={fmtMoneyShort} />
            </Card>
            <Card title="Cumulative gap" width={460}>
              <Bars data={v.cumulativeBars} color="var(--s3)" height={170} fmt={fmtMoneyShort} />
            </Card>
          </div>
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="Buckets" width={680}>
              <SimpleTable headers={['Bucket', 'Assets', 'Liabilities', 'Gap', 'Cumulative', 'Cumulative % (limit)']} rows={v.rows} />
            </Card>
            <Card title="By currency at one year" width={260}>
              <SimpleTable headers={['Currency', 'Cumulative', '% assets']} rows={v.ccyRows} />
            </Card>
          </div>
        </>
      )}
    </ToolLayout>
  );
}
