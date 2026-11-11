import { useMemo } from 'react';
import type { BalanceSheetMode, NiiProduct, NiiResult } from '../../api/types';
import { Card, Chip } from '../../components/ui';
import { Bars } from '../../components/charts';
import { useAlm, useTool } from '../../alm/state';
import { AssumptionsHeader, Group, Headline, Num, Pct, ResultsHeader, Row, SimpleTable, ToolLayout } from '../../alm/tool';
import { MODE_LABEL, niiView } from '../../alm/views';
import { fmtDateLong, fmtMoneyShort, fmtMonth, nice } from '../../lib/fmt';

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

/** `CURRENT_ACCOUNT.RETAIL` → "Current account, retail". */
export function betaLabel(key: string): string {
  const [product, segment] = key.split('.');
  return segment ? `${nice(product)}, ${nice(segment).toLowerCase()}` : nice(product);
}

export function BetaRows({ betas, base, onChange }: { betas: Record<string, number>; base: Record<string, number>; onChange: (key: string, v: number) => void }) {
  return (
    <>
      {Object.keys(betas)
        .sort()
        .map((k) => (
          <Row key={k} label={betaLabel(k)} cover={`nii.betas.${k}`}>
            <Pct value={betas[k] * 100} base={base[k] === undefined ? undefined : base[k] * 100} onChange={(v) => onChange(k, v / 100)} />
          </Row>
        ))}
    </>
  );
}

const MODES: BalanceSheetMode[] = ['CONSTANT', 'RUN_OFF', 'DYNAMIC'];
const MODE_HELP: Record<BalanceSheetMode, string> = {
  CONSTANT: 'Static: a cohort that matures is replaced like for like, so balances stay as they are today.',
  RUN_OFF: 'Static, running off: a cohort earns or pays nothing after it matures, and the cash it frees or takes goes to the balancing book at the one-month rate.',
  DYNAMIC: 'Cohorts are replaced and every balance grows at the yearly rate below. The cash growth takes or brings goes to the balancing book at the one-month rate.',
};
const modeOf = (n: { dynamic: boolean; constant_balance_sheet: boolean }): BalanceSheetMode => (n.dynamic ? 'DYNAMIC' : n.constant_balance_sheet ? 'CONSTANT' : 'RUN_OFF');

const Growth1 = ({ value, base, onChange }: { value: number; base?: number; onChange: (v: number) => void }) => <Num value={value} base={base} min={-50} max={100} step={0.5} decimals={1} suffix="%" onChange={onChange} />;

/** Yearly growth on a dynamic balance sheet: one rate a side, and a product's own where it differs. */
function Growth({ products }: { products: NiiProduct[] }) {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const n = working.nii;
  const b = bank.nii;
  const side = (p: NiiProduct) => (p.side === 'ASSET' ? n.growth_assets_pct : n.growth_liabilities_pct);
  return (
    <>
      <Group title="Growth (% a year)" help="How fast balances grow on a dynamic balance sheet. New volume is priced as the cohort it joins. With both at zero the dynamic balance sheet is the constant one.">
        <Row label="Assets">
          <Growth1 value={n.growth_assets_pct} base={b.growth_assets_pct} onChange={(v) => edit((p) => void (p.nii.growth_assets_pct = v))} />
        </Row>
        <Row label="Liabilities">
          <Growth1 value={n.growth_liabilities_pct} base={b.growth_liabilities_pct} onChange={(v) => edit((p) => void (p.nii.growth_liabilities_pct = v))} />
        </Row>
      </Group>
      <Group title="Growth by product (% a year)" help="A product's own rate, where it differs from its side's. Leave a product at its side's rate to follow it." open={false} count={products.length}>
        {products.map((p) => (
          <Row key={`${p.side}.${p.product}`} label={`${nice(p.product)} (${p.side === 'ASSET' ? 'asset' : 'liability'}, ${fmtMoneyShort(p.balance)})`}>
            <Growth1 value={n.growth_by_product[p.product] ?? side(p)} base={b.growth_by_product[p.product]} onChange={(v) => edit((d) => void (d.nii.growth_by_product[p.product] = v))} />
          </Row>
        ))}
      </Group>
    </>
  );
}

