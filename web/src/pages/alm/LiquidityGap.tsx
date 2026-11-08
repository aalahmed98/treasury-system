// The liquidity maturity ladder: when cash comes in and goes out, contractually and under the
// bank's behavioural assumptions, with the floors on the cumulative gap.
import { useState } from 'react';
import type { LiquidityGapResult, LiquidityLadder } from '../../api/types';
import { Chip } from '../../components/ui';
import { Bars } from '../../components/charts';
import { useAlm, useTool } from '../../alm/state';
import { AssumptionsHeader, Group, Headline, Pct, ResultsHeader, Row, ToolLayout } from '../../alm/tool';
import { whenLine } from '../../alm/views';
import { fmtMoneyShort, signed } from '../../lib/fmt';

const SEGMENTS: [string, string][] = [
  ['RETAIL', 'Retail'],
  ['SME', 'Small business'],
  ['CORPORATE', 'Corporate'],
  ['FI', 'Banks'],
];
const BUCKET_ORDER = ['ON', '1W', '1M', '3M', '6M', '1Y', '2Y', '5Y', '10Y+'];

function Assumptions() {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const g = working.liquidity_gap;
  const b = bank.liquidity_gap;
  const floors = Object.keys(g.cumulative_floor_pct_assets).sort((a, b) => BUCKET_ORDER.indexOf(a) - BUCKET_ORDER.indexOf(b));
  return (
    <>
      <AssumptionsHeader />
      <Group title="Stable share of deposits" help="The share of current, savings and operational deposits that stays, run off over their behavioural life; the rest is assumed to leave overnight.">
        {SEGMENTS.map(([s, label]) => (
          <Row key={s} label={label} cover={`liquidity_gap.stable_share_pct.${s}`}>
            <Pct value={g.stable_share_pct[s] ?? 50} base={b.stable_share_pct[s] ?? 50} onChange={(v) => edit((p) => void (p.liquidity_gap.stable_share_pct[s] = v))} />
          </Row>
        ))}
      </Group>
      <Group title="Commitments" help="The share of undrawn credit lines assumed drawn within a month.">
        <Row label="Drawn within a month" cover="liquidity_gap.commitment_draw_pct">
          <Pct value={g.commitment_draw_pct} base={b.commitment_draw_pct} onChange={(v) => edit((p) => void (p.liquidity_gap.commitment_draw_pct = v))} />
        </Row>
      </Group>
      <Group title="Floors on the cumulative gap" help="The behavioural cumulative gap may not fall below minus this share of total assets." count={floors.length}>
        {floors.map((k) => (
          <Row key={k} label={`To ${k}`}>
            <Pct value={g.cumulative_floor_pct_assets[k]} base={b.cumulative_floor_pct_assets[k]} onChange={(v) => edit((p) => void (p.liquidity_gap.cumulative_floor_pct_assets[k] = v))} />
          </Row>
        ))}
      </Group>
      <p className="t-small prose" style={{ marginTop: 10 }}>
        Liquid securities count as sold or repoed on the first day after their LCR haircut; equity and other assets never fall due; behavioural lives come from the gap tool.
      </p>
    </>
  );
}

function LadderTable({ l, ccy }: { l: LiquidityLadder; ccy: string }) {
  return (
    <table className="table" data-testid="liquidity-ladder">
      <thead>
        <tr>
          <th>Bucket</th>
          <th className="num">In</th>
          <th className="num">Out</th>
          <th className="num">Gap</th>
          <th className="num">Cumulative</th>
          <th className="num">% of assets</th>
          <th className="num">Floor</th>
        </tr>
      </thead>
      <tbody>
        {l.buckets.map((b) => (
          <tr key={b.label}>
            <td className="t-medium">{b.label}</td>
            <td className="num">{fmtMoneyShort(b.inflows)}</td>
            <td className="num">{fmtMoneyShort(b.outflows)}</td>
            <td className={`num ${b.gap < 0 ? 't-bad' : ''}`}>{signed(b.gap)}</td>
            <td className={`num t-medium ${b.cumulative < 0 ? 't-bad' : ''}`}>{signed(b.cumulative)}</td>
            <td className={`num ${b.breach ? 't-bad t-medium' : ''}`}>{b.cumulative_pct_assets.toFixed(1)}%</td>
            <td className="num t-small">{b.floor_pct !== null ? `${b.floor_pct.toFixed(0)}%${b.breach ? ' · breach' : ''}` : '—'}</td>
          </tr>
        ))}
      </tbody>
      <tfoot>
        <tr>
          <td colSpan={7} className="t-small">
            in {ccy}, principal only
          </td>
        </tr>
      </tfoot>
    </table>
  );
}

