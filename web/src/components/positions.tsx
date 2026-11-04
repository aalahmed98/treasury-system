// The trading pages' side column: the positions panel (net notional per desk and currency), which
// the user can close and the page remembers, and a slot above it for the booking ticket.
import { createContext, useContext, useMemo, type ReactNode } from 'react';
import { usePositions } from '../api/queries';
import { fmtAmount } from '../lib/fmt';
import { usePref } from '../lib/prefs';
import { SkeletonLines } from './feedback';

interface Side {
  open: boolean;
  toggle: () => void;
}
const Ctx = createContext<Side>({ open: true, toggle: () => {} });

export function PositionsPanel() {
  const { data, isLoading } = usePositions();
  const { toggle } = useContext(Ctx);
  const rows = data ?? [];
  return (
    <section aria-label="Positions">
      <div className="row" style={{ marginBottom: 2 }}>
        <span className="t-heading">Positions</span>
        <span className="grow" />
        <button type="button" className="btn btn-ghost btn-sm" onClick={toggle} aria-label="hide positions" title="hide the positions panel">
          ✕
        </button>
      </div>
      <div className="t-small" style={{ marginBottom: 8 }}>
        Net notional per desk and currency on the desks you can see.
      </div>
      {isLoading ? (
        <SkeletonLines n={5} />
      ) : (
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
                  no positions on your desks
                </td>
              </tr>
            )}
          </tbody>
        </table>
      )}
    </section>
  );
}

/** The button a page's toolbar shows to bring the positions back. */
export function SideToggle() {
  const { open, toggle } = useContext(Ctx);
  if (open) return null;
  return (
    <button type="button" className="btn btn-ghost btn-sm" onClick={toggle} title="show the positions panel">
      Positions
    </button>
  );
}

/** The trading pages: the page on the left; on the right the ticket (when open) and the positions. */
export function WithSide({ page, children, ready, sheet }: { page: string; children: ReactNode; ready?: boolean; sheet?: ReactNode }) {
  const [open, setOpen] = usePref<boolean>(`side-${page}`, page === 'blotter');
  const side = useMemo<Side>(() => ({ open, toggle: () => setOpen((o) => !o) }), [open, setOpen]);
  const show = open || sheet != null;
  return (
    <Ctx.Provider value={side}>
      <div className={`layout-side${show ? '' : ' closed'}`}>
        <div className="main-col" data-ready={ready ? '1' : undefined}>
          {children}
        </div>
        <aside className="side">
          {sheet}
          {open && <PositionsPanel />}
        </aside>
      </div>
    </Ctx.Provider>
  );
}
