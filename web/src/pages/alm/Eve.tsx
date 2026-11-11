// Economic value of equity under the rate shock chosen above: the full revaluation, where on the
// curve it comes from, and the duration gap's estimate beside it.
import { useMemo } from 'react';
import type { EveResult } from '../../api/types';
import { Card } from '../../components/ui';
import { Bars } from '../../components/charts';
import { useTool, useUnhedged } from '../../alm/state';
import { AssumptionsHeader, Headline, HedgeEffect, KV, ResultsHeader, SimpleTable, ToolLayout } from '../../alm/tool';
import { eveView, whenLine } from '../../alm/views';
import { fmtMoneyShort, signed } from '../../lib/fmt';
import { DepositBehaviour, EarlyRepayment } from './Gap';
import { RateLimits, ShockSizes } from './Stress';

function Assumptions() {
  return (
    <>
      <AssumptionsHeader />
      <RateLimits nii={false} />
      <DepositBehaviour help="How deposits with no contractual maturity are assumed to behave. A longer life or a larger core puts their value further out the curve, where a rate move changes it more." />
      <EarlyRepayment />
      <ShockSizes />
      <p className="t-small prose" style={{ marginTop: 10 }}>
        Every rate-sensitive cash flow is discounted on today's curve and again on the shocked one. Fixed cohorts contribute their schedule (shortened by any prepayment or early withdrawal set below), floating cohorts their balance and accrued interest at the next reset, deposits with no maturity their behavioural run-off. Equity and other items that do not respond to rates are left out.
      </p>
    </>
  );
}

export function Eve() {
  const q = useTool<EveResult>('eve');
  const v = useMemo(() => (q.data ? eveView(q.data) : null), [q.data]);
  const raw = q.data;
  const plain = useUnhedged<EveResult>('eve');
  return (
    <ToolLayout assumptions={<Assumptions />} ready={!!v} busy={q.isFetching && !!v}>
      <ResultsHeader title="Economic value of equity (EVE)" loading={q.isFetching} error={q.error?.message} onRetry={() => void q.refetch()} shock={raw?.shock_label} rates />
      {v && raw && (
        <>
          {v.shocked ? (
            <Headline
              value={v.headline}
              standing={v.standing}
              line1={`change in EVE under ${raw.shock_label} · loss of ${v.lossPct} of Tier 1 (${fmtMoneyShort(raw.tier1_capital)}) against a limit of ${raw.limit_pct_tier1.toFixed(0)}%`}
              line2={`${whenLine(raw.as_of, v.basis, raw.reporting_ccy)} · EVE today ${fmtMoneyShort(raw.base)}`}
              testId="headline"
            />
          ) : (
            <Headline value={v.headline} standing="comfortable" line1="economic value of equity on today's curve: the present value of assets less liabilities" line2={whenLine(raw.as_of, v.basis, raw.reporting_ccy)} testId="headline" />
          )}
          {plain && (
            <HedgeEffect>
              {v.shocked ? (
                <>
                  Without the trial hedges the change in EVE would be {signed(plain.delta)}; they move it by <span className="t-medium">{signed(raw.delta - plain.delta)}</span>.
                </>
              ) : (
                <>
                  The trial hedges are worth <span className="t-medium">{signed(raw.base - plain.base)}</span> on today's curve and change the duration gap from {plain.duration.gap_years.toFixed(2)} to <span className="t-medium">{raw.duration.gap_years.toFixed(2)} years</span>.
                </>
              )}
            </HedgeEffect>
          )}
          {!v.shocked && (
            <div className="shock-result">
              <span className="t-muted">Choose a rate shock above to see how much of this value it adds or takes away.</span>
            </div>
          )}
          <div className="row-wrap">
            {v.shocked ? (
              <Card title="Change in EVE by bucket" width={460}>
                <Bars data={v.deltaBars} color="var(--s2)" height={170} fmt={fmtMoneyShort} />
              </Card>
            ) : (
              <Card title="Value by bucket (assets less liabilities)" width={460}>
                <Bars data={v.valueBars} color="var(--s1)" height={170} fmt={fmtMoneyShort} />
              </Card>
            )}
            <Card title="Duration gap" width={460}>
              <div data-testid="duration-gap">
                <KV rows={v.durationRows} />
                {v.estimateRows.length > 0 && (
                  <div style={{ marginTop: 8, paddingTop: 8, borderTop: '1px solid var(--hairline)' }}>
                    <KV rows={v.estimateRows} boldLast />
                  </div>
                )}
              </div>
              <div className="t-small" style={{ marginTop: 8 }}>
                ΔEVE ≈ −[D_A − (L/A) × D_L] × A × Δy / (1 + y)
              </div>
              <div className="t-small">
                {v.noEstimate
                  ? 'The formula assumes every tenor moves by the same amount, so it has no estimate for this shock. The key rate estimate adds up each bucket’s sensitivity times its own move.'
                  : v.estimateRows.length > 0
                    ? 'Both estimates are a straight line through today’s value; the full revaluation also catches the curvature and the post-shock floor.'
                    : 'A positive gap means assets are longer than liabilities, so a rise in rates lowers EVE.'}
              </div>
            </Card>
          </div>
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="Key rate sensitivity (EVE per +1 bp, by bucket)" width={460}>
              <Bars data={v.keyBars} color="var(--s4)" height={170} fmt={fmtMoneyShort} />
              <div className="t-small" style={{ marginTop: 4 }}>
                A bar below the line is a tenor where a rise in rates costs value; above it, one where it adds value. All tenors together: {signed(v.keyTotal)} per basis point.
              </div>
            </Card>
            <Card title="Key rates per basis point" width={460}>
              <SimpleTable headers={v.keyHeaders} rows={v.keyRows} testId="key-rates" />
              <div className="t-small" style={{ marginTop: 4 }}>
                Assets lose value when their rate rises; liabilities are worth less too, which EVE gains. To hedge a tenor, look for the bars furthest from the line: paying fixed at that tenor adds a positive bar there.
              </div>
            </Card>
          </div>
          <div className="row-wrap" style={{ marginTop: 10 }}>
            <Card title="Buckets (present value)" width={v.shocked ? 620 : 460}>
              <SimpleTable headers={v.bucketHeaders} rows={v.bucketRows} testId="eve-buckets" />
            </Card>
            <Card title="By currency" width={v.shocked ? 400 : 260}>
              <SimpleTable headers={v.ccyHeaders} rows={v.ccyRows} />
            </Card>
          </div>
        </>
      )}
    </ToolLayout>
  );
}