export function LiquidityGap() {
  const q = useTool<LiquidityGapResult>('liquidity-gap');
  const [view, setView] = useState<'behavioural' | 'contractual'>('behavioural');
  const r = q.data;
  const l = r ? r[view] : null;
  const oneMonth = l?.buckets.find((b) => b.label === '1M');
  const breach = (r?.behavioural.buckets ?? []).some((b) => b.breach);
  return (
    <ToolLayout assumptions={<Assumptions />} ready={!!r} busy={q.isFetching && !!r}>
      <ResultsHeader title="Liquidity maturity ladder" loading={q.isFetching} error={q.error?.message} onRetry={() => void q.refetch()} />
      {r && l && (
        <>
          <Headline
            value={`${(oneMonth?.cumulative_pct_assets ?? 0) >= 0 ? '+' : ''}${(oneMonth?.cumulative_pct_assets ?? 0).toFixed(1)}%`}
            standing={breach ? 'breach' : r.behavioural.first_negative ? 'watch' : 'comfortable'}
            line1={`cumulative gap to one month, ${view}, as a share of total assets of ${fmtMoneyShort(r.total_assets)}${l.first_negative ? ` · first negative at ${l.first_negative}` : ' · positive in every bucket'}`}
            line2={whenLine(r.as_of, r.params_version !== null ? `bank parameters v${r.params_version}` : 'unsaved what-if', r.reporting_ccy)}
            testId="headline"
          />
          {r.breaches.length > 0 && <div className="t-bad" style={{ marginBottom: 8 }}>{r.breaches.join('; ')}</div>}
          <div className="chips" role="radiogroup" aria-label="View" style={{ marginBottom: 8 }}>
            <Chip selected={view === 'behavioural'} onClick={() => setView('behavioural')}>
              Behavioural
            </Chip>
            <Chip selected={view === 'contractual'} onClick={() => setView('contractual')}>
              Contractual
            </Chip>
            <span className="t-small" style={{ marginLeft: 8 }}>
              {view === 'contractual' ? 'what the contracts say: every current account repayable today' : 'what the bank expects: stable deposits stay, liquid securities can be sold'}
            </span>
          </div>
          <div className="panel-row two">
            <div className="panel">
              <div className="t-heading">Cumulative gap</div>
              <Bars data={l.buckets.map((b) => [b.label, b.cumulative])} color="var(--s1)" fmt={signed} labels="extremes" height={160} />
            </div>
            <div className="panel">
              <div className="t-heading">Gap per bucket</div>
              <Bars data={l.buckets.map((b) => [b.label, b.gap])} color="var(--s3)" fmt={signed} labels="extremes" height={160} />
            </div>
          </div>
          <div className="panel" style={{ marginTop: 12 }}>
            <LadderTable l={l} ccy={r.reporting_ccy} />
          </div>
          <div className="panel" style={{ marginTop: 12 }}>
            <div className="t-heading">Cumulative gap by currency</div>
            <table className="table">
              <thead>
                <tr>
                  <th>Ccy</th>
                  {l.buckets.map((b) => (
                    <th key={b.label} className="num">
                      {b.label}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {l.by_ccy.map((c) => (
                  <tr key={c.ccy}>
                    <td className="t-medium">{c.ccy}</td>
                    {c.cumulative.map((v, i) => (
                      <td key={i} className={`num ${v < 0 ? 't-bad' : ''}`}>
                        {signed(v)}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
    </ToolLayout>
  );
}
