import { useMemo } from 'react';
import type { AttributionResult } from '../../api/types';
import { Card } from '../../components/ui';
import { Waterfall } from '../../components/charts';
import { useAlm, useTool } from '../../alm/state';
import { ComparePicker, Group, Headline, KV, ResultsHeader, SimpleTable, ToolLayout } from '../../alm/tool';
import { attributionView } from '../../alm/views';
import { fmtMonth, fmtMoneyShort } from '../../lib/fmt';

function addMonths(d: string, n: number): string {
  const [y, m] = d.split('-').map(Number);
  const t = new Date(Date.UTC(y, m - 1 + n, 1));
  return `${t.getUTCFullYear()}-${String(t.getUTCMonth() + 1).padStart(2, '0')}-01`;
}

function Inputs({ shown }: { shown: string | null }) {
  const { period, setPeriod } = useAlm();
  return (
    <>
      <div className="t-heading" style={{ marginBottom: 6 }}>
        Comparison
      </div>
      <ComparePicker />
      <Group title="Month" />
      <div className="row">
        <button type="button" className="btn btn-sm" disabled={!shown} onClick={() => shown && setPeriod(addMonths(shown, -1))} aria-label="previous month">
          ◀
        </button>
        <span className="t-medium">{shown ? fmtMonth(shown, true) : 'latest'}</span>
        <button type="button" className="btn btn-sm" disabled={!shown} onClick={() => shown && setPeriod(addMonths(shown, 1))} aria-label="next month">
          ▶
        </button>
        {period && (
          <button type="button" className="btn btn-sm" onClick={() => setPeriod(null)}>
            latest
          </button>
        )}
      </div>
      <p className="t-small" style={{ marginTop: 12 }}>
        Each line's change splits into the effect of the number of days, of volume at the old rate, of rate on the old volume and the joint residual. Volume is split at portfolio level into pure growth and mix. Assets and liabilities are separate portfolios and every
        currency is converted at one spot, so the effects add up to the change exactly.
      </p>
    </>
  );
}

export function Attribution() {
  const { period } = useAlm();
  const q = useTool<AttributionResult>('attribution');
  const v = useMemo(() => (q.data ? attributionView(q.data) : null), [q.data]);
  const shown = period ?? q.data?.period_b ?? null;
  return (
    <ToolLayout assumptions={<Inputs shown={shown} />} ready={!!v} busy={q.isFetching && !!v}>
      <ResultsHeader title="NII variance attribution" loading={q.isFetching} error={q.error?.message} onRetry={() => void q.refetch()} />
      {v && (
        <>
          <Headline value={v.headline} standing={v.standing} line1={v.subtitle} line2={v.range} testId="headline" />
          <div className="row-wrap">
            <Card title="Waterfall" width={620}>
              <Waterfall steps={v.steps} fmt={fmtMoneyShort} />
            </Card>
            <Card title="Effects" width={260}>
              <KV rows={v.effectRows} boldLast />
            </Card>
          </div>
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="By line" width={900}>
              <SimpleTable headers={['Book', 'Side', 'Ccy', 'NII before', 'NII after', 'Rate before', 'Rate after', 'Change']} rows={v.rows} />
            </Card>
          </div>
        </>
      )}
    </ToolLayout>
  );
}
