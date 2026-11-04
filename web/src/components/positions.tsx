// The positions side panel on the trading pages: net notional per desk and currency.
import { usePositions } from '../api/queries';
import { fmtAmount } from '../lib/fmt';

export function PositionsPanel() {
  const { data } = usePositions();
  const rows = data ?? [];
  return (
    <aside className="side" aria-label="Positions">
      <div className="t-heading">Positions</div>
      <div className="t-small" style={{ marginBottom: 6 }}>
        Net notional per desk and currency, from the server's entitlement-filtered view.
      </div>
      <table className="table">
        <thead>
          <tr>
            <th>Desk</th>
            <th>Ccy</th>
            <th className="num">Net</th>
            <th className="num">Deals</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((p) => (
            <tr key={`${p.desk_id}-${p.ccy}`}>
              <td>{p.desk_code}</td>
              <td className="t-medium">{p.ccy}</td>
              <td className={`num ${Number(p.net_notional) < 0 ? 't-bad' : 't-good'}`}>{fmtAmount(p.net_notional)}</td>
              <td className="num">{p.deal_count}</td>
            </tr>
          ))}
          {rows.length === 0 && (
            <tr>
              <td colSpan={4} className="t-muted">
                no positions
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </aside>
  );
}

/** The trading pages: the page on the left, positions on the right. */
export function WithPositions({ children, ready }: { children: React.ReactNode; ready?: boolean }) {
  return (
    <div className="layout-side">
      <div className="main-col" data-ready={ready ? '1' : undefined}>
        {children}
      </div>
      <PositionsPanel />
    </div>
  );
}
