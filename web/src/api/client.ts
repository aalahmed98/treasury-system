// One fetch wrapper: the bearer session, the passive header for background polls, the server's
// `{error}` body as the message, and a 401 that ends the session. Same contract as the native
// client's net layer.
import type { Booked, BondSearchHit, BondTerms, RepoPreview, RepoRequest, TicketPreview, TicketRequest, PmuBondDetail, PmuForecast, PmuFunding, PmuIncome, PmuOverview, PmuPositions, PmuRisk, FtpCurrent, FtpHistory, FtpPreview,
  FtpLog, FtpPublication, FtpCurve, AlcoPack, Breach, FxDealValue, FxRiskResult, RiskLimits, RiskLimitsResponse, RiskLimitsVersion, AlmParamsResponse, AlmParamsVersion, AlmRequest, AmendDeal, AttentionItem, ActivityRow, AlmOverview, AlmParams, AlmSnapshot, BreakDetail, BreakView, CashFlow, Deal, DealsResponse, DeskAccess, FeedsResponse, HistoryEntry, LadderResponse, LoginResponse, MarketResponse, MeResponse, NewDeal, NostroMovement, Position, ProvenanceResponse, ResolveRequest, UserInfo } from './types';

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
  /** A file sent as it is (an uploaded workbook) instead of a JSON body. */
  raw?: Blob;
}

