// The FX limits: the active version and its history for everyone, and for the treasury head a
// draft that saves as a new version once it says what changes and why.
import { useMemo, useState } from 'react';
import { Link } from 'react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ApiError, endpoints } from '../api/client';
import { useRiskLimits } from '../api/queries';
import type { RiskLimits } from '../api/types';
import { fmtLocal, fmtMoneyShort } from '../lib/fmt';
import { Badge } from '../components/ui';
import { Empty, SkeletonLines, useToast } from '../components/feedback';
import { ChangeList, Group, Num, Row, diffParams, revertPath } from '../alm/tool';

const amount = { min: 0, max: 1e12, step: 1_000_000, width: 130 };

export function LimitsPage() {
  const q = useRiskLimits();
  const qc = useQueryClient();
  const toast = useToast();
  const [draft, setDraft] = useState<RiskLimits | null>(null);
  const [note, setNote] = useState('');
  const [error, setError] = useState<string | null>(null);
  const r = q.data;
  const changes = useMemo(() => (draft && r ? diffParams(r.limits, draft) : []), [draft, r]);
  const save = useMutation({
    mutationFn: () => endpoints.saveLimits(draft!, note.trim()),
    onSuccess: (v) => {
      setDraft(null);
      setNote('');
      setError(null);
      void qc.invalidateQueries({ queryKey: ['risk'] });
      toast({ tone: 'good', title: `FX limits saved as version ${v.version}`, detail: 'Every figure on the risk page is now measured against it.' });
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'the save failed; nothing was changed'),
  });
  if (!r) {
    return (
      <div className="page">
        {q.isError ? (
          <Empty tone="warn" title="The limits could not be loaded">
            <button type="button" className="btn btn-sm" onClick={() => void q.refetch()}>
              Try again
            </button>
          </Empty>
        ) : (
          <SkeletonLines n={6} />
        )}
      </div>
    );
  }
  const shown = draft ?? r.limits;
  const edit = (fn: (l: RiskLimits) => void) => {
    if (!draft) return;
    const n = structuredClone(draft);
    fn(n);
    setDraft(n);
  };
  const ro = !draft;
  const next = r.active.version + 1;
  return (
    <div className="page" data-ready="1" data-testid="limits-page">
      <div className="page-title">
        <Link to="/risk" className="t-small">
          FX risk
        </Link>
        <span className="t-small">›</span>
        <h1 className="t-title">Limits, version {r.active.version}</h1>
        <span className="t-small">
          saved {fmtLocal(r.active.created_at)} by {r.active.created_by ?? 'the application'}
        </span>
      </div>
      {r.active.note && <p className="t-secondary prose">{r.active.note}</p>}
      <div className="setup-grid">
        <div>
          {!draft ? (
            r.can_edit ? (
              <div className="setup-start">
                <p className="t-secondary prose">The limits below are what every figure on the risk page is held to. A draft starts from this version; saving it creates an audited version the desk works to at once.</p>
                <button type="button" className="btn btn-primary" onClick={() => setDraft(structuredClone(r.limits))}>
                  Start from version {r.active.version}
                </button>
              </div>
            ) : (
              <p className="t-secondary prose" style={{ marginTop: 12 }}>
                Read only. The treasury head sets the FX limits; the risk page measures every position, loss and exposure against them.
              </p>
            )
          ) : (
            <div className="save-box" data-testid="limits-save-box">
              <div className="row">
                <span className="t-heading">What changes in version {next}</span>
                <span className="grow" />
                <button type="button" className="btn btn-ghost btn-sm" onClick={() => setDraft(null)}>
                  Discard draft
                </button>
              </div>
              <ChangeList changes={changes} onRevert={(c) => edit((l) => revertPath(l, c))} empty={`Nothing differs from version ${r.active.version} yet.`} />
              <label className="field" style={{ marginTop: 10 }}>
                <span className="t-label">Why this version</span>
                <input className="input" placeholder="e.g. board approval of 12 November" value={note} onChange={(e) => setNote(e.target.value)} aria-label="Version note" />
              </label>
              <div className="row" style={{ marginTop: 10 }}>
                <button type="button" className="btn btn-primary" disabled={!note.trim() || changes.length === 0 || save.isPending} onClick={() => save.mutate()}>
                  {save.isPending ? 'Saving…' : `Save as version ${next}`}
                </button>
                <span className="t-small prose">{changes.length === 0 ? 'The button wakes once something differs and the note is filled in.' : `The desk is held to version ${next} the moment you save.`}</span>
              </div>
              {error && (
                <div className="notice bad" role="alert" style={{ marginTop: 6 }}>
                  {error}
                </div>
              )}
            </div>
          )}

          <div className="draft-grid" style={{ marginTop: 8 }}>
            <div>
              <Group title="Net open position, BHD" help="The shorthand aggregate: the larger of all long and all short currency positions, in dinars.">
                <Row label="Overnight, all currencies">
                  <Num value={shown.nop_overnight_total} base={r.limits.nop_overnight_total} {...amount} disabled={ro} onChange={(v) => edit((l) => void (l.nop_overnight_total = v))} />
                </Row>
                <Row label="Intraday, all currencies">
                  <Num value={shown.nop_intraday_total} base={r.limits.nop_intraday_total} {...amount} disabled={ro} onChange={(v) => edit((l) => void (l.nop_intraday_total = v))} />
                </Row>
                <Row label="Any floating currency">
                  <Num value={shown.nop_default_floating} base={r.limits.nop_default_floating} {...amount} disabled={ro} onChange={(v) => edit((l) => void (l.nop_default_floating = v))} />
                </Row>
                <Row label="Any pegged currency">
                  <Num value={shown.nop_default_pegged} base={r.limits.nop_default_pegged} {...amount} disabled={ro} onChange={(v) => edit((l) => void (l.nop_default_pegged = v))} />
                </Row>
                <Row label="One dealer, all currencies">
                  <Num value={shown.dealer_nop} base={r.limits.dealer_nop} {...amount} disabled={ro} onChange={(v) => edit((l) => void (l.dealer_nop = v))} />
                </Row>
                <Row label="Pegged currencies">
                  <input className="input input-sm" style={{ width: 200 }} value={shown.pegged.join(', ')} disabled={ro} onChange={(e) => edit((l) => void (l.pegged = e.target.value.toUpperCase().split(/[\s,]+/).filter(Boolean)))} aria-label="Pegged currencies" />
                </Row>
              </Group>
              <Group title="Losses and value at risk, BHD" help="A stop-loss forces the desk to square off once the loss is reached.">
                <Row label="Stop-loss, one day">
                  <Num value={shown.stop_loss_daily} base={r.limits.stop_loss_daily} {...amount} step={10_000} disabled={ro} onChange={(v) => edit((l) => void (l.stop_loss_daily = v))} />
                </Row>
                <Row label="Stop-loss, month to date">
                  <Num value={shown.stop_loss_monthly} base={r.limits.stop_loss_monthly} {...amount} step={10_000} disabled={ro} onChange={(v) => edit((l) => void (l.stop_loss_monthly = v))} />
                </Row>
                <Row label="Value at risk, 1 day 99%">
                  <Num value={shown.var_limit} base={r.limits.var_limit} {...amount} step={10_000} disabled={ro} onChange={(v) => edit((l) => void (l.var_limit = v))} />
                </Row>
              </Group>
              <Group title="Pricing and watch level" help="A deal further than this from the market mid on its trade date is flagged off-market; a limit used beyond the watch level shows as watch.">
                <Row label="Off-market, spot (bp)">
                  <Num value={shown.off_market_bp_spot} base={r.limits.off_market_bp_spot} min={1} max={1000} disabled={ro} onChange={(v) => edit((l) => void (l.off_market_bp_spot = v))} />
                </Row>
                <Row label="Off-market, forward (bp)">
                  <Num value={shown.off_market_bp_forward} base={r.limits.off_market_bp_forward} min={1} max={1000} disabled={ro} onChange={(v) => edit((l) => void (l.off_market_bp_forward = v))} />
                </Row>
                <Row label="Watch from (% of a limit)">
                  <Num value={shown.warn_pct} base={r.limits.warn_pct} min={1} max={100} suffix="%" disabled={ro} onChange={(v) => edit((l) => void (l.warn_pct = v))} />
                </Row>
              </Group>
            </div>
            <div>
              <Group title="Counterparty lines, BHD" help="Pre-settlement: the most the bank may be owed on unsettled deals. Settlement: the most it may pay away on one value date.">
                <table className="table" data-testid="lines">
                  <thead>
                    <tr>
                      <th>Counterparty</th>
                      <th className="num">Pre-settlement</th>
                      <th className="num">Settlement</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {Object.entries(shown.counterparty_lines).map(([name, line]) => (
                      <tr key={name}>
                        <td>{name}</td>
                        <td className="num">
                          {ro ? fmtMoneyShort(line.pre_settlement) : <Num value={line.pre_settlement} {...amount} width={110} onChange={(v) => edit((l) => void (l.counterparty_lines[name].pre_settlement = v))} />}
                        </td>
                        <td className="num">
                          {ro ? fmtMoneyShort(line.settlement) : <Num value={line.settlement} {...amount} width={110} onChange={(v) => edit((l) => void (l.counterparty_lines[name].settlement = v))} />}
                        </td>
                        <td>
                          {!ro && (
                            <button type="button" className="btn btn-ghost btn-sm" onClick={() => edit((l) => void delete l.counterparty_lines[name])} aria-label={`remove ${name}`}>
                              remove
                            </button>
                          )}
                        </td>
                      </tr>
                    ))}
                    <tr>
                      <td className="t-secondary">Any other counterparty</td>
                      <td className="num">{ro ? fmtMoneyShort(shown.default_line.pre_settlement) : <Num value={shown.default_line.pre_settlement} {...amount} width={110} onChange={(v) => edit((l) => void (l.default_line.pre_settlement = v))} />}</td>
                      <td className="num">{ro ? fmtMoneyShort(shown.default_line.settlement) : <Num value={shown.default_line.settlement} {...amount} width={110} onChange={(v) => edit((l) => void (l.default_line.settlement = v))} />}</td>
                      <td />
                    </tr>
                  </tbody>
                </table>
                {!ro && <AddLine onAdd={(name) => edit((l) => void (l.counterparty_lines[name.trim().toUpperCase()] = { ...l.default_line }))} existing={Object.keys(shown.counterparty_lines)} />}
              </Group>
            </div>
          </div>
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
    </div>
  );
}

function AddLine({ onAdd, existing }: { onAdd: (name: string) => void; existing: string[] }) {
  const [name, setName] = useState('');
  const clash = existing.includes(name.trim().toUpperCase());
  return (
    <div className="row" style={{ marginTop: 6 }}>
      <input className="input input-sm" style={{ width: 220 }} placeholder="counterparty name" value={name} onChange={(e) => setName(e.target.value)} aria-label="New counterparty" />
      <button
        type="button"
        className="btn btn-sm"
        disabled={!name.trim() || clash}
        onClick={() => {
          onAdd(name);
          setName('');
        }}
      >
        Add a line
      </button>
      {clash && <span className="t-small t-warn">already has a line</span>}
    </div>
  );
}
