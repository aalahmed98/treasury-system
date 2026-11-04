// The bank's parameter set: the active version, its history, and an editor that saves a new
// version with a note. Any ALM user may save; the server validates and audits.
import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ApiError, endpoints } from '../../api/client';
import type { AlmParams } from '../../api/types';
import { Badge } from '../../components/ui';
import { useAlm } from '../../alm/state';
import { Bp, Group, Num, Pct, Row } from '../../alm/tool';
import { fmtLocal, nice } from '../../lib/fmt';

export function Setup() {
  const { params, bank, overrides, draft, setDraft, note, setNote } = useAlm();
  const qc = useQueryClient();
  const [saved, setSaved] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const save = useMutation({
    mutationFn: ({ p, n }: { p: AlmParams; n: string }) => endpoints.almSave(p, n),
    onSuccess: (v) => {
      setSaved(v.version);
      setError(null);
      setDraft(null);
      setNote('');
      void qc.invalidateQueries({ queryKey: ['alm'] });
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'the save failed'),
  });
  const r = params.data;
  if (!r || !bank) return <div className="alm-results t-muted">loading…</div>;
  return (
    <div className="alm-layout single" data-ready="1">
      <section className="alm-results">
        <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) 360px', gap: 20 }}>
          <div>
            <div className="row" style={{ alignItems: 'baseline', flexWrap: 'wrap' }}>
              <h1 className="t-title">Parameters, version {r.active.version}</h1>
              <span className="t-small">
                saved {fmtLocal(r.active.created_at)} by {r.active.created_by ?? 'the application'}
              </span>
            </div>
            {r.active.note && <div className="t-small">{r.active.note}</div>}
            <div className="row" style={{ marginTop: 12, flexWrap: 'wrap' }}>
              <span className="t-heading">New version</span>
              <button type="button" className="btn btn-sm" onClick={() => setDraft(structuredClone(bank))}>
                Start from the active version
              </button>
              {overrides && (
                <button type="button" className="btn btn-sm" onClick={() => setDraft(structuredClone(overrides))}>
                  Start from the current what-if
                </button>
              )}
              {draft && (
                <button type="button" className="btn btn-sm" onClick={() => setDraft(null)}>
                  Discard
                </button>
              )}
            </div>
            {!draft ? (
              <p className="t-small" style={{ marginTop: 6 }}>
                Start a draft from the active version or from the what-if you have been editing on the tools; every number below is then editable. Saving creates a new audited version that every open client picks up.
              </p>
            ) : (
              <>
                <div className="row" style={{ marginTop: 8 }}>
                  <span className="t-secondary">Note</span>
                  <input className="input input-sm" style={{ width: 380 }} placeholder="why this version" value={note} onChange={(e) => setNote(e.target.value)} aria-label="Version note" />
                  <button type="button" className="btn btn-primary btn-sm" disabled={!note.trim() || save.isPending} onClick={() => save.mutate({ p: draft, n: note.trim() })}>
                    Save as a new version
                  </button>
                  {save.isPending && <span className="t-small">saving…</span>}
                  {error && <span className="t-bad">{error}</span>}
                </div>
                <DraftEditor d={draft} onChange={(fn) => setDraft((() => {
                  const next = structuredClone(draft);
                  fn(next);
                  return next;
                })())} />
              </>
            )}
            {saved !== null && !draft && <div className="t-good" style={{ marginTop: 8 }}>saved as version {saved}</div>}
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

function DraftEditor({ d, onChange }: { d: AlmParams; onChange: (fn: (p: AlmParams) => void) => void }) {
  const BUCKETS = ['ON', '1W', '1M', '3M', '6M', '1Y', '2Y', '5Y', '10Y+'];
  const gapKeys = Object.keys(d.gap.cumulative_gap_limit_pct_assets).sort((a, b) => BUCKETS.indexOf(a) - BUCKETS.indexOf(b));
  return (
    <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) minmax(0, 1fr)', gap: 24, marginTop: 8 }} data-testid="draft-editor">
      <div>
        <Group title="General" />
        <Row label="Reporting currency">
          <input className="input input-sm" style={{ width: 64 }} maxLength={3} value={d.reporting_ccy} onChange={(e) => onChange((p) => void (p.reporting_ccy = e.target.value.toUpperCase()))} />
        </Row>
        <Row label="Tier 1 capital">
          <Num value={d.tier1_capital} min={1} max={1e12} step={1e6} width={120} onChange={(v) => onChange((p) => void (p.tier1_capital = v))} />
        </Row>
        <Group title="LCR thresholds, haircuts and caps" />
        <Row label="Regulatory minimum">
          <Pct value={d.lcr.minimum} max={500} onChange={(v) => onChange((p) => void (p.lcr.minimum = v))} />
        </Row>
        <Row label="Internal minimum">
          <Pct value={d.lcr.internal_minimum} max={500} onChange={(v) => onChange((p) => void (p.lcr.internal_minimum = v))} />
        </Row>
        <Row label="Early warning">
          <Pct value={d.lcr.early_warning} max={500} onChange={(v) => onChange((p) => void (p.lcr.early_warning = v))} />
        </Row>
        {['L1', 'L2A', 'L2B'].map((level) => (
          <Row key={level} label={`${level} haircut`}>
            <Pct value={d.lcr.haircuts[level] ?? 0} onChange={(v) => onChange((p) => void (p.lcr.haircuts[level] = v))} />
          </Row>
        ))}
        <Row label="Level 2 cap">
          <Pct value={d.lcr.cap_l2_total_pct} max={99} onChange={(v) => onChange((p) => void (p.lcr.cap_l2_total_pct = v))} />
        </Row>
        <Row label="Level 2B cap">
          <Pct value={d.lcr.cap_l2b_pct} max={99} onChange={(v) => onChange((p) => void (p.lcr.cap_l2b_pct = v))} />
        </Row>
        <Row label="Inflow cap">
          <Pct value={d.lcr.inflow_cap_pct} onChange={(v) => onChange((p) => void (p.lcr.inflow_cap_pct = v))} />
        </Row>
        <Group title="LCR run-off and inflow rates" />
        {Object.entries(d.lcr.outflow_rates).map(([k, v]) => (
          <Row key={k} label={nice(k)}>
            <Pct value={v} onChange={(x) => onChange((p) => void (p.lcr.outflow_rates[k] = x))} />
          </Row>
        ))}
        {Object.entries(d.lcr.inflow_rates).map(([k, v]) => (
          <Row key={k} label={nice(k)}>
            <Pct value={v} onChange={(x) => onChange((p) => void (p.lcr.inflow_rates[k] = x))} />
          </Row>
        ))}
        <Group title="Funding concentration limits" />
        <Row label="Largest depositor">
          <Pct value={d.funding.top1_limit_pct} onChange={(v) => onChange((p) => void (p.funding.top1_limit_pct = v))} />
        </Row>
        <Row label="Ten largest depositors">
          <Pct value={d.funding.top10_limit_pct} onChange={(v) => onChange((p) => void (p.funding.top10_limit_pct = v))} />
        </Row>
        <Row label="Wholesale funding">
          <Pct value={d.funding.wholesale_share_limit_pct} onChange={(v) => onChange((p) => void (p.funding.wholesale_share_limit_pct = v))} />
        </Row>
      </div>
      <div>
        <Group title="NSFR thresholds and factors (< 6M · 6M-1Y · ≥ 1Y)" />
        <Row label="Regulatory minimum">
          <Pct value={d.nsfr.minimum} max={500} onChange={(v) => onChange((p) => void (p.nsfr.minimum = v))} />
        </Row>
        <Row label="Internal minimum">
          <Pct value={d.nsfr.internal_minimum} max={500} onChange={(v) => onChange((p) => void (p.nsfr.internal_minimum = v))} />
        </Row>
        <div className="factor-grid" style={{ marginTop: 4 }}>
          {[...Object.entries(d.nsfr.asf).map(([k, f]) => ['asf', k, f] as const), ...Object.entries(d.nsfr.rsf).map(([k, f]) => ['rsf', k, f] as const)].map(([which, k, f]) => (
            <div key={`${which}-${k}`} style={{ display: 'contents' }}>
              <span className="k">{nice(k)}</span>
              {f.map((v, i) => (
                <Num key={i} value={v} min={0} max={100} step={0.5} decimals={1} width={64} onChange={(x) => onChange((p) => void (p.nsfr[which][k][i] = x))} />
              ))}
            </div>
          ))}
        </div>
        <Group title="Gap, EVE and NII limits" />
        {gapKeys.map((k) => (
          <Row key={k} label={`Cumulative gap ${k} (% assets)`}>
            <Pct value={d.gap.cumulative_gap_limit_pct_assets[k]} onChange={(v) => onChange((p) => void (p.gap.cumulative_gap_limit_pct_assets[k] = v))} />
          </Row>
        ))}
        <Row label="EVE loss (% of Tier 1)">
          <Pct value={d.gap.eve_limit_pct_tier1} onChange={(v) => onChange((p) => void (p.gap.eve_limit_pct_tier1 = v))} />
        </Row>
        <Row label="NII sensitivity (%)">
          <Pct value={d.gap.nii_sensitivity_limit_pct} onChange={(v) => onChange((p) => void (p.gap.nii_sensitivity_limit_pct = v))} />
        </Row>
        {Object.entries(d.gap.nmd_behavioural_months).map(([k, v]) => (
          <Row key={k} label={`${nice(k)} behavioural months`}>
            <Num value={v} min={1} max={240} step={1} onChange={(x) => onChange((p) => void (p.gap.nmd_behavioural_months[k] = x))} />
          </Row>
        ))}
        <Group title="Stress shocks (bp: parallel · short · long)" />
        <div className="factor-grid">
          {Object.entries(d.stress.shocks).map(([ccy, s]) => (
            <div key={ccy} style={{ display: 'contents' }}>
              <span className="k">{ccy}</span>
              {(['parallel', 'short', 'long'] as const).map((f) => (
                <Num key={f} value={s[f]} min={0} max={2000} step={5} width={64} onChange={(x) => onChange((p) => void (p.stress.shocks[ccy][f] = x))} />
              ))}
            </div>
          ))}
        </div>
        <Row label="Post-shock floor">
          <Bp value={d.stress.floor_bp} onChange={(v) => onChange((p) => void (p.stress.floor_bp = v))} />
        </Row>
        <Row label="Floor rises per year (bp)">
          <Num value={d.stress.floor_slope_bp_per_year} min={0} max={50} step={1} onChange={(v) => onChange((p) => void (p.stress.floor_slope_bp_per_year = v))} />
        </Row>
        <Row label="Liquidity run-off multiplier">
          <Num value={d.stress.liquidity.outflow_multiplier} min={1} max={5} step={0.05} decimals={2} onChange={(v) => onChange((p) => void (p.stress.liquidity.outflow_multiplier = v))} />
        </Row>
        <Row label="Extra HQLA haircut">
          <Pct value={d.stress.liquidity.extra_hqla_haircut_pct} onChange={(v) => onChange((p) => void (p.stress.liquidity.extra_hqla_haircut_pct = v))} />
        </Row>
        <Row label="Inflow multiplier">
          <Num value={d.stress.liquidity.inflow_multiplier} min={0} max={1} step={0.05} decimals={2} onChange={(v) => onChange((p) => void (p.stress.liquidity.inflow_multiplier = v))} />
        </Row>
        {Object.entries(d.stress.fx_shock_pct).map(([ccy, v]) => (
          <Row key={ccy} label={`FX shock ${ccy}`}>
            <Pct value={v} onChange={(x) => onChange((p) => void (p.stress.fx_shock_pct[ccy] = x))} />
          </Row>
        ))}
        <Row label="NII horizon (months)">
          <Num value={d.nii.horizon_months} min={1} max={60} step={1} onChange={(v) => onChange((p) => void (p.nii.horizon_months = Math.round(v)))} />
        </Row>
        <Row label="Default deposit beta">
          <Pct value={d.nii.default_beta * 100} onChange={(v) => onChange((p) => void (p.nii.default_beta = v / 100))} />
        </Row>
      </div>
    </div>
  );
}
