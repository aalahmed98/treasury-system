import { useMemo } from 'react';
import type { NiiResult } from '../../api/types';
import { Card } from '../../components/ui';
import { Bars } from '../../components/charts';
import { useAlm, useTool } from '../../alm/state';
import { AssumptionsHeader, Group, Headline, Num, Pct, ResultsHeader, Row, ScenarioPicker, SimpleTable, ToolLayout } from '../../alm/tool';
import { niiView } from '../../alm/views';
import { fmtDateLong, fmtMoneyShort } from '../../lib/fmt';

export function ShockGrid() {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  return (
    <div className="factor-grid">
      <span />
      {['Parallel', 'Short', 'Long'].map((h) => (
        <span key={h} className="h">
          {h}
        </span>
      ))}
      {Object.entries(working.stress.shocks).map(([ccy, s]) => (
        <div key={ccy} style={{ display: 'contents' }}>
          <span className="k">{ccy}</span>
          {(['parallel', 'short', 'long'] as const).map((f) => (
            <Num key={f} value={s[f]} base={bank.stress.shocks[ccy]?.[f]} min={0} max={2000} step={5} width={64} onChange={(x) => edit((p) => void (p.stress.shocks[ccy][f] = x))} />
          ))}
        </div>
      ))}
    </div>
  );
}

function Assumptions() {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const n = working.nii;
  const b = bank.nii;
  return (
    <>
      <AssumptionsHeader />
      <Group title="Rate scenario" help="The curve the projection runs on: today's, one of the Basel shocks, or a custom parallel move.">
        <ScenarioPicker includeBase />
      </Group>
      <Group title="Projection" help="How far ahead to project, whether the balance sheet is held constant, and how much of a rate move administered deposits pass on.">
        <Row label="Horizon (months)">
          <Num value={n.horizon_months} base={b.horizon_months} min={1} max={60} step={1} onChange={(v) => edit((p) => void (p.nii.horizon_months = Math.round(v)))} />
        </Row>
        <Row label="Constant balance sheet">
          <input type="checkbox" checked={n.constant_balance_sheet} onChange={(e) => edit((p) => void (p.nii.constant_balance_sheet = e.target.checked))} />
        </Row>
        <Row label="Default deposit beta">
          <Pct value={n.default_beta * 100} base={b.default_beta * 100} onChange={(v) => edit((p) => void (p.nii.default_beta = v / 100))} />
        </Row>
      </Group>
      <Group title="Rate shocks (bp)" help="The size of each Basel shock per currency: parallel, short end and long end." open={false} count={Object.keys(working.stress.shocks).length}>
        <ShockGrid />
      </Group>
      <p className="t-small prose" style={{ marginTop: 10 }}>
        Fixed cohorts keep their rate until maturity and roll at the scenario curve with their margin; floating cohorts reprice at each reset; administered deposits move by their beta.
      </p>
    </>
  );
}

export function Nii() {
  const q = useTool<NiiResult>('nii');
  const v = useMemo(() => (q.data ? niiView(q.data) : null), [q.data]);
  const raw = q.data;
  return (
    <ToolLayout assumptions={<Assumptions />} ready={!!v} busy={q.isFetching && !!v}>
      <ResultsHeader title="Net interest income (NII)" loading={q.isFetching} error={q.error?.message} onRetry={() => void q.refetch()} />
      {v && raw && (
        <>
          <Headline value={v.headline} standing="comfortable" line1={`over ${raw.horizon_months} months from ${fmtDateLong(raw.as_of)} · ${v.basis} · in ${raw.reporting_ccy}`} line2={`NIM ${v.nim} on earning assets of ${fmtMoneyShort(raw.earning_assets)}${v.delta ? ` · ${v.delta}` : ''}`} testId="headline" />
          <Card title="NII by month" width={720}>
            <Bars data={v.monthBars} color="var(--s1)" height={170} fmt={fmtMoneyShort} labels="extremes" />
          </Card>
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="Months" width={440}>
              <SimpleTable headers={['Month', 'Income', 'Expense', 'NII']} rows={v.rows} />
            </Card>
            <Card title="By book" width={240}>
              <SimpleTable headers={['Book', 'NII']} rows={v.byBook} />
            </Card>
            <Card title="By currency" width={220}>
              <SimpleTable headers={['Currency', 'NII']} rows={v.byCcy} />
            </Card>
          </div>
        </>
      )}
    </ToolLayout>
  );
}
