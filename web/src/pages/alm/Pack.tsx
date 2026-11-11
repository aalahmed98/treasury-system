// The monthly ALCO pack: the committee's standard paper assembled from the official figures (the
// bank's saved parameters, never a what-if), with commentary per section and the treasury head's
// sign-off, which makes the commentary final. Prints as the paper itself.
import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ApiError, endpoints } from '../../api/client';
import { useAlmOverview, useFxRisk } from '../../api/queries';
import { useSession } from '../../api/session';
import type { AlcoPack, EwiResult, FundingResult, LcrResult, LiquidityGapResult, NsfrResult, StressResult } from '../../api/types';
import { limitRows, overviewCells, standingTone } from '../../lib/alm';
import { fmtDateLong, fmtLocal, fmtMoneyShort, fmtMonth, fmtPct, signed } from '../../lib/fmt';
import { Badge, Dot, Gauge, StatStrip } from '../../components/ui';
import { SkeletonLines, useToast } from '../../components/feedback';
import { useTool } from '../../alm/state';
import { ewiTone } from './Ewi';

// The pack shows the bank's own figures: no what-if, no rate shock from the bar, no trial hedges.
const OFFICIAL = { params: null, scenario: null, custom_shock_bp: null, custom_shock_buckets: null, hedges: null };

function Note({ pack, section }: { pack: AlcoPack; section: string }) {
  const qc = useQueryClient();
  const toast = useToast();
  const saved = pack.notes[section];
  const [text, setText] = useState(saved?.body ?? '');
  useEffect(() => setText(saved?.body ?? ''), [saved?.body]);
  const final = pack.signoff !== null;
  const save = useMutation({
    mutationFn: () => endpoints.packNote(pack.period, section, text),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ['alm', 'pack'] }),
    onError: (e) => toast({ tone: 'bad', title: 'The commentary was not saved', detail: e instanceof ApiError ? e.message : undefined }),
  });
  return (
    <div className="pack-note">
      <div className="t-label">Commentary</div>
      {final ? (
        <p className="prose">{saved?.body || <span className="t-muted">none</span>}</p>
      ) : (
        <>
          <textarea
            className="input noprint"
            rows={3}
            value={text}
            placeholder="What the committee should take from this section"
            onChange={(e) => setText(e.target.value)}
            onBlur={() => text !== (saved?.body ?? '') && save.mutate()}
            aria-label={`Commentary on ${section}`}
          />
          <p className="print-text prose">{text}</p>
        </>
      )}
      <div className="t-small">{save.isPending ? 'saving…' : saved ? `last edited by ${saved.updated_by ?? 'someone'} at ${fmtLocal(saved.updated_at)}` : final ? '' : 'saved when you leave the box'}</div>
    </div>
  );
}

