// The PMU tab: the bank's bond portfolio in five pages, replacing the desk's eight-tab Power BI
// report (see docs/pmu-desk-spec.md for which tab went where). One strip moves between them.
import { Navigate, Route, Routes, useLocation, useNavigate } from 'react-router';
import { useSession } from '../../api/session';
import { Chip } from '../../components/ui';
import { PmuOverviewPage } from './Overview';
import { PositionsPage } from './Positions';
import { IncomePage } from './Income';
import { RiskPage } from './Risk';
import { FundingPage } from './Funding';

const PAGES: [string, string][] = [
  ['', 'Overview'],
  ['positions', 'Positions'],
  ['income', 'Income'],
  ['risk', 'Risk and hedges'],
  ['funding', 'Funding and repo'],
];

export function Pmu() {
  const { canSeePmu } = useSession();
  const loc = useLocation();
  const navigate = useNavigate();
  if (!canSeePmu) return <Navigate to="/" replace />;
  const current = loc.pathname.replace(/^\/pmu\/?/, '').split('/')[0];
  return (
    <div className="alm-page">
      <div className="alm-strip" data-testid="pmu-strip">
        {PAGES.map(([p, label]) => (
          <Chip key={p} selected={current === p} onClick={() => navigate(p ? `/pmu/${p}` : '/pmu')}>
            {label}
          </Chip>
        ))}
      </div>
      <div className="pmu-body">
        <Routes>
          <Route index element={<PmuOverviewPage />} />
          <Route path="positions" element={<PositionsPage />} />
          <Route path="income" element={<IncomePage />} />
          <Route path="risk" element={<RiskPage />} />
          <Route path="funding" element={<FundingPage />} />
          <Route path="*" element={<Navigate to="/pmu" replace />} />
        </Routes>
      </div>
    </div>
  );
}