function Assumptions({ products }: { products: NiiProduct[] }) {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const n = working.nii;
  const b = bank.nii;
  const mode = modeOf(n);
  return (
    <>
      <AssumptionsHeader />
      <Group title="Projection" help="How far ahead to project and what the balance sheet is taken to do over that time.">
        <Row label="Horizon (months)">
          <Num value={n.horizon_months} base={b.horizon_months} min={1} max={60} step={1} onChange={(v) => edit((p) => void (p.nii.horizon_months = Math.round(v)))} />
        </Row>
        <div className="arow">
          <span className="k">Balance sheet</span>
          <span className="v chips" data-testid="balance-sheet-mode">
            {MODES.map((m) => (
              <Chip
                key={m}
                small
                outlined
                selected={mode === m}
                title={MODE_HELP[m]}
                onClick={() =>
                  edit((p) => {
                    p.nii.dynamic = m === 'DYNAMIC';
                    p.nii.constant_balance_sheet = m !== 'RUN_OFF';
                  })
                }
              >
                {MODE_LABEL[m]}
              </Chip>
            ))}
          </span>
        </div>
        <div className="agroup-help">{MODE_HELP[mode]}</div>
      </Group>
      {mode === 'DYNAMIC' && <Growth products={products} />}
      <Group title="Rate pass-through (beta)" help="How much of a move in short rates each administered product passes on to its rate. An entry for a product and segment beats the product's; products not listed take the default. They matter only when rates move, so choose a rate shock above to see them work." count={Object.keys(n.betas).length + 1}>
        <BetaRows betas={n.betas} base={b.betas} onChange={(k, v) => edit((p) => void (p.nii.betas[k] = v))} />
        <Row label="Other products (default)" cover="nii.default_beta">
          <Pct value={n.default_beta * 100} base={b.default_beta * 100} onChange={(v) => edit((p) => void (p.nii.default_beta = v / 100))} />
        </Row>
      </Group>
      <Group title="Rate shocks (bp)" help="The size of each Basel shock per currency: parallel, short end and long end." open={false} count={Object.keys(working.stress.shocks).length}>
        <ShockGrid />
      </Group>
      <p className="t-small prose" style={{ marginTop: 10 }}>
        Fixed cohorts keep their rate until maturity and roll at the shocked curve with their margin; floating cohorts reprice at each reset; administered deposits move by their beta.
      </p>
    </>
  );
}

export function Nii() {
  const q = useTool<NiiResult>('nii');
  const v = useMemo(() => (q.data ? niiView(q.data) : null), [q.data]);
  const raw = q.data;
  return (
    <ToolLayout assumptions={<Assumptions products={raw?.products ?? []} />} ready={!!v} busy={q.isFetching && !!v}>
      <ResultsHeader title="Net interest income (NII)" loading={q.isFetching} error={q.error?.message} onRetry={() => void q.refetch()} shock={raw?.shock_label} />
      {v && raw && (
        <>
          <Headline value={v.headline} standing="comfortable" line1={`over ${raw.horizon_months} months from ${fmtDateLong(raw.as_of)} · ${v.basis} · in ${raw.reporting_ccy}`} line2={`NIM ${v.nim} on earning assets of ${fmtMoneyShort(raw.earning_assets)}${v.delta ? ` · ${v.delta}` : ''}`} testId="headline" />
          <Card title="NII by month" width={720}>
            <Bars
              data={v.monthBars}
              color="var(--s1)"
              height={170}
              fmt={fmtMoneyShort}
              detail={(i) => {
                const m = raw.months[i];
                return `${fmtMonth(m.period, true)}: NII ${fmtMoneyShort(m.nii)} · income ${fmtMoneyShort(m.income)} · expense ${fmtMoneyShort(m.expense)}`;
              }}
            />
          </Card>
          <Card title="Static against dynamic: the same projection under each balance-sheet assumption" width={720} style={{ marginTop: 10 }}>
            <SimpleTable headers={v.modeHeaders} rows={v.modeRows} mark={v.modeMark >= 0 ? v.modeMark : undefined} testId="nii-modes" />
            <div className="t-small" style={{ marginTop: 4 }}>
              The highlighted row is the one charted above. Constant and run-off are the two static views; dynamic adds the growth set under Assumptions.
            </div>
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
