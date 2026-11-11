// The ALM workbench's shared state: the bank's saved parameters, the user's unsaved what-if
// overrides, the snapshot and scenario in use, and the Setup draft. Tool results are React Query
// entries keyed by the request, so a change to any input refetches after a short debounce.
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { keepPreviousData, useQuery, type UseQueryResult } from '@tanstack/react-query';
import { endpoints } from '../api/client';
import type { AlmParams, AlmParamsResponse, AlmRequest, AlmSnapshot, CompareTo, RateScenario } from '../api/types';
import { RATE_TOOLS, normaliseBuckets } from './shock';

/** The rate shock is a simulation for this sitting: it survives a reload of the tab and nothing
 * else. A blocked or cleared store simply means it starts from the base curve. */
interface StoredShock {
  scenario: RateScenario;
  customBp: number;
  buckets: string[];
}
const SHOCK_KEY = 'nbb-alm-shock';
const NO_SHOCK: StoredShock = { scenario: 'BASE', customBp: 100, buckets: [] };
function readShock(): StoredShock {
  try {
    const raw = sessionStorage.getItem(SHOCK_KEY);
    if (!raw) return NO_SHOCK;
    const v = JSON.parse(raw) as Partial<StoredShock>;
    return { scenario: v.scenario ?? 'BASE', customBp: Number.isFinite(v.customBp) ? (v.customBp as number) : 100, buckets: Array.isArray(v.buckets) ? normaliseBuckets(v.buckets) : [] };
  } catch {
    return NO_SHOCK;
  }
}
function writeShock(v: StoredShock) {
  try {
    sessionStorage.setItem(SHOCK_KEY, JSON.stringify(v));
  } catch {
    // storage unavailable: the shock lives for this page only
  }
}

export function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = window.setTimeout(() => setV(value), ms);
    return () => window.clearTimeout(t);
  }, [value, ms]);
  return v;
}

export interface AlmState {
  params: UseQueryResult<AlmParamsResponse>;
  snapshots: UseQueryResult<AlmSnapshot[]>;
  bank: AlmParams | null;
  bankVersion: number | null;
  overrides: AlmParams | null;
  /** What the tools run on: the overrides when editing, else the bank's set. */
  working: AlmParams | null;
  /** Clone the working set, apply the change, keep it as the what-if. */
  edit: (change: (p: AlmParams) => void) => void;
  reset: () => void;
  bankChanged: boolean;
  asOf: string | null;
  setAsOf: (d: string | null) => void;
  scenario: RateScenario;
  setScenario: (s: RateScenario) => void;
  customBp: number;
  setCustomBp: (bp: number) => void;
  /** The gap buckets the custom shock moves; empty means every tenor. */
  shockBuckets: string[];
  setShockBuckets: (b: string[]) => void;
  /** Back to the base curve. */
  clearShock: () => void;
  compareTo: CompareTo;
  setCompareTo: (c: CompareTo) => void;
  period: string | null;
  setPeriod: (p: string | null) => void;
  /** The request a tool sends, with the debounced what-if. */
  request: (extra?: Partial<AlmRequest>) => AlmRequest;
  draft: AlmParams | null;
  setDraft: (d: AlmParams | null) => void;
  note: string;
  setNote: (n: string) => void;
}

const Ctx = createContext<AlmState | null>(null);

export function AlmProvider({ children }: { children: ReactNode }) {
  const params = useQuery({ queryKey: ['alm', 'params'], queryFn: endpoints.almParams });
  const snapshots = useQuery({ queryKey: ['alm', 'snapshots'], queryFn: endpoints.almSnapshots });
  const [overrides, setOverrides] = useState<AlmParams | null>(null);
  const overridesFrom = useRef<number | null>(null);
  const [asOf, setAsOf] = useState<string | null>(null);
  const [stored] = useState(readShock);
  const [scenario, setScenario] = useState<RateScenario>(stored.scenario);
  const [customBp, setCustomBp] = useState(stored.customBp);
  const [shockBuckets, setBuckets] = useState<string[]>(stored.buckets);
  const setShockBuckets = useCallback((b: string[]) => setBuckets(normaliseBuckets(b)), []);
  const clearShock = useCallback(() => setScenario('BASE'), []);
  useEffect(() => writeShock({ scenario, customBp, buckets: shockBuckets }), [scenario, customBp, shockBuckets]);
  const debouncedBp = useDebounced(customBp, 300);
  const [compareTo, setCompareTo] = useState<CompareTo>('PRIOR');
  const [period, setPeriod] = useState<string | null>(null);
  const [draft, setDraft] = useState<AlmParams | null>(null);
  const [note, setNote] = useState('');
  const bank = params.data?.params ?? null;
  const bankVersion = params.data?.active.version ?? null;
  const working = overrides ?? bank;
  const debounced = useDebounced(overrides, 300);

  const edit = useCallback(
    (change: (p: AlmParams) => void) => {
      setOverrides((prev) => {
        const base = prev ?? bank;
        if (!base) return prev;
        const next = structuredClone(base);
        change(next);
        if (prev === null) overridesFrom.current = bankVersion;
        return next;
      });
    },
    [bank, bankVersion],
  );
  const reset = useCallback(() => {
    setOverrides(null);
    overridesFrom.current = null;
  }, []);
  const bankChanged = overrides !== null && overridesFrom.current !== null && bankVersion !== null && overridesFrom.current !== bankVersion;

  const request = useCallback(
    (extra: Partial<AlmRequest> = {}): AlmRequest => ({
      as_of: asOf,
      params: debounced,
      scenario,
      // The custom size and its buckets travel only with the custom scenario.
      custom_shock_bp: scenario === 'CUSTOM' ? debouncedBp : null,
      custom_shock_buckets: scenario === 'CUSTOM' && shockBuckets.length ? shockBuckets : null,
      compare_to: compareTo,
      period,
      ...extra,
    }),
    [asOf, debounced, scenario, debouncedBp, shockBuckets, compareTo, period],
  );

  const value = useMemo<AlmState>(
    () => ({ params, snapshots, bank, bankVersion, overrides, working, edit, reset, bankChanged, asOf, setAsOf, scenario, setScenario, customBp, setCustomBp, shockBuckets, setShockBuckets, clearShock, compareTo, setCompareTo, period, setPeriod, request, draft, setDraft, note, setNote }),
    [params, snapshots, bank, bankVersion, overrides, working, edit, reset, bankChanged, asOf, scenario, customBp, shockBuckets, setShockBuckets, clearShock, compareTo, period, request, draft, note],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useAlm(): AlmState {
  const v = useContext(Ctx);
  if (!v) throw new Error('useAlm outside AlmProvider');
  return v;
}

/** One tool's result for the current inputs; the previous result stays on screen while the next computes. */
export function useTool<T>(tool: string, extra: Partial<AlmRequest> = {}) {
  const { request, bank } = useAlm();
  // A tool the shock does not reach asks without it, so changing the shock never refetches it. The
  // stress table runs every scenario itself and takes only the custom one from the shock.
  const quiet: Partial<AlmRequest> = RATE_TOOLS.has(tool) ? (tool === 'stress' ? { scenario: null } : {}) : { scenario: null, custom_shock_bp: null, custom_shock_buckets: null };
  const req = request({ ...quiet, ...extra });
  return useQuery({ queryKey: ['alm', 'tool', tool, req], queryFn: () => endpoints.almTool<T>(tool, req), enabled: bank !== null, placeholderData: keepPreviousData, staleTime: 10_000 });
}
