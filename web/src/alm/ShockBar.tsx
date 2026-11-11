// The rate shock bar under the ALM tool strip: the one place the shock is chosen. It stays in the
// same spot on every analysis tool, and says so on the tools a rate move does not reach.
import { useCallback, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { endpoints } from '../api/client';
import type { HedgeKind, RateScenario } from '../api/types';
import { SCENARIO_LABEL } from '../lib/alm';
import { Chip, useOutsideClose } from '../components/ui';
import { useAlm } from './state';
import { ScenarioMenu } from './ScenarioMenu';
import { BASEL, Bp, Num } from './tool';
import { HEDGE_HELP, HEDGE_KINDS, TENORS, curveRate, hedgeLabel, hedgesSummary } from './hedge';
import { BUCKETS, SHORTCUTS, customLabel, normaliseBuckets, shapeTitle, shockShape, shockSizes, tenorsLabel, toggleBucket } from './shock';

const OPTIONS: RateScenario[] = ['BASE', ...BASEL, 'CUSTOM'];

/** Nine small bars: the basis points applied at each bucket, up from the middle line or down. */
function Shape({ shape }: { shape: number[] }) {
  const peak = Math.max(1, ...shape.map(Math.abs));
  const w = 7;
  const gap = 3;
  const half = 9;
  return (
    <svg className="shock-shape" width={shape.length * (w + gap) - gap} height={half * 2 + 1} role="img" aria-label={`shock by tenor: ${shapeTitle(shape)}`}>
      <title>{shapeTitle(shape)}</title>
      <line x1={0} x2={shape.length * (w + gap) - gap} y1={half + 0.5} y2={half + 0.5} className="axis" />
      {shape.map((v, i) => {
        const h = Math.abs(v) < 0.5 ? 0 : Math.max(2, (Math.abs(v) / peak) * half);
        return <rect key={BUCKETS[i]} x={i * (w + gap)} y={v >= 0 ? half - h : half + 1} width={w} height={h} rx={1} className={v >= 0 ? 'up' : 'down'} />;
      })}
    </svg>
  );
}

function Tenors() {
  const { shockBuckets, setShockBuckets } = useAlm();
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const ref = useOutsideClose<HTMLDivElement>(open, close);
  const all = shockBuckets.length === 0;
  return (
    <div className="anchor" ref={ref}>
      <button type="button" className="btn btn-sm" onClick={() => setOpen((o) => !o)} aria-expanded={open} aria-haspopup="dialog" title="which tenors the shock moves">
        Tenors: {tenorsLabel(shockBuckets)} ▾
      </button>
      {open && (
        <div className="popup" role="dialog" aria-label="Tenors the shock moves" style={{ left: 0, right: 'auto', minWidth: 390 }}>
          <div className="t-eyebrow" style={{ marginBottom: 6 }}>
            Move the rates at
          </div>
          <div className="chips" data-testid="shock-buckets">
            {BUCKETS.map((b) => (
              <Chip key={b} small outlined selected={all || shockBuckets.includes(b)} onClick={() => setShockBuckets(toggleBucket(shockBuckets, b))}>
                {b}
              </Chip>
            ))}
          </div>
          <div className="row" style={{ gap: 6, marginTop: 8 }}>
            {SHORTCUTS.map(([label, set]) => (
              <button key={label} type="button" className="btn btn-ghost btn-sm" onClick={() => setShockBuckets(set)} aria-pressed={normaliseBuckets(set).join() === shockBuckets.join()}>
                {label}
              </button>
            ))}
          </div>
          <p className="t-small" style={{ margin: '8px 0 0' }}>
            These are the repricing gap's buckets. Rates at the chosen tenors move by the full size; the rest of the curve stays where it is.
          </p>
        </div>
      )}
    </div>
  );
}

/** The trial hedges: the list, and a short form that adds one at the market rate unless told otherwise. */
function Hedges() {
  const { hedges, setHedges, working } = useAlm();
  const market = useQuery({ queryKey: ['market', 'rates'], queryFn: endpoints.marketRates, staleTime: 60_000 });
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const ref = useOutsideClose<HTMLDivElement>(open, close);
  const curves = market.data?.curves ?? [];
  const ccys = curves.map((c) => c.ccy);
  const [kind, setKind] = useState<HedgeKind>('PAY_FIXED_SWAP');
  const [ccyChoice, setCcy] = useState<string | null>(null);
  const [millions, setMillions] = useState(50);
  const [tenor, setTenor] = useState(60);
  const [atMarket, setAtMarket] = useState(true);
  const [rate, setRate] = useState(5);
  const ccy = ccyChoice ?? (working && ccys.includes(working.reporting_ccy) ? working.reporting_ccy : (ccys[0] ?? ''));
  const curve = curves.find((c) => c.ccy === ccy);
  const marketRate = curve ? curveRate(curve, tenor / 12) : null;
  const full = hedges.length >= 20;
  const add = () => {
    if (!ccy || full) return;
    setHedges([...hedges, { kind, ccy, notional: millions * 1e6, tenor_months: tenor, rate: atMarket ? null : rate }]);
  };
  return (
    <div className="anchor" ref={ref}>
      <button type="button" className="btn btn-sm" onClick={() => setOpen((o) => !o)} aria-expanded={open} aria-haspopup="dialog" title="hedges tried on the balance sheet; never booked" data-testid="hedges-button">
        Hedges: {hedgesSummary(hedges.length)} ▾
      </button>
      {open && (
        <div className="popup hedge-panel" role="dialog" aria-label="Trial hedges" style={{ left: 0, right: 'auto' }}>
          <div className="t-eyebrow" style={{ marginBottom: 4 }}>
            Trial hedges
          </div>
          <p className="t-small" style={{ margin: '0 0 8px' }}>
            Tried on the balance sheet to see what they do to Gap, NII, EVE and Stress. Nothing is booked.
          </p>
          {hedges.length > 0 && (
            <div className="hedge-list" data-testid="hedge-list">
              {hedges.map((h, i) => (
                <div key={i} className="row" style={{ gap: 6 }}>
                  <span style={{ flex: 1 }}>{hedgeLabel(h)}</span>
                  <button type="button" className="btn btn-ghost btn-sm" onClick={() => setHedges(hedges.filter((_, j) => j !== i))} aria-label={`remove ${hedgeLabel(h)}`} title="remove this hedge">
                    ✕
                  </button>
                </div>
              ))}
              <button type="button" className="btn btn-ghost btn-sm" onClick={() => setHedges([])} style={{ marginTop: 2 }}>
                Remove all
              </button>
            </div>
          )}
          <div className="hedge-form">
            <label>
              <span className="t-small">Type</span>
              <select className="input input-sm" value={kind} onChange={(e) => setKind(e.target.value as HedgeKind)} aria-label="Hedge type">
                {HEDGE_KINDS.map(([k, label]) => (
                  <option key={k} value={k}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
            <label>
              <span className="t-small">Currency</span>
              <select className="input input-sm" value={ccy} onChange={(e) => setCcy(e.target.value)} aria-label="Hedge currency">
                {ccys.map((c) => (
                  <option key={c} value={c}>
                    {c}
                  </option>
                ))}
              </select>
            </label>
            <label>
              <span className="t-small">Notional (millions)</span>
              <Num value={millions} onChange={setMillions} min={1} max={100000} step={10} width={90} />
            </label>
            <label>
              <span className="t-small">Tenor</span>
              <select className="input input-sm" value={tenor} onChange={(e) => setTenor(Number(e.target.value))} aria-label="Hedge tenor">
                {TENORS.map(([label, m]) => (
                  <option key={m} value={m}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
            <label className="wide">
              <span className="t-small">Fixed rate</span>
              <span className="row" style={{ gap: 8 }}>
                <span className="row" style={{ gap: 4 }}>
                  <input type="checkbox" checked={atMarket} onChange={(e) => setAtMarket(e.target.checked)} aria-label="at the market rate" />
                  <span>at market{marketRate !== null ? ` (${marketRate.toFixed(2)}%)` : ''}</span>
                </span>
                {!atMarket && <Num value={rate} onChange={setRate} min={-5} max={50} step={0.05} decimals={2} suffix="%" width={72} />}
              </span>
            </label>
          </div>
          <p className="t-small" style={{ margin: '6px 0 8px' }}>
            {HEDGE_HELP[kind]}
          </p>
          <button type="button" className="btn btn-sm btn-primary" onClick={add} disabled={!ccy || full} title={full ? 'twenty hedges at most' : undefined}>
            Add hedge
          </button>
          {market.error && <span className="t-bad t-small"> {market.error.message}</span>}
        </div>
      )}
    </div>
  );
}

export function ShockBar({ applies, tool }: { /** Whether the shock changes this tool's figures. */ applies: boolean; /** The tool's name, for the note when it does not. */ tool: string }) {
  const { scenario, setScenario, customBp, setCustomBp, shockBuckets, clearShock, working, simulating } = useAlm();
  const shape = useMemo(() => shockShape(scenario, customBp, shockBuckets, working), [scenario, customBp, shockBuckets, working]);
  const on = scenario !== 'BASE';
  const sizes = shockSizes(working);
  return (
    <div className={`shock-bar noprint${simulating ? ' on' : ''}`} data-testid="shock-bar" data-on={on ? '1' : undefined}>
      <span className="t-eyebrow">Rate shock</span>
      <select className="input input-sm" value={scenario} onChange={(e) => setScenario(e.target.value as RateScenario)} aria-label="Rate shock" title="the rate move every rates tool runs under">
        {OPTIONS.map((s) => (
          <option key={s} value={s}>
            {s === 'BASE' ? 'None (today’s curve)' : SCENARIO_LABEL[s]}
          </option>
        ))}
      </select>
      {scenario === 'CUSTOM' && (
        <>
          <Bp value={customBp} onChange={setCustomBp} />
          <Tenors />
        </>
      )}
      {on && <Shape shape={shape} />}
      {on && scenario !== 'CUSTOM' && (
        <span className="t-small" title="The sizes of the Basel shocks per currency are set under Stress, in Rate shocks.">
          Basel shape, sized for {sizes.ccy}
        </span>
      )}
      {scenario === 'CUSTOM' && <span className="t-small">{customLabel(customBp, shockBuckets)}</span>}
      {on && (
        <button type="button" className="btn btn-ghost btn-sm" onClick={clearShock} title="back to today's curve">
          Reset
        </button>
      )}
      <span className="vsep" />
      <Hedges />
      <ScenarioMenu />
      <span style={{ flex: 1 }} />
      <span className="t-small" data-testid="shock-note">
        {!applies ? `no effect on ${tool}` : simulating ? 'simulation, not saved' : 'choose a shock, try a hedge or open a scenario to see Gap, NII, EVE and Stress under it'}
      </span>
    </div>
  );
}
