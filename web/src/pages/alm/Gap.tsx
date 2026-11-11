import { useMemo } from 'react';
import type { GapResult } from '../../api/types';
import { Card } from '../../components/ui';
import { Bars } from '../../components/charts';
import { useAlm, useTool, useUnhedged } from '../../alm/state';
import { AssumptionsHeader, Group, Headline, HedgeEffect, MigrationNote, Num, Pct, ResultsHeader, Row, SimpleTable, ToolLayout } from '../../alm/tool';
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
  const m = working.behaviour;
  const bm = bank.behaviour;
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
            <Row label="To term, per +100 bp">
              <Pct value={m.migration_pct_per_100bp[k] ?? 0} base={bm.migration_pct_per_100bp[k] ?? 0} onChange={(x) => edit((p) => void (p.behaviour.migration_pct_per_100bp[k] = x))} />
            </Row>
          </div>
        ))}
        <div className="t-medium" style={{ margin: '8px 0 2px', fontSize: 12 }}>
          Migration into term deposits
        </div>
        <Row label="Most that can move">
          <Pct value={m.migration_cap_pct} base={bm.migration_cap_pct} onChange={(x) => edit((p) => void (p.behaviour.migration_cap_pct = x))} />
        </Row>
        <Row label="Term it moves to (months)">
          <Num value={m.migration_term_months} base={bm.migration_term_months} min={1} max={120} step={1} onChange={(x) => edit((p) => void (p.behaviour.migration_term_months = Math.round(x)))} />
        </Row>
      </div>
      <div className="agroup-help" style={{ marginTop: 6 }}>
        The non-core share sits in the overnight bucket. The core runs off over its life: evenly at zero decay, otherwise front-loaded, with whatever is left at the end of the life falling due then. Migration applies only when a shock raises short rates: that share of the balance becomes term deposits paying the shocked rate for the term.
      </div>
    </Group>
  );
}

const MULTIPLIERS = [
  ['prepayment_when_rates_rise', 'Prepayment when rates rise'],
  ['prepayment_when_rates_fall', 'Prepayment when rates fall'],
  ['withdrawal_when_rates_rise', 'Withdrawal when rates rise'],
  ['withdrawal_when_rates_fall', 'Withdrawal when rates fall'],
] as const;

/** Loans prepaid and term deposits broken before maturity: it shortens fixed-rate contracts in the
 * gap, NII and EVE. Closed to start with, as everything is zero until the bank sets it. */
export function EarlyRepayment() {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const w = working.behaviour;
  const b = bank.behaviour;
  const set = Object.values(w.prepayment_pct).concat(Object.values(w.withdrawal_pct)).some((v) => v > 0);
  return (
    <Group
      title="Prepayment and early withdrawal"
      help="How much of each fixed-rate product leaves before maturity each year. At zero the contractual schedule is used. It shortens these contracts in the gap, NII and EVE; the liquidity tools stay contractual."
      open={set}
      count={Object.keys(w.prepayment_pct).length + Object.keys(w.withdrawal_pct).length}
    >
      <div data-testid="early-repayment">
        <div className="t-medium" style={{ margin: '6px 0 2px', fontSize: 12 }}>
          Loans prepaid (a year)
        </div>
        {Object.entries(w.prepayment_pct).map(([k, v]) => (
          <Row key={k} label={nice(k)} cover={`behaviour.prepayment_pct.${k}`}>
            <Pct value={v} base={b.prepayment_pct[k]} max={50} onChange={(x) => edit((p) => void (p.behaviour.prepayment_pct[k] = x))} />
          </Row>
        ))}
        <div className="t-medium" style={{ margin: '8px 0 2px', fontSize: 12 }}>
          Term deposits broken early (a year)
        </div>
        {Object.entries(w.withdrawal_pct).map(([k, v]) => (
          <Row key={k} label={nice(k)} cover={`behaviour.withdrawal_pct.${k}`}>
            <Pct value={v} base={b.withdrawal_pct[k]} max={50} onChange={(x) => edit((p) => void (p.behaviour.withdrawal_pct[k] = x))} />
          </Row>
        ))}
        <div className="t-medium" style={{ margin: '8px 0 2px', fontSize: 12 }}>
          Speed under a rate shock (times the rate above)
        </div>
        {MULTIPLIERS.map(([k, label]) => (
          <Row key={k} label={label}>
            <Num value={w[k]} base={b[k]} min={0} max={3} step={0.05} decimals={2} suffix="×" onChange={(x) => edit((p) => void (p.behaviour[k] = x))} />
          </Row>
        ))}
      </div>
      <div className="agroup-help" style={{ marginTop: 6 }}>
        Borrowers refinance when rates fall and hold on when they rise; depositors break a deposit to earn more when rates rise. The multipliers are Basel's standardised values.
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
      <EarlyRepayment />
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
              <span className="t-muted">A rate shock leaves these balances where they are, unless deposit migration is set. Choose one above to see what the gap does to income.</span>
            )}
          </div>
          <MigrationNote amount={raw.migrated}>That share has left the buckets its behavioural life gave it and sits in the term's bucket below.</MigrationNote>
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
