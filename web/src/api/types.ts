// The server's JSON, mirrored from crates/domain. Decimals arrive as strings and stay strings
// until they are formatted; dates are ISO (`YYYY-MM-DD` for a date, RFC 3339 for an instant).

export type Product = 'FX_SPOT' | 'FX_FWD' | 'MM_DEPO' | 'MM_LOAN';
export type Side = 'BUY' | 'SELL' | 'LEND' | 'BORROW';
export type DealStatus = 'EXECUTED' | 'BOOKED' | 'CONFIRMED' | 'SETTLED' | 'CANCELLED';
export type MatchStatus = 'UNMATCHED' | 'PARTIAL' | 'MATCHED' | 'BREAK';
export type Source = 'KONDOR' | 'BLOOMBERG' | 'QUANTUM' | 'INTERNAL' | 'USER';
export type BreakKind = 'MISSING' | 'MISMATCH' | 'ORPHAN' | 'AMBIGUOUS' | 'DUPLICATE';
export type BreakStatus = 'OPEN' | 'ACKNOWLEDGED' | 'RESOLVED';
export type Resolution = 'AUTO' | 'LINKED' | 'DISMISSED';
export type ResolveAction = 'LINK' | 'DISMISS';
export type AttentionSeverity = 'CRITICAL' | 'WARNING' | 'INFO';
export type RateScenario = 'BASE' | 'PARALLEL_UP' | 'PARALLEL_DOWN' | 'STEEPENER' | 'FLATTENER' | 'SHORT_UP' | 'SHORT_DOWN' | 'CUSTOM';
export type CompareTo = 'PRIOR' | 'BUDGET';

export const PRODUCT_LABEL: Record<Product, string> = { FX_SPOT: 'FX Spot', FX_FWD: 'FX Fwd', MM_DEPO: 'MM Depo', MM_LOAN: 'MM Loan' };
export const SOURCE_SHORT: Record<Source, string> = { KONDOR: 'K', BLOOMBERG: 'B', QUANTUM: 'Q', INTERNAL: 'I', USER: 'U' };
export const isFx = (p: Product) => p === 'FX_SPOT' || p === 'FX_FWD';

export interface UserInfo {
  id: string;
  username: string;
  display_name: string;
  home_desk_id: number;
}
export interface DeskAccess {
  desk_id: number;
  code: string;
  name: string;
  can_book: boolean;
  can_resolve: boolean;
}
export interface LoginResponse {
  token: string;
  expires_at: string;
  user: UserInfo;
  desks: DeskAccess[];
}
export interface MeResponse {
  user: UserInfo;
  desks: DeskAccess[];
}
export interface ErrorBody {
  error: string;
}

export interface Deal {
  id: string;
  deal_ref: string;
  desk_id: number;
  desk_code: string;
  product: Product;
  side: Side;
  ccy1: string;
  ccy2: string | null;
  notional: string;
  rate: string;
  trade_date: string;
  value_date: string;
  maturity_date: string | null;
  counterparty: string;
  trader_id: string | null;
  trader_name: string;
  status: DealStatus;
  version: number;
  created_at: string;
  updated_at: string;
  origin: Source;
  match_status: MatchStatus;
  confirmed: boolean;
  sources: Source[];
}
export interface DealsResponse {
  deals: Deal[];
  as_of: string;
}
export interface NewDeal {
  desk_id: number;
  product: Product;
  side: Side;
  ccy1: string;
  ccy2: string | null;
  notional: string;
  rate: string;
  trade_date: string;
  value_date: string;
  maturity_date: string | null;
  counterparty: string;
}
export interface AmendDeal {
  counterparty?: string;
  rate?: string;
  notional?: string;
  value_date?: string;
  maturity_date?: string;
}
export interface Position {
  desk_id: number;
  desk_code: string;
  ccy: string;
  net_notional: string;
  deal_count: number;
}

export interface FeedStatus {
  source: Source;
  last_heartbeat_at: string | null;
  last_event_at: string | null;
  last_source_ts: string | null;
  events_total: number;
  duplicates_dropped: number;
}
export interface ReconStatus {
  name: string;
  now: string;
  last_tick_at: string;
  pending: number;
  detail: unknown;
}
export interface FeedsResponse {
  feeds: FeedStatus[];
  recon: ReconStatus | null;
  server_time: string;
}