export function Pack() {
  const { canSeeRisk } = useSession();
  const pack = useQuery({ queryKey: ['alm', 'pack'], queryFn: () => endpoints.pack() });
  const overview = useAlmOverview();
  const lcr = useTool<LcrResult>('lcr', OFFICIAL);
  const nsfr = useTool<NsfrResult>('nsfr', OFFICIAL);
  const ladder = useTool<LiquidityGapResult>('liquidity-gap', OFFICIAL);
  const ewi = useTool<EwiResult>('ewi', OFFICIAL);
  const stress = useTool<StressResult>('stress', OFFICIAL);
  const scenarios = useQuery({ queryKey: ['alm', 'compare', 'pack'], queryFn: () => endpoints.almCompare({ pack: true }), staleTime: 30_000 });
  const funding = useTool<FundingResult>('funding', OFFICIAL);
  const fx = useFxRisk(canSeeRisk);
  const p = pack.data;
  const o = overview.data;
  const ready = !!p && !!o && !!stress.data && !!ladder.data && !!ewi.data;
  if (!p || !o) {
    return (
      <div className="alm-results">
        <SkeletonLines n={8} />
      </div>
    );
  }
  const month = fmtMonth(p.period, true);
  const cells = overviewCells(o).filter((c) => ['lcr', 'nsfr', 'stress', 'funding'].includes(c.tool));
  return (
    <div className="alm-layout single" data-ready={ready ? '1' : undefined}>
      <section className="alm-results pack" data-testid="alco-pack">
        <div className="row" style={{ marginBottom: 4 }}>
          <h1 className="t-title">ALCO pack, {month}</h1>
          {p.signoff ? (
            <Badge tone="good" dot>
              signed off
            </Badge>
          ) : (
            <Badge tone="warn" dot>
              draft
            </Badge>
          )}
          <span className="grow" />
          <button type="button" className="btn btn-sm noprint" onClick={() => window.print()}>
            Print
          </button>
        </div>
        <div className="t-small" style={{ marginBottom: 12 }}>
          Figures as of {fmtDateLong(o.as_of)} on the bank's parameters v{o.params_version}, in {o.reporting_ccy}. The commentary is the committee's; the figures are computed, never typed.
        </div>

        <PackSection n={1} title="Headline">
          <StatStrip cells={cells.map((c) => ({ ...c, onClick: undefined }))} hero />
          <Note pack={p} section="headline" />
        </PackSection>

        <PackSection n={2} title="Limits">
          <table className="table limits">
            <tbody>
              {limitRows(o).map((l) => (
                <tr key={l.metric}>
                  <td style={{ width: 18 }}>
                    <Dot tone={standingTone(l.standing)} />
                  </td>
                  <td>{l.metric}</td>
                  <td className="num t-medium">{l.value}</td>
                  <td className="t-small">{l.limit}</td>
                  <td>
                    <Gauge value={l.gaugeValue} limit={l.gaugeLimit} trigger={l.gaugeTrigger} max={l.gaugeMax} floor={l.floor} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <Note pack={p} section="limits" />
        </PackSection>

        <PackSection n={3} title="Liquidity">
          <div className="kv" style={{ maxWidth: 560 }}>
            <span className="k">LCR</span>
            <span className="v t-medium">{fmtPct(lcr.data?.total.ratio ?? null)}</span>
            <span className="k">NSFR</span>
            <span className="v t-medium">{fmtPct(nsfr.data?.ratio ?? null)}</span>
            <span className="k">Survival under the liquidity stress</span>
            <span className="v">{stress.data ? (stress.data.liquidity.survival_days === null ? 'beyond 90 days' : `${stress.data.liquidity.survival_days} days`) : '…'}</span>
            <span className="k">Contingency funding plan</span>
            <span className="v">{ewi.data ? ewi.data.stage.toLowerCase() : '…'}</span>
          </div>
          {ladder.data && (
            <table className="table" style={{ marginTop: 10 }}>
              <thead>
                <tr>
                  <th>Behavioural ladder</th>
                  {ladder.data.behavioural.buckets.slice(0, 6).map((b) => (
                    <th key={b.label} className="num">
                      {b.label}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                <tr>
                  <td>Cumulative gap</td>
                  {ladder.data.behavioural.buckets.slice(0, 6).map((b) => (
                    <td key={b.label} className={`num ${b.cumulative < 0 ? 't-bad' : ''}`}>
                      {signed(b.cumulative)}
                    </td>
                  ))}
                </tr>
                <tr>
                  <td>% of assets</td>
                  {ladder.data.behavioural.buckets.slice(0, 6).map((b) => (
                    <td key={b.label} className={`num ${b.breach ? 't-bad' : ''}`}>
                      {b.cumulative_pct_assets.toFixed(1)}%
                    </td>
                  ))}
                </tr>
              </tbody>
            </table>
          )}
          {ewi.data && (
            <div className="row-wrap" style={{ marginTop: 8, gap: '4px 14px' }}>
              {ewi.data.indicators.map((i) => (
                <span key={i.key} className="row" style={{ gap: 5 }}>
                  <Dot tone={ewiTone(i.status)} />
                  <span className="t-small">{i.label}</span>
                </span>
              ))}
            </div>
          )}
          <Note pack={p} section="liquidity" />
        </PackSection>

        <PackSection n={4} title="Interest rate risk">
          <div className="t-small" style={{ marginBottom: 6 }}>
            Cumulative repricing gap at one year {o.gap_1y_pct >= 0 ? '+' : ''}
            {o.gap_1y_pct.toFixed(1)}% of assets.
          </div>
          {stress.data && (
            <table className="table">
              <thead>
                <tr>
                  <th>Scenario</th>
                  <th className="num">ΔNII 12m</th>
                  <th className="num">ΔEVE</th>
                  <th className="num">EVE loss, % Tier 1</th>
                </tr>
              </thead>
              <tbody>
                {[...stress.data.rate_scenarios, ...stress.data.uniform_200].map((s) => (
                  <tr key={s.label}>
                    <td>{s.label}</td>
                    <td className={`num ${s.nii_delta < 0 ? 't-bad' : ''}`}>{signed(s.nii_delta)}</td>
                    <td className={`num ${s.eve_delta < 0 ? 't-bad' : ''}`}>{signed(s.eve_delta)}</td>
                    <td className={`num ${s.eve_breach ? 't-bad t-medium' : ''}`}>{s.eve_loss_pct_tier1 !== null ? `${s.eve_loss_pct_tier1.toFixed(1)}%` : '—'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {(scenarios.data?.length ?? 0) > 0 && (
            <div data-testid="pack-scenarios" style={{ marginTop: 10 }}>
              <div className="t-small" style={{ marginBottom: 4 }}>
                Simulations the desk has put in the pack. They are not official figures: each is a saved scenario with its own shock, assumptions and trial hedges, run on today's balance sheet.
              </div>
              <table className="table">
                <thead>
                  <tr>
                    <th>Simulation</th>
                    <th>What it is</th>
                    <th className="num">ΔNII 12m</th>
                    <th className="num">ΔEVE</th>
                    <th className="num">EVE loss, % Tier 1</th>
                  </tr>
                </thead>
                <tbody>
                  {scenarios.data!.map((r) => (
                    <tr key={r.id ?? r.name}>
                      <td>
                        {r.name}
                        <span className="t-small"> · {r.author}</span>
                      </td>
                      <td>{r.row ? r.summary : (r.error ?? 'could not be run')}</td>
                      <td className={`num ${r.row && r.row.nii_delta < 0 ? 't-bad' : ''}`}>{r.row ? signed(r.row.nii_delta) : ''}</td>
                      <td className={`num ${r.row && r.row.eve_delta < 0 ? 't-bad' : ''}`}>{r.row ? signed(r.row.eve_delta) : ''}</td>
                      <td className={`num ${r.row?.eve_breach ? 't-bad t-medium' : ''}`}>{r.row && r.row.eve_loss_pct_tier1 !== null ? `${r.row.eve_loss_pct_tier1.toFixed(1)}%` : r.row ? '—' : ''}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          <Note pack={p} section="rates" />
        </PackSection>

        <PackSection n={5} title="Income">
          <div className="kv" style={{ maxWidth: 560 }}>
            <span className="k">NII, next 12 months</span>
            <span className="v t-medium">{fmtMoneyShort(o.nii_12m)}</span>
            <span className="k">NIM</span>
            <span className="v">{fmtPct(o.nim, 2)}</span>
            <span className="k">Month to date against budget</span>
            <span className="v">
              {fmtMoneyShort(o.nii_mtd)} / {fmtMoneyShort(o.budget_mtd)}
            </span>
            <span className="k">Year to date against budget</span>
            <span className="v">
              {fmtMoneyShort(o.nii_ytd)} / {fmtMoneyShort(o.budget_ytd)}
            </span>
            {o.ratios && (
              <>
                <span className="k">Cost of funds · asset yield · net spread</span>
                <span className="v">
                  {fmtPct(o.ratios.cost_of_funds, 2)} · {fmtPct(o.ratios.asset_yield, 2)} · {fmtPct(o.ratios.net_spread, 2)}
                </span>
              </>
            )}
          </div>
          <Note pack={p} section="income" />
        </PackSection>

        <PackSection n={6} title="Funding">
          <div className="kv" style={{ maxWidth: 560 }}>
            <span className="k">Ten largest depositors</span>
            <span className="v">{fmtPct(funding.data?.top10_pct ?? o.top10_pct)}</span>
            <span className="k">Largest depositor</span>
            <span className="v">{fmtPct(funding.data?.top1_pct ?? null)}</span>
            <span className="k">Wholesale funding</span>
            <span className="v">{fmtPct(o.wholesale_pct)}</span>
            {o.ratios && (
              <>
                <span className="k">Loans to deposits</span>
                <span className="v">{fmtPct(o.ratios.loan_to_deposit)}</span>
              </>
            )}
          </div>
          <Note pack={p} section="funding" />
        </PackSection>

        {canSeeRisk && (
          <PackSection n={7} title="FX dealing">
            {fx.data ? (
              <div className="kv" style={{ maxWidth: 560 }}>
                <span className="k">Net open position</span>
                <span className="v">{fmtMoneyShort(fx.data.nop_total)}</span>
                <span className="k">Value at risk, 1 day 99%</span>
                <span className="v">{fmtMoneyShort(fx.data.var.var99)}</span>
                <span className="k">P&L month to date</span>
                <span className="v">{signed(fx.data.pnl.mtd)}</span>
                <span className="k">Limits in breach today</span>
                <span className="v">{fx.data.limits.filter((l) => l.status === 'BREACH').length}</span>
              </div>
            ) : (
              <SkeletonLines n={3} />
            )}
            <Note pack={p} section="fx" />
          </PackSection>
        )}

        <SignOff pack={p} month={month} />
      </section>
    </div>
  );
}

function PackSection({ n, title, children }: { n: number; title: string; children: React.ReactNode }) {
  return (
    <section className="pack-section">
      <h2 className="t-heading">
        <span className="t-muted">{n}</span> {title}
      </h2>
      {children}
    </section>
  );
}

function SignOff({ pack, month }: { pack: AlcoPack; month: string }) {
  const qc = useQueryClient();
  const toast = useToast();
  const [note, setNote] = useState('');
  const [confirming, setConfirming] = useState(false);
  const sign = useMutation({
    mutationFn: () => endpoints.packSign(pack.period, note.trim()),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['alm', 'pack'] });
      toast({ tone: 'good', title: `The ${month} pack is signed off`, detail: 'Its commentary is now final.' });
    },
    onError: (e) => toast({ tone: 'bad', title: 'The sign-off failed', detail: e instanceof ApiError ? e.message : undefined }),
  });
  return (
    <section className="pack-section save-box" data-testid="pack-signoff">
      <div className="t-heading">Sign-off</div>
      {pack.signoff ? (
        <p className="prose">
          Signed off by {pack.signoff.signed_by ?? 'the treasury head'} on {fmtLocal(pack.signoff.signed_at)}
          {pack.signoff.note ? `: ${pack.signoff.note}` : '.'}
        </p>
      ) : pack.can_sign ? (
        <div className="stack">
          <input className="input" placeholder="note for the record, optional" value={note} onChange={(e) => setNote(e.target.value)} aria-label="Sign-off note" style={{ maxWidth: 520 }} />
          <div className="row">
            {confirming ? (
              <button type="button" className="btn btn-primary" disabled={sign.isPending} onClick={() => sign.mutate()}>
                Confirm: sign off {month}
              </button>
            ) : (
              <button type="button" className="btn btn-primary" onClick={() => setConfirming(true)}>
                Sign off {month}
              </button>
            )}
            <span className="t-small prose">Once signed, the commentary in every section is final.</span>
          </div>
        </div>
      ) : (
        <p className="t-secondary">Awaiting the treasury head's sign-off.</p>
      )}
    </section>
  );
}
