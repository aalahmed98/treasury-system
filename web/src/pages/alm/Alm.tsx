// The ALM workbench tab: a tool strip grouped by purpose, the snapshot in use, and one tool at a
// time with its assumptions on the left and its results on the right.
import { useNavigate, useParams } from 'react-router';
import { Chip } from '../../components/ui';
import { AlmBlock } from '../Overview';
import { useAlm } from '../../alm/state';
import { fmtDateLong } from '../../lib/fmt';
import { Lcr } from './Lcr';
import { Nsfr } from './Nsfr';
import { Gap } from './Gap';
import { Nii } from './Nii';
import { Stress } from './Stress';
import { Attribution } from './Attribution';
import { IrdPage } from './Ird';
import { Funding } from './Funding';
import { Setup } from './Setup';
import { LiquidityGap } from './LiquidityGap';
import { Ewi } from './Ewi';
import { Pack } from './Pack';

type Tool = 'dashboard' | 'lcr' | 'nsfr' | 'ladder' | 'ewi' | 'funding' | 'gap' | 'nii' | 'stress' | 'ird' | 'attribution' | 'pack' | 'setup';
const LABEL: Record<Tool, string> = {
  dashboard: 'Dashboard',
  lcr: 'LCR',
  nsfr: 'NSFR',
  ladder: 'Ladder',
  ewi: 'Early warning',
  funding: 'Funding',
  gap: 'Gap',
  nii: 'NII / NIM',
  stress: 'Stress',
  ird: 'IRD',
  attribution: 'Attribution',
  pack: 'ALCO pack',
  setup: 'Setup',
};
const GROUPS: [string, Tool[]][] = [
  ['', ['dashboard']],
  ['Liquidity', ['lcr', 'nsfr', 'ladder', 'ewi', 'funding']],
  ['Rates', ['gap', 'nii', 'stress', 'ird']],
  ['Income', ['attribution']],
];

export function Alm() {
  const { tool: raw } = useParams();
  const navigate = useNavigate();
  const tool: Tool = raw && raw in LABEL ? (raw as Tool) : 'dashboard';
  const { params, snapshots, asOf, setAsOf, bank } = useAlm();
  const go = (t: Tool) => navigate(t === 'dashboard' ? '/alm' : `/alm/${t}`);
  return (
    <div className="alm-page">
      <div className="alm-strip" data-testid="alm-strip-bar">
        {GROUPS.map(([name, tools], gi) => (
          <div key={name || 'first'} className="row" style={{ gap: 3 }}>
            {gi > 0 && <span className="vsep" />}
            {name && <span className="t-eyebrow">{name}</span>}
            {tools.map((t) => (
              <Chip key={t} selected={tool === t} onClick={() => go(t)}>
                {LABEL[t]}
              </Chip>
            ))}
          </div>
        ))}
        <span style={{ flex: 1 }} />
        {(params.isFetching || snapshots.isFetching) && <span className="t-small">…</span>}
        {(params.error || snapshots.error) && <span className="t-bad t-small">{(params.error ?? snapshots.error)?.message}</span>}
        <span className="t-small">snapshot</span>
        <select className="input input-sm" value={asOf ?? ''} onChange={(e) => setAsOf(e.target.value || null)} aria-label="Snapshot">
          <option value="">latest</option>
          {(snapshots.data ?? []).map((s) => (
            <option key={s.as_of} value={s.as_of}>
              {fmtDateLong(s.as_of)} ({s.positions} cohorts)
            </option>
          ))}
        </select>
        <span className="vsep" />
        <Chip selected={tool === 'pack'} onClick={() => go('pack')}>
          ALCO pack
        </Chip>
        <Chip selected={tool === 'setup'} onClick={() => go('setup')}>
          Setup
        </Chip>
      </div>
      {!bank ? (
        <div className="alm-results">
          <p className="t-muted">{params.error ? params.error.message : "loading the bank's parameters…"}</p>
        </div>
      ) : (
        <ToolBody tool={tool} />
      )}
    </div>
  );
}

function ToolBody({ tool }: { tool: Tool }) {
  switch (tool) {
    case 'lcr':
      return <Lcr />;
    case 'nsfr':
      return <Nsfr />;
    case 'funding':
      return <Funding />;
    case 'gap':
      return <Gap />;
    case 'nii':
      return <Nii />;
    case 'stress':
      return <Stress />;
    case 'ird':
      return <IrdPage />;
    case 'attribution':
      return <Attribution />;
    case 'setup':
      return <Setup />;
    case 'ladder':
      return <LiquidityGap />;
    case 'ewi':
      return <Ewi />;
    case 'pack':
      return <Pack />;
    default:
      return (
        <div className="alm-layout single">
          <section className="alm-results" data-ready="1">
            <AlmBlock />
          </section>
        </div>
      );
  }
}