export interface BreakView {
  id: number;
  kind: BreakKind;
  deal_id: string | null;
  deal_ref: string | null;
  source: Source | null;
  source_ref: string | null;
  field: string | null;
  desk_id: number;
  desk_code: string;
  details: Record<string, unknown>;
  status: BreakStatus;
  detected_at: string;
  acknowledged_at: string | null;
  resolved_at: string | null;
  resolution: Resolution | null;
  note: string | null;
  updated_at: string;
}
export interface BreakActionView {
  at: string;
  user: string | null;
  action: string;
  note?: string | null;
}
export interface BreakDetail {
  break_: BreakView;
  actions: BreakActionView[];
}
export interface ResolveRequest {
  action: ResolveAction;
  deal_id: string | null;
  note: string | null;
}
export interface SourceDealView {
  source: Source;
  source_ref: string;
  deal_id: string | null;
  link_kind: string | null;
  linked_at: string | null;
  lifecycle: DealStatus;
  raw_state: string | null;
  applied_ts: string;
  first_seen_at: string;
  desk_id: number;
  product: Product;
  side: Side;
  ccy1: string;
  ccy2: string | null;
  notional: string;
  rate: string;
  trade_date: string;
  value_date: string;
  maturity_date: string | null;
  counterparty_raw: string;
  counterparty: string;
  trader_ref: string | null;
  cross_refs: unknown;
  updated_at: string;
}
export interface ProvenanceResponse {
  deal: Deal;
  sources: SourceDealView[];
  breaks: BreakView[];
  events: unknown[];
}
export interface FieldChange {
  field: string;
  from: string | null;
  to: string | null;
}
export interface HistoryEntry {
  version: number;
  op: string;
  changed_at: string;
  changed_by: string | null;
  changed_by_role: string;
  changes: FieldChange[];
}

export interface LadderRow {
  date: string;
  confirmed_in: string;
  confirmed_out: string;
  provisional_in: string;
  provisional_out: string;
  net_expected: string;
  cumulative_expected: string;
  deals: number;
  actual_in: string | null;
  actual_out: string | null;
  actual_net: string | null;
  difference: string | null;
}
export interface LadderOpening {
  balance: string;
  statement_date: string;
  kind: string;
  received_at: string;
  bank: string;
  account_no: string;
}
export interface LadderResponse {
  ccy: string;
  currencies: string[];
  from: string;
  to: string;
  liquidity_access: boolean;
  opening: LadderOpening | null;
  rows: LadderRow[];
  generated_at: string;
}
export interface CashFlow {
  deal_id: string;
  deal_ref: string;
  desk_code: string;
  flow_date: string;
  ccy: string;
  amount: string;
  leg: string;
  confirmed: boolean;
  status: DealStatus;
  match_status: MatchStatus;
}
export interface NostroMovement {
  id: number;
  ccy: string;
  value_date: string;
  booked_at: string;
  amount: string;
  reference: string;
  counterparty: string | null;
  deal_ref_hint: string | null;
  source: string;
}

export interface ActivityRow {
  at: string;
  kind: string;
  actor: string;
  summary: string;
  deal_id: string | null;
  break_id: number | null;
  desk_code: string;
}
export type AttentionTarget =
  | { kind: 'deal'; id: string }
  | { kind: 'break'; id: number }
  | { kind: 'breaks' }
  | { kind: 'provisional' }
  | { kind: 'overview' }
  | { kind: 'alm'; tool: string };
export interface AttentionItem {
  severity: AttentionSeverity;
  title: string;
  detail: string;
  since: string | null;
  target: AttentionTarget;
}

export type ServerEvent =
  | { type: 'deal'; op: string; id: string; desk_id: number; version: number }
  | { type: 'break'; id: number; desk_id: number; deal_id: string | null; status: BreakStatus }
  | { type: 'resync' }
  | { type: 'alm'; params_version: number | null; snapshot: string | null };

// ALM overview block (the balance-sheet section of the Overview and the ALM dashboard).
export interface LimitRow {
  metric: string;
  value: number | null;
  unit: string;
  limit: number;
  trigger: number | null;
  floor: boolean;
  breach: boolean;
  warn: boolean;
}
export interface TrendPoint {
  as_of: string;
  value: number | null;
}
export interface NiiPeriodRow {
  period: string;
  actual: number;
  budget: number;
}
export interface GapBucket {
  label: string;
  assets: number;
  liabilities: number;
  gap: number;
  cumulative: number;
  cumulative_pct_assets: number;
  limit_pct: number | null;
  breach: boolean;
}
export interface AlmOverview {
  as_of: string;
  params_version: number;
  reporting_ccy: string;
  lcr: number | null;
  nsfr: number | null;
  nii_12m: number;
  nim: number | null;
  nii_mtd: number;
  budget_mtd: number;
  nii_ytd: number;
  budget_ytd: number;
  gap_1y_pct: number;
  worst_eve_scenario: RateScenario | null;
  worst_eve_loss_pct_tier1: number | null;
  top10_pct: number;
  wholesale_pct: number;
  limits: LimitRow[];
  lcr_trend: TrendPoint[];
  nii_months: NiiPeriodRow[];
  gap_buckets: GapBucket[];
}
export interface AlmSnapshot {
  as_of: string;
  business_date: string;
  generated_at: string;
  positions: number;
}