export async function api<T>(path: string, init: CallInit = {}): Promise<T> {
  const headers: Record<string, string> = { accept: 'application/json' };
  if (init.body !== undefined) headers['content-type'] = 'application/json';
  if (init.raw) headers['content-type'] = 'application/octet-stream';
  const res = await send(path, init, headers);
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

async function send(path: string, init: CallInit, headers: Record<string, string>): Promise<Response> {
  const tok = current?.token;
  if (tok) headers.authorization = `Bearer ${tok}`;
  if (init.passive) headers['x-treasury-passive'] = '1';
  return fetch(path, {
    method: init.method ?? 'GET',
    headers,
    body: init.raw ?? (init.body === undefined ? undefined : JSON.stringify(init.body)),
    signal: init.signal,
    cache: 'no-store',
  });
}

/** Fetches a file with the session (the browser cannot add the bearer to a plain link) and saves it. */
export async function download(path: string, fallbackName: string) {
  const res = await send(path, {}, {});
  if (!res.ok) throw new ApiError(res.status, `${res.status} ${res.statusText}`);
  const name = /filename="([^"]+)"/.exec(res.headers.get('content-disposition') ?? '')?.[1] ?? fallbackName;
  const url = URL.createObjectURL(await res.blob());
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
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
  almTool: <T>(tool: string, req: AlmRequest) => api<T>(`/alm/${tool}`, { method: 'POST', body: req, passive: true }),
  almParams: () => api<AlmParamsResponse>('/alm/params', { passive: true }),
  almSave: (params: AlmParams, note: string) => api<AlmParamsVersion>('/alm/params', { method: 'POST', body: { params, note } }),
  marketRates: () => api<MarketResponse>('/market/rates', { passive: true }),
  fxRisk: () => api<FxRiskResult>('/risk/fx', { passive: true }),
  fxHistory: (days: number) => api<{ points: import('./types').FxHistoryPoint[] }>(`/risk/fx/history${q({ days })}`, { passive: true }),
  fxDeals: () => api<FxDealValue[]>('/risk/fx/deals', { passive: true }),
  riskLimits: () => api<RiskLimitsResponse>('/risk/limits', { passive: true }),
  saveLimits: (limits: RiskLimits, note: string) => api<RiskLimitsVersion>('/risk/limits', { method: 'POST', body: { limits, note } }),
  breaches: (days: number) => api<Breach[]>(`/risk/breaches${q({ days })}`, { passive: true }),
  signBreach: (date: string, key: string, note: string) => api<void>('/risk/breaches/sign', { method: 'POST', body: { date, key, note } }),
  pack: (period?: string) => api<AlcoPack>(`/alm/pack${q({ period })}`, { passive: true }),
  packNote: (period: string, section: string, body: string) => api<void>('/alm/pack/note', { method: 'POST', body: { period, section, body } }),
  packSign: (period: string, note: string) => api<void>('/alm/pack/sign', { method: 'POST', body: { period, note } }),
  ftp: (asOf?: string) => api<FtpCurrent>(`/ftp${q({ as_of: asOf })}`, { passive: true }),
  ftpHistory: (ccy: string, tenor: string, days: number) => api<FtpHistory>(`/ftp/history${q({ ccy, tenor, days })}`, { passive: true }),
  ftpPreview: (file: File) => api<FtpPreview>(`/ftp/preview${q({ name: file.name })}`, { method: 'POST', raw: file }),
  ftpPublish: (req: { as_of: string; curves: FtpCurve[]; file_name: string | null; file_sha256: string | null; note: string; upload_id?: number | null }) =>
    api<FtpPublication>('/ftp/publish', { method: 'POST', body: req }),
  ftpLog: (limit: number, filesOnly: boolean) => api<FtpLog>(`/ftp/log${q({ limit: String(limit), files_only: String(filesOnly) })}`, { passive: true }),
  ftpFile: (uploadId: number, name: string) => download(`/ftp/upload/${uploadId}/file`, name),
  ftpTemplate: () => download('/ftp/template.xlsx', 'ftp-curves.xlsx'),
  pmuOverview: () => api<PmuOverview>('/pmu/overview', { passive: true }),
  pmuPositions: (from?: string, to?: string) => api<PmuPositions>(`/pmu/positions${q({ from, to })}`, { passive: true }),
  pmuIncome: (from?: string, to?: string) => api<PmuIncome>(`/pmu/income${q({ from, to })}`, { passive: true }),
  pmuForecast: (reinvest: boolean) => api<PmuForecast>(`/pmu/forecast${q({ reinvest: String(reinvest) })}`, { passive: true }),
  pmuRisk: () => api<PmuRisk>('/pmu/risk', { passive: true }),
  pmuFunding: () => api<PmuFunding>('/pmu/funding', { passive: true }),
  pmuBond: (isin: string) => api<PmuBondDetail>(`/pmu/bond/${encodeURIComponent(isin)}`, { passive: true }),
  pmuBonds: (q: string) => api<BondSearchHit[]>(`/pmu/bonds${q ? `?q=${encodeURIComponent(q)}` : ''}`, { passive: true }),
  pmuAddBond: (b: BondTerms) => api<Booked>('/pmu/bonds', { method: 'POST', body: b }),
  pmuTicketPreview: (t: TicketRequest) => api<TicketPreview>('/pmu/ticket/preview', { method: 'POST', body: t, passive: true }),
  pmuBuy: (t: TicketRequest) => api<Booked>('/pmu/buy', { method: 'POST', body: t }),
  pmuSell: (t: TicketRequest) => api<Booked>('/pmu/sell', { method: 'POST', body: t }),
  pmuRepoPreview: (r: RepoRequest) => api<RepoPreview>('/pmu/repo/preview', { method: 'POST', body: r, passive: true }),
  pmuRepo: (r: RepoRequest) => api<Booked>('/pmu/repo', { method: 'POST', body: r }),
  pmuRepoClose: (id: string, on: string) => api<Booked>(`/pmu/repo/${id}/close`, { method: 'POST', body: { on } }),
  pmuRepoRoll: (id: string, body: { on: string; end: string; rate: number; cash?: number | null; kondor_id?: string | null }) => api<Booked>(`/pmu/repo/${id}/roll`, { method: 'POST', body }),
  pmuKondor: (kind: 'lot' | 'repo' | 'sale', id: string, kondor_id: string) => api<Booked>(`/pmu/${kind}/${encodeURIComponent(id)}`, { method: 'PATCH', body: { kondor_id } }),
};
