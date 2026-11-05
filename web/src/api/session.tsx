// The signed-in user for the whole page: a React view of the session store in client.ts.
import { createContext, useCallback, useContext, useMemo, useSyncExternalStore, type ReactNode } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { clearNotice, endpoints, session, type StoredSession } from './client';
import type { DeskAccess } from './types';

export interface SessionApi {
  current: StoredSession | null;
  desks: DeskAccess[];
  homeDesk: DeskAccess | null;
  /** ALM and treasury-head users see the balance sheet; the server is the authority with 404. */
  canSeeAlm: boolean;
  /** FX dealers and the treasury head see the FX risk page. */
  canSeeRisk: boolean;
  /** The PMU desk, ALM and the head see the bond portfolio. */
  canSeePmu: boolean;
  /** Users who can see a dealing desk (FX or money market): the blotter, breaks and liquidity. */
  canSeeTrading: boolean;
  /** The PMU desk's bookers: purchases, sales and repos of the bond portfolio. */
  canBookPmu: boolean;
  isHead: boolean;
  canBook: (deskId: number) => boolean;
  canResolve: (deskId: number) => boolean;
  /** Dealing desks (FX, MM) the user may book on; a PMU user's booking right is not for deals. */
  bookableDesks: DeskAccess[];
  login: (username: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
}

/** The desks that book deals. */
export const DEALING = new Set(['FX', 'MM']);

const Ctx = createContext<SessionApi | null>(null);

export function SessionProvider({ children }: { children: ReactNode }) {
  const current = useSyncExternalStore(session.subscribe, session.get, session.get);
  const qc = useQueryClient();
  const login = useCallback(async (username: string, password: string) => {
    const r = await endpoints.login(username, password);
    clearNotice();
    qc.clear();
    session.set({ token: r.token, expires_at: r.expires_at, user: r.user, desks: r.desks });
  }, [qc]);
  const logout = useCallback(async () => {
    try {
      await endpoints.logout();
    } catch {
      // the token is dropped either way
    }
    session.set(null);
    qc.clear();
  }, [qc]);
  const value = useMemo<SessionApi>(() => {
    const desks = current?.desks ?? [];
    const codes = new Set(desks.map((d) => d.code));
    return {
      current,
      desks,
      homeDesk: desks.find((d) => d.desk_id === current?.user.home_desk_id) ?? null,
      canSeeAlm: codes.has('ALM') || codes.has('HEAD'),
      canSeeRisk: codes.has('FX') || codes.has('HEAD'),
      canSeePmu: codes.has('PMU') || codes.has('ALM') || codes.has('HEAD'),
      canSeeTrading: codes.has('FX') || codes.has('MM'),
      canBookPmu: desks.some((d) => d.code === 'PMU' && d.can_book),
      isHead: codes.has('HEAD'),
      canBook: (id) => desks.some((d) => d.desk_id === id && d.can_book),
      canResolve: (id) => desks.some((d) => d.desk_id === id && d.can_resolve),
      bookableDesks: desks.filter((d) => d.can_book && DEALING.has(d.code)),
      login,
      logout,
    };
  }, [current, login, logout]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useSession(): SessionApi {
  const v = useContext(Ctx);
  if (!v) throw new Error('useSession outside SessionProvider');
  return v;
}
