// The bank's parameter set: the active version, its history, and an editor that saves a new
// version. This is the one write that moves every user's limits, so the page says exactly what
// will change before it lets anyone save.
import { useMemo, useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ApiError, endpoints } from '../../api/client';
import type { AlmParams } from '../../api/types';
import { Badge } from '../../components/ui';
import { useToast } from '../../components/feedback';
import { useAlm } from '../../alm/state';
import { Bp, ChangeList, Group, Num, Pct, Row, diffParams, revertPath } from '../../alm/tool';
import { BetaRows } from './Nii';
import { fmtLocal, nice } from '../../lib/fmt';

export function Setup() {
  const { params, bank, overrides, draft, setDraft, note, setNote } = useAlm();
  const qc = useQueryClient();
  const toast = useToast();
  const [error, setError] = useState<string | null>(null);
  const save = useMutation({
    mutationFn: ({ p, n }: { p: AlmParams; n: string }) => endpoints.almSave(p, n),
    onSuccess: (v) => {
      setError(null);
      setDraft(null);
      setNote('');
      void qc.invalidateQueries({ queryKey: ['alm'] });
      toast({ tone: 'good', title: `Parameters saved as version ${v.version}`, detail: 'Every open client is picking it up now; the tools recompute against it.' });
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'the save failed; nothing was changed'),
  });
  const changes = useMemo(() => (draft && bank ? diffParams(bank, draft) : []), [draft, bank]);
  const r = params.data;
  if (!r || !bank) {
    return (
      <div className="alm-results">
        {params.isError ? (
          <div className="row">
            <span className="t-bad">The parameters could not be loaded.</span>
            <button type="button" className="btn btn-sm" onClick={() => void params.refetch()}>
              Try again
            </button>
          </div>
        ) : (
          <p className="t-muted">loading the bank's parameters…</p>
        )}
      </div>
    );
  }
  const next = r.active.version + 1;
  const edit = (fn: (p: AlmParams) => void) =>
    setDraft(
      (() => {
        if (!draft) return draft;
        const n = structuredClone(draft);
        fn(n);
        return n;
      })(),
    );
  return (
    <div className="alm-layout single" data-ready="1">
      <section className="alm-results">
        <div className="setup-grid">
          <div>
            <div className="row" style={{ alignItems: 'baseline', flexWrap: 'wrap' }}>
              <h1 className="t-title">Parameters, version {r.active.version}</h1>
              <span className="t-small">
                saved {fmtLocal(r.active.created_at)} by {r.active.created_by ?? 'the application'}
              </span>
            </div>
            {r.active.note && <div className="t-small">{r.active.note}</div>}

            {!draft ? (
              <div className="setup-start">
                <div className="t-heading">New version</div>
                <p className="t-secondary prose">A draft starts from the active version, or from the what-if you have been editing on the tools. Saving it creates an audited version that every open client picks up.</p>
                <div className="row" style={{ flexWrap: 'wrap' }}>
                  <button type="button" className="btn btn-primary" onClick={() => setDraft(structuredClone(bank))}>
                    Start from version {r.active.version}
                  </button>
                  {overrides && (
                    <button type="button" className="btn" onClick={() => setDraft(structuredClone(overrides))}>
                      Start from the current what-if
                    </button>
                  )}
                </div>
              </div>
            ) : (
              <>
                <div className="save-box" data-testid="save-box">
                  <div className="row">
                    <span className="t-heading">What changes in version {next}</span>
                    <span className="grow" />
                    <button type="button" className="btn btn-ghost btn-sm" onClick={() => setDraft(null)} title="drop this draft; the active version is untouched">
                      Discard draft
                    </button>
                  </div>
                  <ChangeList changes={changes} onRevert={(c) => edit((p) => revertPath(p, c))} empty={`Nothing differs from version ${r.active.version} yet. Edit a value below and it appears here.`} />
                  <label className="field" style={{ marginTop: 10 }}>
                    <span className="t-label">Why this version</span>
                    <input className="input" placeholder="e.g. CBB LM-11 calibration of the run-off rates" value={note} onChange={(e) => setNote(e.target.value)} aria-label="Version note" />
                  </label>
                  <div className="row" style={{ marginTop: 10 }}>
                    <button type="button" className="btn btn-primary" disabled={!note.trim() || changes.length === 0 || save.isPending} onClick={() => save.mutate({ p: draft, n: note.trim() })} title={changes.length === 0 ? 'nothing has changed yet' : !note.trim() ? 'a note is required' : undefined}>
                      {save.isPending ? 'Saving…' : `Save as version ${next}`}
                    </button>
                    <span className="t-small prose">
                      {changes.length === 0 ? 'The button wakes once something differs and the note is filled in.' : `Every user's limits and triggers move to version ${next} the moment you save. The change is audited with your name and this note.`}
                    </span>
                  </div>
                  {error && (
                    <div className="notice bad" role="alert" style={{ marginTop: 6 }}>
                      {error}
                    </div>
                  )}
                </div>
                <DraftEditor d={draft} bank={bank} onChange={edit} />
              </>
            )}
          </div>
          <aside>
            <div className="t-heading" style={{ marginBottom: 6 }}>
              Versions
            </div>
            <div className="versions">
              {r.history.map((v) => (
                <div key={v.version} className="version">
                  <span className="t-medium">v{v.version}</span>
                  {v.is_active && <Badge tone="good">active</Badge>}
                  <span className="t-small">{fmtLocal(v.created_at)}</span>
                  <span className="t-small">{v.created_by ?? 'application'}</span>
                  {v.note && <span className="t-secondary">{v.note}</span>}
                </div>
              ))}
            </div>
          </aside>
        </div>
      </section>
    </div>
  );
}

