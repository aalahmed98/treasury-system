// One fetch wrapper: the bearer session, the passive header for background polls, the server's
// `{error}` body as the message, and a 401 that ends the session. Same contract as the native
// client's net layer.
import type { AmendDeal, AttentionItem, ActivityRow, AlmOverview, AlmSnapshot, BreakDetail, BreakView, CashFlow, Deal, DealsResponse, DeskAccess, FeedsResponse, HistoryEntry, LadderResponse, LoginResponse, MeResponse, NewDeal, NostroMovement, Position, ProvenanceResponse, ResolveRequest, UserInfo } from './types';

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

export interface StoredSession {
  token: string;
  expires_at: string;
  user: UserInfo;
  desks: DeskAccess[];
}

const KEY = 'nbb-session';
type Listener = () => void;
const listeners = new Set<Listener>();
let current: StoredSession | null = load();
/** Why the user is back on the sign-in page, if the session ended on its own. */
export let sessionNotice: string | null = null;

function load(): StoredSession | null {
  try {
    const raw = sessionStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as StoredSession) : null;
  } catch {
    return null;
  }
}

export const session = {
  get(): StoredSession | null {
    return current;
  },
  set(s: StoredSession | null) {
    current = s;
    try {
      if (s) sessionStorage.setItem(KEY, JSON.stringify(s));
      else sessionStorage.removeItem(KEY);
    } catch {
      // storage can be unavailable; the session still lives for this page
    }
    listeners.forEach((l) => l());
  },
  subscribe(l: Listener) {
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  },
};

export function endSession(notice: string | null) {
  sessionNotice = notice;
  session.set(null);
}

export function clearNotice() {
  sessionNotice = null;
}

export interface CallInit {
  method?: 'GET' | 'POST' | 'PATCH' | 'DELETE';
  body?: unknown;
  /** A background poll: the server does not count it as user presence. */
  passive?: boolean;
  signal?: AbortSignal;
}

export async function api<T>(path: string, init: CallInit = {}): Promise<T> {
  const headers: Record<string, string> = { accept: 'application/json' };
  if (init.body !== undefined) headers['content-type'] = 'application/json';
  const tok = current?.token;
  if (tok) headers.authorization = `Bearer ${tok}`;
  if (init.passive) headers['x-treasury-passive'] = '1';
  const res = await fetch(path, {
    method: init.method ?? 'GET',
    headers,
    body: init.body === undefined ? undefined : JSON.stringify(init.body),
    signal: init.signal,
    cache: 'no-store',
  });
  if (res.status === 401 && current) {
    endSession('Your session ended. Sign in again.');
    throw new ApiError(401, 'session ended');
  }
  if (!res.ok) {
    let msg = `${res.status} ${res.statusText}`;
    try {
      const body = (await res.json()) as { error?: unknown };
      if (body && typeof body.error === 'string') msg = body.error;
    } catch {
      // not a JSON error body
    }
    throw new ApiError(res.status, msg);
  }
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

const q = (params: Record<string, string | number | undefined>) => {
  const u = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v !== undefined) u.set(k, String(v));
  const s = u.toString();
  return s ? `?${s}` : '';
};

/** Every call the pages make, with the same passive flags as the native client. */
export const endpoints = {
  login: (username: string, password: string) => api<LoginResponse>('/auth/login', { method: 'POST', body: { username, password } }),
  logout: () => api<void>('/auth/logout', { method: 'POST' }),
  /** Not passive: this is the heartbeat that keeps a present user's session alive. */
  me: () => api<MeResponse>('/me'),
  deals: (since?: string, passive = true) => api<DealsResponse>(`/deals${q({ since })}`, { passive }),
  deal: (id: string) => api<Deal>(`/deals/${id}`, { passive: true }),
  provenance: (id: string) => api<ProvenanceResponse>(`/deals/${id}/provenance`, { passive: true }),
  history: (id: string) => api<HistoryEntry[]>(`/deals/${id}/history`, { passive: true }),
  book: (deal: NewDeal) => api<Deal>('/deals', { method: 'POST', body: deal }),
  amend: (id: string, req: AmendDeal) => api<Deal>(`/deals/${id}`, { method: 'PATCH', body: req }),
  cancel: (id: string) => api<Deal>(`/deals/${id}/cancel`, { method: 'POST' }),
  breaks: (all: boolean, passive = true) => api<BreakView[]>(`/breaks${q({ status: all ? 'all' : 'open' })}`, { passive }),
  breakDetail: (id: number) => api<BreakDetail>(`/breaks/${id}`, { passive: true }),
  ackBreak: (id: number) => api<BreakView>(`/breaks/${id}/ack`, { method: 'POST' }),
  resolveBreak: (id: number, req: ResolveRequest) => api<BreakView>(`/breaks/${id}/resolve`, { method: 'POST', body: req }),
  feeds: () => api<FeedsResponse>('/feeds', { passive: true }),
  positions: (passive = true) => api<Position[]>('/positions', { passive }),
  ladder: (ccy?: string, passive = true) => api<LadderResponse>(`/ladder${q({ days: 35, ccy })}`, { passive }),
  flows: (ccy: string, date: string) => api<CashFlow[]>(`/ladder/flows${q({ ccy, date })}`, { passive: true }),
  movements: (ccy: string, date: string) => api<NostroMovement[]>(`/ladder/movements${q({ ccy, date })}`, { passive: true }),
  activity: (limit: number) => api<ActivityRow[]>(`/activity${q({ limit })}`, { passive: true }),
  attention: () => api<AttentionItem[]>('/attention', { passive: true }),
  almOverview: () => api<AlmOverview>('/alm/overview', { passive: true }),
  almSnapshots: () => api<AlmSnapshot[]>('/alm/snapshots', { passive: true }),
  almTool: <T>(tool: string, req: unknown) => api<T>(`/alm/${tool}`, { method: 'POST', body: req, passive: true }),
};
