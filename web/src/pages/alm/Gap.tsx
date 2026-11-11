import { useMemo } from 'react';
import type { GapResult } from '../../api/types';
import { Card } from '../../components/ui';
import { Bars } from '../../components/charts';
import { useAlm, useTool, useUnhedged } from '../../alm/state';
import { AssumptionsHeader, Group, Headline, HedgeEffect, Num, Pct, ResultsHeader, Row, SimpleTable, ToolLayout } from '../../alm/tool';
import { gapView, whenLine } from '../../alm/views';
import { fmtMoneyShort, nice, signed } from '../../lib/fmt';

const pct1 = (v: number) => `${v >= 0 ? '+' : ''}${v.toFixed(1)}%`;

const BUCKETS = ['ON', '1W', '1M', '3M', '6M', '1Y', '2Y', '5Y', '10Y+'];

/** How deposits with no maturity are taken to behave: it drives both the gap and EVE. Per
 * product: how long the core stays, how much of the balance is core, and how fast the core decays. */
export function DepositBehaviour({ help }: { help: string }) {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const g = working.gap;
  const b = bank.gap;
  return (
    <Group title="Deposit behaviour" help={help} count={Object.keys(g.nmd_behavioural_months).length}>
      <div data-testid="deposit-behaviour">
        {Object.entries(g.nmd_behavioural_months).map(([k, v]) => (
          <div key={k}>
            <div className="t-medium" style={{ margin: '8px 0 2px', fontSize: 12 }}>
              {nice(k)}
            </div>
            <Row label="Life (months)" cover={`gap.nmd_behavioural_months.${k}`}>
              <Num value={v} base={b.nmd_behavioural_months[k]} min={1} max={240} step={1} onChange={(x) => edit((p) => void (p.gap.nmd_behavioural_months[k] = x))} />
            </Row>
            <Row label="Core share">
              <Pct value={g.nmd_core_pct[k] ?? 100} base={b.nmd_core_pct[k] ?? 100} onChange={(x) => edit((p) => void (p.gap.nmd_core_pct[k] = x))} />
            </Row>
            <Row label="Core decay (a year)">
              <Pct value={g.nmd_decay_pct[k] ?? 0} base={b.nmd_decay_pct[k] ?? 0} max={99} onChange={(x) => edit((p) => void (p.gap.nmd_decay_pct[k] = x))} />
            </Row>
          </div>
        ))}
      </div>
      <div className="agroup-help" style={{ marginTop: 6 }}>
        The non-core share sits in the overnight bucket. The core runs off over its life: evenly at zero decay, otherwise front-loaded, with whatever is left at the end of the life falling due then.
      </div>
    </Group>
  );
}

function Assumptions() {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const g = working.gap;
  const b = bank.gap;
  const keys = Object.keys(g.cumulative_gap_limit_pct_assets).sort((a, b) => BUCKETS.indexOf(a) - BUCKETS.indexOf(b));
  return (
    <>
      <AssumptionsHeader />
      <DepositBehaviour help="How deposits with no contractual maturity are assumed to behave, which decides the buckets they reprice in." />
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
  const plain = useUnhedged<GapResult>('gap');
  const plainY1 = plain?.buckets.find((b) => b.label === '1Y');
  return (
    <ToolLayout assumptions={<Assumptions />} ready={!!v} busy={q.isFetching && !!v}>
      <ResultsHeader title="Repricing gap" loading={q.isFetching} error={q.error?.message} onRetry={() => void q.refetch()} shock={raw?.shock_label} rates />
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
          {plain && plainY1 && v.y1 && (
            <HedgeEffect>
              With the trial hedges the cumulative gap at one year is <span className="t-medium">{pct1(v.y1.cumulative_pct_assets)}</span> of assets; without them it is {pct1(plainY1.cumulative_pct_assets)}.
              {raw.nii_effect_12m !== null && plain.nii_effect_12m !== null && (
                <>
                  {' '}
                  The gap estimate of the shock's effect on NII moves from {signed(plain.nii_effect_12m)} to <span className="t-medium">{signed(raw.nii_effect_12m)}</span>.
                </>
              )}
            </HedgeEffect>
          )}
          <div className="shock-result" data-testid="gap-effect">
            {v.shocked ? (
              <>
                <span className="t-medium">{v.effect}</span> on the next twelve months' NII under {raw.shock_label}, by the gap estimate: each bucket's gap times the rate move times the part of the year left after it reprices.
              </>
            ) : (
              <span className="t-muted">A rate shock does not move these balances. Choose one above to see what the gap does to income.</span>
            )}
          </div>
          <div className="row-wrap">
            <Card title="Gap per bucket (assets less liabilities)" width={460}>
              <Bars data={v.gapBars} color="var(--s1)" height={170} fmt={fmtMoneyShort} />
            </Card>
            <Card title="Cumulative gap" width={460}>
              <Bars data={v.cumulativeBars} color="var(--s3)" height={170} fmt={fmtMoneyShort} />
            </Card>
          </div>
          {v.shocked && (
            <div className="row-wrap" style={{ marginTop: 10 }}>
              <Card title="Effect on 12-month NII per bucket (gap estimate)" width={460}>
                <Bars data={v.effectBars} color="var(--s2)" height={170} fmt={fmtMoneyShort} />
              </Card>
              <Card title="Reading it" width={460}>
                <p className="t-small prose" style={{ margin: 0 }}>
                  A positive gap means more assets than liabilities reprice in that bucket, so a rise in rates adds income and a fall takes it away. Buckets beyond one year add nothing, because nothing in them reprices within the twelve months. The estimate takes every balance to pass the move on in full; the NII tab applies each deposit's beta and is the fuller answer.
                </p>
              </Card>
            </div>
          )}
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="Buckets" width={v.shocked ? 860 : 680}>
              <SimpleTable headers={v.headers} rows={v.rows} testId="gap-buckets" />
            </Card>
            <Card title="By currency at one year" width={v.shocked ? 380 : 260}>
              <SimpleTable headers={v.ccyHeaders} rows={v.ccyRows} />
            </Card>
          </div>
        </>
      )}
    </ToolLayout>
  );
}