function DraftEditor({ d, bank: b, onChange }: { d: AlmParams; bank: AlmParams; onChange: (fn: (p: AlmParams) => void) => void }) {
  const BUCKETS = ['ON', '1W', '1M', '3M', '6M', '1Y', '2Y', '5Y', '10Y+'];
  const gapKeys = Object.keys(d.gap.cumulative_gap_limit_pct_assets).sort((a, b) => BUCKETS.indexOf(a) - BUCKETS.indexOf(b));
  return (
    <div className="draft-grid" data-testid="draft-editor">
      <div>
        <Group title="General" />
        <Row label="Reporting currency">
          <input className="input input-sm" style={{ width: 64 }} maxLength={3} value={d.reporting_ccy} onChange={(e) => onChange((p) => void (p.reporting_ccy = e.target.value.toUpperCase()))} aria-label="Reporting currency" />
        </Row>
        <Row label="Tier 1 capital">
          <Num value={d.tier1_capital} base={b.tier1_capital} min={1} max={1e12} step={1e6} width={120} onChange={(v) => onChange((p) => void (p.tier1_capital = v))} />
        </Row>
        <Group title="LCR thresholds and haircuts" />
        <Row label="Regulatory minimum">
          <Pct value={d.lcr.minimum} base={b.lcr.minimum} max={500} onChange={(v) => onChange((p) => void (p.lcr.minimum = v))} />
        </Row>
        <Row label="Internal minimum">
          <Pct value={d.lcr.internal_minimum} base={b.lcr.internal_minimum} max={500} onChange={(v) => onChange((p) => void (p.lcr.internal_minimum = v))} />
        </Row>
        <Row label="Early warning">
          <Pct value={d.lcr.early_warning} base={b.lcr.early_warning} max={500} onChange={(v) => onChange((p) => void (p.lcr.early_warning = v))} />
        </Row>
        {['L1', 'L2A', 'L2B'].map((level) => (
          <Row key={level} label={`${level} haircut`}>
            <Pct value={d.lcr.haircuts[level] ?? 0} base={b.lcr.haircuts[level] ?? 0} onChange={(v) => onChange((p) => void (p.lcr.haircuts[level] = v))} />
          </Row>
        ))}
        <Row label="Level 2 cap">
          <Pct value={d.lcr.cap_l2_total_pct} base={b.lcr.cap_l2_total_pct} max={99} onChange={(v) => onChange((p) => void (p.lcr.cap_l2_total_pct = v))} />
        </Row>
        <Row label="Level 2B cap">
          <Pct value={d.lcr.cap_l2b_pct} base={b.lcr.cap_l2b_pct} max={99} onChange={(v) => onChange((p) => void (p.lcr.cap_l2b_pct = v))} />
        </Row>
        <Row label="Inflow cap">
          <Pct value={d.lcr.inflow_cap_pct} base={b.lcr.inflow_cap_pct} onChange={(v) => onChange((p) => void (p.lcr.inflow_cap_pct = v))} />
        </Row>
        <Group title="LCR run-off and inflow rates" />
        {Object.entries(d.lcr.outflow_rates).map(([k, v]) => (
          <Row key={k} label={nice(k)}>
            <Pct value={v} base={b.lcr.outflow_rates[k]} onChange={(x) => onChange((p) => void (p.lcr.outflow_rates[k] = x))} />
          </Row>
        ))}
        {Object.entries(d.lcr.inflow_rates).map(([k, v]) => (
          <Row key={k} label={nice(k)}>
            <Pct value={v} base={b.lcr.inflow_rates[k]} onChange={(x) => onChange((p) => void (p.lcr.inflow_rates[k] = x))} />
          </Row>
        ))}
        <Group title="Funding concentration limits" />
        <Row label="Largest depositor">
          <Pct value={d.funding.top1_limit_pct} base={b.funding.top1_limit_pct} onChange={(v) => onChange((p) => void (p.funding.top1_limit_pct = v))} />
        </Row>
        <Row label="Ten largest depositors">
          <Pct value={d.funding.top10_limit_pct} base={b.funding.top10_limit_pct} onChange={(v) => onChange((p) => void (p.funding.top10_limit_pct = v))} />
        </Row>
        <Row label="Wholesale funding">
          <Pct value={d.funding.wholesale_share_limit_pct} base={b.funding.wholesale_share_limit_pct} onChange={(v) => onChange((p) => void (p.funding.wholesale_share_limit_pct = v))} />
        </Row>
      </div>
      <div>
        <Group title="NSFR thresholds and factors" />
        <div className="t-small" style={{ marginBottom: 4 }}>
          factors by time to maturity: under 6 months · 6 months to a year · a year or more
        </div>
        <Row label="Regulatory minimum">
          <Pct value={d.nsfr.minimum} base={b.nsfr.minimum} max={500} onChange={(v) => onChange((p) => void (p.nsfr.minimum = v))} />
        </Row>
        <Row label="Internal minimum">
          <Pct value={d.nsfr.internal_minimum} base={b.nsfr.internal_minimum} max={500} onChange={(v) => onChange((p) => void (p.nsfr.internal_minimum = v))} />
        </Row>
        <div className="factor-grid" style={{ marginTop: 4 }}>
          {[...Object.entries(d.nsfr.asf).map(([k, f]) => ['asf', k, f] as const), ...Object.entries(d.nsfr.rsf).map(([k, f]) => ['rsf', k, f] as const)].map(([which, k, f]) => (
            <div key={`${which}-${k}`} style={{ display: 'contents' }}>
              <span className="k">{nice(k)}</span>
              {f.map((v, i) => (
                <Num key={i} value={v} base={b.nsfr[which][k]?.[i]} min={0} max={100} step={0.5} decimals={1} width={64} onChange={(x) => onChange((p) => void (p.nsfr[which][k][i] = x))} />
              ))}
            </div>
          ))}
        </div>
        <Group title="Gap, EVE and NII limits" />
        {gapKeys.map((k) => (
          <Row key={k} label={`Cumulative gap ${k} (% assets)`}>
            <Pct value={d.gap.cumulative_gap_limit_pct_assets[k]} base={b.gap.cumulative_gap_limit_pct_assets[k]} onChange={(v) => onChange((p) => void (p.gap.cumulative_gap_limit_pct_assets[k] = v))} />
          </Row>
        ))}
        <Row label="EVE loss (% of Tier 1)">
          <Pct value={d.gap.eve_limit_pct_tier1} base={b.gap.eve_limit_pct_tier1} onChange={(v) => onChange((p) => void (p.gap.eve_limit_pct_tier1 = v))} />
        </Row>
        <Row label="NII sensitivity (%)">
          <Pct value={d.gap.nii_sensitivity_limit_pct} base={b.gap.nii_sensitivity_limit_pct} onChange={(v) => onChange((p) => void (p.gap.nii_sensitivity_limit_pct = v))} />
        </Row>
        {Object.entries(d.gap.nmd_behavioural_months).map(([k, v]) => (
          <Row key={k} label={`${nice(k)} behavioural months`}>
            <Num value={v} base={b.gap.nmd_behavioural_months[k]} min={1} max={240} step={1} onChange={(x) => onChange((p) => void (p.gap.nmd_behavioural_months[k] = x))} />
          </Row>
        ))}
        <Group title="Stress shocks (bp)" />
        <div className="t-small" style={{ marginBottom: 4 }}>
          per currency: parallel · short end · long end
        </div>
        <div className="factor-grid">
          {Object.entries(d.stress.shocks).map(([ccy, s]) => (
            <div key={ccy} style={{ display: 'contents' }}>
              <span className="k">{ccy}</span>
              {(['parallel', 'short', 'long'] as const).map((f) => (
                <Num key={f} value={s[f]} base={b.stress.shocks[ccy]?.[f]} min={0} max={2000} step={5} width={64} onChange={(x) => onChange((p) => void (p.stress.shocks[ccy][f] = x))} />
              ))}
            </div>
          ))}
        </div>
        <Row label="Post-shock floor">
          <Bp value={d.stress.floor_bp} base={b.stress.floor_bp} onChange={(v) => onChange((p) => void (p.stress.floor_bp = v))} />
        </Row>
        <Row label="Floor rises per year (bp)">
          <Num value={d.stress.floor_slope_bp_per_year} base={b.stress.floor_slope_bp_per_year} min={0} max={50} step={1} onChange={(v) => onChange((p) => void (p.stress.floor_slope_bp_per_year = v))} />
        </Row>
        <Row label="Liquidity run-off multiplier">
          <Num value={d.stress.liquidity.outflow_multiplier} base={b.stress.liquidity.outflow_multiplier} min={1} max={5} step={0.05} decimals={2} onChange={(v) => onChange((p) => void (p.stress.liquidity.outflow_multiplier = v))} />
        </Row>
        <Row label="Extra HQLA haircut">
          <Pct value={d.stress.liquidity.extra_hqla_haircut_pct} base={b.stress.liquidity.extra_hqla_haircut_pct} onChange={(v) => onChange((p) => void (p.stress.liquidity.extra_hqla_haircut_pct = v))} />
        </Row>
        <Row label="Inflow multiplier">
          <Num value={d.stress.liquidity.inflow_multiplier} base={b.stress.liquidity.inflow_multiplier} min={0} max={1} step={0.05} decimals={2} onChange={(v) => onChange((p) => void (p.stress.liquidity.inflow_multiplier = v))} />
        </Row>
        {Object.entries(d.stress.fx_shock_pct).map(([ccy, v]) => (
          <Row key={ccy} label={`FX shock ${ccy}`}>
            <Pct value={v} base={b.stress.fx_shock_pct[ccy]} onChange={(x) => onChange((p) => void (p.stress.fx_shock_pct[ccy] = x))} />
          </Row>
        ))}
        <Row label="NII horizon (months)">
          <Num value={d.nii.horizon_months} base={b.nii.horizon_months} min={1} max={60} step={1} onChange={(v) => onChange((p) => void (p.nii.horizon_months = Math.round(v)))} />
        </Row>
        <Group title="Rate pass-through (beta)" />
        <BetaRows betas={d.nii.betas} base={b.nii.betas} onChange={(k, v) => onChange((p) => void (p.nii.betas[k] = v))} />
        <Row label="Other products (default)">
          <Pct value={d.nii.default_beta * 100} base={b.nii.default_beta * 100} onChange={(v) => onChange((p) => void (p.nii.default_beta = v / 100))} />
        </Row>
        <Group title="Liquidity ladder" />
        {Object.entries(d.liquidity_gap.stable_share_pct).map(([k, v]) => (
          <Row key={k} label={`Stable share, ${nice(k).toLowerCase()} deposits`}>
            <Pct value={v} base={b.liquidity_gap.stable_share_pct[k]} onChange={(x) => onChange((p) => void (p.liquidity_gap.stable_share_pct[k] = x))} />
          </Row>
        ))}
        <Row label="Commitments drawn within a month">
          <Pct value={d.liquidity_gap.commitment_draw_pct} base={b.liquidity_gap.commitment_draw_pct} onChange={(v) => onChange((p) => void (p.liquidity_gap.commitment_draw_pct = v))} />
        </Row>
        {Object.entries(d.liquidity_gap.cumulative_floor_pct_assets).map(([k, v]) => (
          <Row key={k} label={`Gap floor to ${k} (% assets)`}>
            <Pct value={v} base={b.liquidity_gap.cumulative_floor_pct_assets[k]} onChange={(x) => onChange((p) => void (p.liquidity_gap.cumulative_floor_pct_assets[k] = x))} />
          </Row>
        ))}
        <Group title="Early-warning thresholds" />
        <div className="t-small" style={{ marginBottom: 4 }}>
          amber · red
        </div>
        <div className="factor-grid" style={{ gridTemplateColumns: 'minmax(0, 1fr) auto auto' }}>
          {Object.entries(d.ewi.thresholds).map(([k, t]) => (
            <div key={k} style={{ display: 'contents' }}>
              <span className="k">{nice(k)}</span>
              <Num value={t.amber} base={b.ewi.thresholds[k]?.amber} min={-1000} max={1000} step={0.5} decimals={1} width={64} onChange={(x) => onChange((p) => void (p.ewi.thresholds[k].amber = x))} />
              <Num value={t.red} base={b.ewi.thresholds[k]?.red} min={-1000} max={1000} step={0.5} decimals={1} width={64} onChange={(x) => onChange((p) => void (p.ewi.thresholds[k].red = x))} />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
