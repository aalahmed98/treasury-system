// The live event stream for the signed-in session, and what it does to the query cache:
// a deal or break event refetches the affected lists, an ALM event the parameter-driven views,
// a resync everything. Also the presence heartbeat: a non-passive request every minute while the
// user is actually using the page, like the native client's track_presence.
import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { endpoints, session } from './client';
import { openEvents, type StreamState } from './sse';
import type { ServerEvent } from './types';

export interface Live {
  state: StreamState;
  /** When the last event (or keep-alive) arrived. */
  lastEvent: Date | null;
  /** Ids of deals and breaks that changed in the last few seconds, for a row flash. */
  flash: { deals: Set<string>; breaks: Set<number> };
}

const Ctx = createContext<Live>({ state: 'disconnected', lastEvent: null, flash: { deals: new Set(), breaks: new Set() } });

const HEARTBEAT_MS = 60_000;
const FLASH_MS = 4_000;

export function LiveProvider({ children }: { children: ReactNode }) {
  const qc = useQueryClient();
  const token = session.get()?.token ?? null;
  const [state, setState] = useState<StreamState>('disconnected');
  const [lastEvent, setLastEvent] = useState<Date | null>(null);
  const [flashTick, setFlashTick] = useState(0);
  const flashRef = useRef<{ deals: Map<string, number>; breaks: Map<number, number> }>({ deals: new Map(), breaks: new Map() });

  useEffect(() => {
    if (!token) return;
    const mark = () => setLastEvent(new Date());
    const stop = openEvents({
      onState: setState,
      onPing: mark,
      onEvent: (ev: ServerEvent) => {
        mark();
        const now = Date.now();
        switch (ev.type) {
          case 'deal':
            flashRef.current.deals.set(ev.id, now);
            void qc.invalidateQueries({ queryKey: ['deals'] });
            void qc.invalidateQueries({ queryKey: ['positions'] });
            void qc.invalidateQueries({ queryKey: ['ladder'] });
            void qc.invalidateQueries({ queryKey: ['risk'] });
            void qc.invalidateQueries({ queryKey: ['deal', ev.id] });
            break;
          case 'break':
            flashRef.current.breaks.set(ev.id, now);
            void qc.invalidateQueries({ queryKey: ['breaks'] });
            void qc.invalidateQueries({ queryKey: ['deals'] });
            if (ev.deal_id) void qc.invalidateQueries({ queryKey: ['deal', ev.deal_id] });
            break;
          case 'alm':
            if (ev.ftp) void qc.invalidateQueries({ queryKey: ['ftp'] });
            if (ev.ftp || ev.pmu) void qc.invalidateQueries({ queryKey: ['pmu'] });
            void qc.invalidateQueries({ queryKey: ['alm'] });
            void qc.invalidateQueries({ queryKey: ['risk'] });
            void qc.invalidateQueries({ queryKey: ['attention'] });
            void qc.invalidateQueries({ queryKey: ['activity'] });
            break;
          case 'resync':
            void qc.invalidateQueries();
            break;
        }
        setFlashTick((t) => t + 1);
        window.setTimeout(() => setFlashTick((t) => t + 1), FLASH_MS + 50);
      },
    });
    return stop;
  }, [token, qc]);

  // Presence: one real request a minute while there is input.
  useEffect(() => {
    if (!token) return;
    let lastInput = Date.now();
    const touch = () => {
      lastInput = Date.now();
    };
    const events: (keyof WindowEventMap)[] = ['pointerdown', 'keydown', 'wheel'];
    events.forEach((e) => window.addEventListener(e, touch, { passive: true }));
    const timer = window.setInterval(() => {
      if (Date.now() - lastInput < HEARTBEAT_MS) void endpoints.me().catch(() => {});
    }, HEARTBEAT_MS);
    return () => {
      events.forEach((e) => window.removeEventListener(e, touch));
      window.clearInterval(timer);
    };
  }, [token]);

  const value = useMemo<Live>(() => {
    const cutoff = Date.now() - FLASH_MS;
    const deals = new Set<string>();
    const breaks = new Set<number>();
    flashRef.current.deals.forEach((at, id) => (at >= cutoff ? deals.add(id) : flashRef.current.deals.delete(id)));
    flashRef.current.breaks.forEach((at, id) => (at >= cutoff ? breaks.add(id) : flashRef.current.breaks.delete(id)));
    return { state, lastEvent, flash: { deals, breaks } };
    // flashTick is the dependency that re-evaluates the sets as flashes expire.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, lastEvent, flashTick]);

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useLive(): Live {
  return useContext(Ctx);
}
