// The rate shock bar under the ALM tool strip: the one place the shock is chosen. It stays in the
// same spot on every analysis tool, and says so on the tools a rate move does not reach.
import { useCallback, useMemo, useState } from 'react';
import type { RateScenario } from '../api/types';
import { SCENARIO_LABEL } from '../lib/alm';
import { Chip, useOutsideClose } from '../components/ui';
import { useAlm } from './state';
import { BASEL, Bp } from './tool';
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

export function ShockBar({ applies, tool }: { /** Whether the shock changes this tool's figures. */ applies: boolean; /** The tool's name, for the note when it does not. */ tool: string }) {
  const { scenario, setScenario, customBp, setCustomBp, shockBuckets, clearShock, working } = useAlm();
  const shape = useMemo(() => shockShape(scenario, customBp, shockBuckets, working), [scenario, customBp, shockBuckets, working]);
  const on = scenario !== 'BASE';
  const sizes = shockSizes(working);
  return (
    <div className={`shock-bar noprint${on ? ' on' : ''}`} data-testid="shock-bar" data-on={on ? '1' : undefined}>
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
      <span style={{ flex: 1 }} />
      <span className="t-small" data-testid="shock-note">
        {!applies ? `no effect on ${tool}` : on ? 'simulation, not saved' : 'choose a shock to see Gap, NII, EVE and Stress under it'}
      </span>
    </div>
  );
}
