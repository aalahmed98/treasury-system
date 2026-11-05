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
  | { kind: 'alm'; tool: string }
  | { kind: 'risk' }
  | { kind: 'ftp' }
  | { kind: 'pmu'; page: string };
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
  | { type: 'alm'; params_version: number | null; snapshot: string | null; ftp?: string | null; pmu?: boolean };

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
  previous: OverviewPrevious | null;
  ratios: Ratios | null;
}
export interface OverviewPrevious {
  as_of: string;
  lcr: number | null;
  nsfr: number | null;
  worst_eve_loss_pct_tier1: number | null;
  top10_pct: number;
  nii_12m: number;
}
export interface Ratios {
  loan_to_deposit: number | null;
  liquid_assets_to_assets: number | null;
  cost_of_funds: number | null;
  asset_yield: number | null;
  net_spread: number | null;
  wholesale_funding: number;
  equity_to_assets: number | null;
}
export interface AlmSnapshot {
  as_of: string;
  business_date: string;
  generated_at: string;
  positions: number;
}

// ---- ALM workbench ----------------------------------------------------------------------------

export type Factors3 = [number, number, number];
export interface LcrParams {
  minimum: number;
  internal_minimum: number;
  early_warning: number;
  significant_ccys: string[];
  haircuts: Record<string, number>;
  cap_l2_total_pct: number;
  cap_l2b_pct: number;
  inflow_cap_pct: number;
  outflow_rates: Record<string, number>;
  inflow_rates: Record<string, number>;
}
export interface NsfrParams {
  minimum: number;
  internal_minimum: number;
  asf: Record<string, Factors3>;
  rsf: Record<string, Factors3>;
  hqla_rsf: Record<string, number>;
  off_balance_rsf_pct: number;
  encumbered_rsf_pct: number;
}
export interface GapParams {
  nmd_behavioural_months: Record<string, number>;
  cumulative_gap_limit_pct_assets: Record<string, number>;
  eve_limit_pct_tier1: number;
  nii_sensitivity_limit_pct: number;
}
export interface RateShock {
  parallel: number;
  short: number;
  long: number;
}
export interface LiquidityStress {
  outflow_multiplier: number;
  extra_hqla_haircut_pct: number;
  inflow_multiplier: number;
}
export interface StressParams {
  shocks: Record<string, RateShock>;
  floor_bp: number;
  floor_slope_bp_per_year: number;
  liquidity: LiquidityStress;
  fx_shock_pct: Record<string, number>;
}
export interface NiiParams {
  horizon_months: number;
  constant_balance_sheet: boolean;
  default_beta: number;
}
export interface FundingParams {
  top1_limit_pct: number;
  top10_limit_pct: number;
  wholesale_share_limit_pct: number;
}
export interface AlmParams {
  reporting_ccy: string;
  tier1_capital: number;
  lcr: LcrParams;
  nsfr: NsfrParams;
  gap: GapParams;
  stress: StressParams;
  nii: NiiParams;
  funding: FundingParams;
  liquidity_gap: LiquidityGapParams;
  ewi: EwiParams;
}
export interface LiquidityGapParams {
  stable_share_pct: Record<string, number>;
  commitment_draw_pct: number;
  cumulative_floor_pct_assets: Record<string, number>;
}
export interface EwiThreshold {
  amber: number;
  red: number;
}
export interface EwiParams {
  thresholds: Record<string, EwiThreshold>;
  actions: Record<string, string[]>;
}
export interface AlmParamsVersion {
  version: number;
  created_at: string;
  created_by: string | null;
  note: string | null;
  is_active: boolean;
}
export interface AlmParamsResponse {
  active: AlmParamsVersion;
  params: AlmParams;
  history: AlmParamsVersion[];
}
export interface AlmRequest {
  as_of?: string | null;
  params?: AlmParams | null;
  scenario?: RateScenario | null;
  custom_shock_bp?: number | null;
  compare_to?: CompareTo | null;
  period?: string | null;
}
export interface LcrLine {
  class: string;
  amount: number;
  rate: number;
  weighted: number;
}
export interface LcrFigures {
  ccy: string;
  hqla_l1: number;
  hqla_l2a: number;
  hqla_l2b: number;
  adjustment_l2b_cap: number;
  adjustment_l2_cap: number;
  stock: number;
  outflows: LcrLine[];
  total_outflows: number;
  inflows: LcrLine[];
  total_inflows: number;
  capped_inflows: number;
  net_outflows: number;
  ratio: number | null;
}
export interface LcrResult {
  as_of: string;
  params_version: number | null;
  reporting_ccy: string;
  minimum: number;
  internal_minimum: number;
  early_warning: number;
  total: LcrFigures;
  by_ccy: LcrFigures[];
}
export interface NsfrLine {
  class: string;
  bucket: string;
  amount: number;
  factor: number;
  weighted: number;
}
export interface NsfrResult {
  as_of: string;
  params_version: number | null;
  reporting_ccy: string;
  minimum: number;
  internal_minimum: number;
  asf_lines: NsfrLine[];
  rsf_lines: NsfrLine[];
  asf: number;
  rsf: number;
  ratio: number | null;
}
export interface GapCurrency {
  ccy: string;
  buckets: GapBucket[];
}
export interface GapResult {
  as_of: string;
  params_version: number | null;
  reporting_ccy: string;
  total_assets: number;
  buckets: GapBucket[];
  by_ccy: GapCurrency[];
}
export interface NiiMonth {
  period: string;
  income: number;
  expense: number;
  nii: number;
}
export interface NiiBreakdown {
  key: string;
  nii: number;
}
export interface NiiResult {
  as_of: string;
  params_version: number | null;
  reporting_ccy: string;
  scenario: RateScenario;
  horizon_months: number;
  months: NiiMonth[];
  income: number;
  expense: number;
  nii: number;
  earning_assets: number;
  nim: number | null;
  by_book: NiiBreakdown[];
  by_ccy: NiiBreakdown[];
  base_nii: number | null;
}
export interface StressScenarioRow {
  scenario: RateScenario;
  label: string;
  nii: number;
  nii_delta: number;
  nii_delta_pct: number | null;
  eve_delta: number;
  eve_loss: number;
  eve_loss_pct_tier1: number | null;
  nii_breach: boolean;
  eve_breach: boolean;
}
export interface SurvivalPoint {
  day: number;
  cumulative_net_outflow: number;
}
export interface LiquidityStressResult {
  lcr_base: number | null;
  lcr_stressed: number | null;
  stock_stressed: number;
  net_outflows_stressed: number;
  survival_days: number | null;
  horizon: SurvivalPoint[];
}
export interface FxStressRow {
  ccy: string;
  shock_pct: number;
  exposure: number;
  pnl: number;
}
export interface StressResult {
  as_of: string;
  params_version: number | null;
  reporting_ccy: string;
  base_nii: number;
  base_eve: number;
  tier1_capital: number;
  nii_limit_pct: number;
  eve_limit_pct_tier1: number;
  rate_scenarios: StressScenarioRow[];
  worst_eve: RateScenario | null;
  liquidity: LiquidityStressResult;
  fx: FxStressRow[];
  uniform_200: StressScenarioRow[];
}
export interface AttributionLine {
  book: string;
  side: 'ASSET' | 'LIABILITY' | 'OFF_BALANCE';
  ccy: string;
  nii_a: number;
  nii_b: number;
  volume_a: number;
  volume_b: number;
  rate_a: number;
  rate_b: number;
  days_effect: number;
  volume_effect: number;
  rate_effect: number;
  residual: number;
}
export interface AttributionStep {
  label: string;
  value: number;
}
export interface AttributionResult {
  period_a: string;
  period_b: string;
  compare_to: CompareTo;
  reporting_ccy: string;
  nii_a: number;
  nii_b: number;
  delta: number;
  days_effect: number;
  pure_volume_effect: number;
  mix_effect: number;
  rate_effect: number;
  residual: number;
  steps: AttributionStep[];
  lines: AttributionLine[];
}
export interface FundingGroup {
  name: string;
  segment: string;
  balance: number;
  share_pct: number;
}
export interface FundingResult {
  as_of: string;
  params_version: number | null;
  reporting_ccy: string;
  total_deposits: number;
  total_funding: number;
  wholesale_funding: number;
  top1_pct: number;
  top10_pct: number;
  wholesale_pct: number;
  top1_limit_pct: number;
  top10_limit_pct: number;
  wholesale_limit_pct: number;
  breaches: string[];
  groups: FundingGroup[];
  by_segment: NiiBreakdown[];
}
export interface CurvePoint {
  tenor: string;
  tenor_days: number;
  rate: number;
}
export interface RateCurve {
  ccy: string;
  index_code: string;
  as_of: string;
  points: CurvePoint[];
}
export interface FxSpot {
  pair: string;
  as_of: string;
  mid: number;
}
export interface MarketResponse {
  curves: RateCurve[];
  spots: FxSpot[];
}

// ---------------------------------------------------------------------------------------------
// Liquidity ladder and early warnings (ALM)
// ---------------------------------------------------------------------------------------------
export interface LiquidityBucket {
  label: string;
  inflows: number;
  outflows: number;
  gap: number;
  cumulative: number;
  cumulative_pct_assets: number;
  floor_pct: number | null;
  breach: boolean;
}
export interface LiquidityCcy {
  ccy: string;
  gaps: number[];
  cumulative: number[];
}
export interface LiquidityLadder {
  buckets: LiquidityBucket[];
  by_ccy: LiquidityCcy[];
  first_negative: string | null;
}
export interface LiquidityGapResult {
  as_of: string;
  params_version: number | null;
  reporting_ccy: string;
  total_assets: number;
  contractual: LiquidityLadder;
  behavioural: LiquidityLadder;
  breaches: string[];
}
export type EwiStatus = 'GREEN' | 'AMBER' | 'RED';
export interface EwiPoint {
  as_of: string;
  value: number | null;
}
export interface EwiIndicator {
  key: string;
  label: string;
  what: string;
  unit: string;
  value: number | null;
  amber: number;
  red: number;
  rising_is_worse: boolean;
  status: EwiStatus;
  series: EwiPoint[];
}
export interface EwiResult {
  as_of: string;
  params_version: number | null;
  stage: 'NORMAL' | 'ALERT' | 'CRISIS';
  indicators: EwiIndicator[];
  actions: string[];
}

// ---------------------------------------------------------------------------------------------
// FX market risk
// ---------------------------------------------------------------------------------------------
export interface CounterpartyLine {
  pre_settlement: number;
  settlement: number;
}
export interface RiskLimits {
  base_ccy: string;
  pegged: string[];
  nop_overnight_total: number;
  nop_intraday_total: number;
  nop_overnight_ccy: Record<string, number>;
  nop_default_floating: number;
  nop_default_pegged: number;
  dealer_nop: number;
  stop_loss_daily: number;
  stop_loss_monthly: number;
  var_limit: number;
  off_market_bp_spot: number;
  off_market_bp_forward: number;
  counterparty_lines: Record<string, CounterpartyLine>;
  default_line: CounterpartyLine;
  warn_pct: number;
}
export interface RiskLimitsVersion {
  version: number;
  created_at: string;
  created_by: string | null;
  note: string | null;
  is_active: boolean;
}
export interface RiskLimitsResponse {
  active: RiskLimitsVersion;
  limits: RiskLimits;
  history: RiskLimitsVersion[];
  can_edit: boolean;
}
export type LimitStatus = 'OK' | 'WATCH' | 'BREACH';
export interface LimitUse {
  key: string;
  label: string;
  value: number;
  limit: number;
  pct: number;
  status: LimitStatus;
}
export interface CcyPosition {
  ccy: string;
  amount: number;
  bhd: number | null;
  pegged: boolean;
  spot: number | null;
  limit: number;
  pct: number;
  status: LimitStatus;
}
export interface FwdLadderRow {
  ccy: string;
  amounts: number[];
  bhd: number[];
}
export interface ForwardLadder {
  buckets: string[];
  rows: FwdLadderRow[];
}
export interface PnlSummary {
  today: number;
  new_deals: number;
  market_move: number;
  mtd: number;
  ytd: number;
  total: number;
  open_mtm: number;
  settled: number;
}
export interface PnlBy {
  key: string;
  today: number;
  mtd: number;
  ytd: number;
  deals: number;
  nop: number;
}
export interface CounterpartyExposure {
  name: string;
  deals: number;
  mtm: number;
  add_on: number;
  pre_settlement: number;
  pre_settlement_line: number;
  pre_settlement_pct: number;
  settlement_peak: number;
  settlement_peak_date: string | null;
  settlement_line: number;
  settlement_pct: number;
  status: LimitStatus;
}
export interface CcyVar {
  ccy: string;
  position: number;
  var99: number;
}
export interface VarResult {
  var99: number;
  es99: number;
  stressed_var99: number;
  stressed_window_end: string | null;
  observations: number;
  by_ccy: CcyVar[];
}
export interface FxDealValue {
  deal_id: string;
  deal_ref: string;
  desk_code: string;
  trader: string;
  counterparty: string;
  pair: string;
  side: string;
  notional: number;
  rate: number;
  trade_date: string;
  value_date: string;
  market_rate: number | null;
  mtm: number | null;
  settled: boolean;
  trade_mid: number | null;
  off_market_bp: number | null;
  off_market: boolean;
  day_one: number | null;
}
export interface FxRiskResult {
  as_of: string;
  base_ccy: string;
  limits_version: number | null;
  positions: CcyPosition[];
  nop_long: number;
  nop_short: number;
  nop_total: number;
  nop_floating: number;
  nop_pegged: number;
  nop_intraday_peak: number;
  ladder: ForwardLadder;
  pnl: PnlSummary;
  by_trader: PnlBy[];
  by_pair: PnlBy[];
  counterparties: CounterpartyExposure[];
  var: VarResult;
  limits: LimitUse[];
  off_market: FxDealValue[];
  unpriced: string[];
  cancelled_30d: number;
}
export interface FxHistoryPoint {
  date: string;
  pnl_day: number;
  pnl_total: number;
  nop_total: number;
  nop_floating: number;
  var99: number;
}
export interface Breach {
  date: string;
  key: string;
  label: string;
  value: number;
  limit: number;
  signed_by: string | null;
  signed_at: string | null;
  note: string | null;
}
export interface AlcoNote {
  body: string;
  updated_by: string | null;
  updated_at: string;
}
export interface AlcoPack {
  period: string;
  notes: Record<string, AlcoNote>;
  signoff: { signed_by: string | null; signed_at: string; note: string | null } | null;
  can_sign: boolean;
}

// FTP: the curves ALM publishes each day from its workbook, read by every desk.
export interface FtpCurve {
  ccy: string;
  points: CurvePoint[];
}
export interface FtpPublication {
  id: number;
  as_of: string;
  published_at: string;
  published_by: string | null;
  file_name: string | null;
  note: string | null;
  superseded: number;
  curves: FtpCurve[];
}
export interface FtpPublicationSummary {
  id: number;
  as_of: string;
  published_at: string;
  published_by: string | null;
  file_name: string | null;
  note: string | null;
  currencies: string[];
  is_current: boolean;
}
export interface FtpCurrent {
  date: string;
  current: FtpPublication | null;
  previous: FtpPublication | null;
  age_days: number | null;
  stale: boolean;
  can_publish: boolean;
  recent: FtpPublicationSummary[];
}
export interface FtpHistory {
  ccy: string;
  tenor: string;
  points: { as_of: string; rate: number }[];
}
export interface SheetIssue {
  sheet: string;
  cell: string;
  message: string;
}
export type FtpChange =
  | { kind: 'added'; ccy: string }
  | { kind: 'missing'; ccy: string }
  | { kind: 'moved'; ccy: string; tenor: string; from: number; to: number }
  | { kind: 'tenors'; ccy: string; added: string[]; removed: string[] };
export interface FtpPreview {
  sheet_date: string | null;
  as_of: string;
  layout: string;
  curves: FtpCurve[];
  fractions: boolean;
  errors: SheetIssue[];
  warnings: SheetIssue[];
  changes: FtpChange[];
  compared_with: string | null;
  previous: FtpCurve[];
  file_sha256: string;
}

// PMU: the bond portfolio. Amounts in BHD unless a field says local; DV01 is the P&L of +1 bp.
export interface PmuIncomeParts {
  coupon: number;
  amortisation: number;
  eir_income: number;
  ftp: number;
  carry: number;
  irs: number;
  net_nii: number;
  repo: number;
  accounting_nii: number;
}
export type RowLevel = 'BOOK' | 'BOND' | 'LOT';
export interface PositionRow {
  level: RowLevel;
  key: string;
  parent: string | null;
  book: string;
  isin: string | null;
  label: string;
  issuer: string | null;
  ccy: string | null;
  kondor_id: string | null;
  trade_date: string | null;
  maturity: string | null;
  lots: number;
  face_local: number | null;
  face_bhd: number;
  price: number;
  market_price: number | null;
  market_value_bhd: number;
  book_value_bhd: number;
  mtm_bhd: number;
  eir: number;
  ytm: number | null;
  ftp_rate: number | null;
  dv01_bhd: number;
  swap_dv01_bhd: number;
  mod_duration: number;
  hedged_face_bhd: number;
  pledged_face_bhd: number;
  share: number;
  income: PmuIncomeParts;
  daily_carry_bhd: number;
}
export interface SwapView {
  id: string;
  book: string;
  kondor_id: string | null;
  ccy: string;
  notional: number;
  closed_notional: number;
  start: string;
  maturity: string;
  pay_fixed: boolean;
  fixed_rate: number;
  float_index: string;
  float_rate: number;
  mtm_bhd: number;
  dv01_bhd: number;
  isins: string[];
}
export interface HedgePair {
  isin: string;
  name: string;
  ccy: string;
  face_bhd: number;
  hedged_face_bhd: number;
  bond_dv01: number;
  irs_dv01: number;
  net_dv01: number;
  bond_mtm: number;
  irs_mtm: number;
  net_mtm: number;
  closed_notional: number;
  swaps: number;
}
export interface BookFigure {
  book: string;
  name: string;
  accounting: 'AFS' | 'HTM' | 'FVTPL';
  face_bhd: number;
  market_value_bhd: number;
  book_value_bhd: number;
  mtm_bhd: number;
  dv01_bhd: number;
  net_dv01_bhd: number;
  mod_duration: number;
  yield_pct: number;
  daily_carry_bhd: number;
}
export interface HedgeBucket {
  label: string;
  hedged_bhd: number;
  unhedged_bhd: number;
}
export interface RepoSummary {
  amount_bhd: number;
  available_face_bhd: number;
  pledged_face_bhd: number;
  due_7d_bhd: number;
  active: number;
  avg_rate: number;
  avg_haircut: number;
  wtd_maturity_days: number;
  receipts_12m_bhd: number;
}
export interface PmuHeadline {
  portfolio_face_bhd: number;
  market_value_bhd: number;
  book_value_bhd: number;
  net_mtm_bhd: number;
  mtd_carry_bhd: number;
  net_dv01_bhd: number;
  mtd_nii_bhd: number;
  portfolio_yield: number;
  mod_duration: number;
  lots: number;
  kondor_pct: number;
  daily_carry_bhd: number;
}
export interface MonthIncome {
  month: string;
  actual: boolean;
  income: PmuIncomeParts;
}
export interface PmuOverview {
  as_of: string;
  previous: string | null;
  headline: PmuHeadline;
  headline_prev: PmuHeadline | null;
  by_book: BookFigure[];
  monthly: MonthIncome[];
  maturity_now: HedgeBucket[];
  maturity_year_end: HedgeBucket[];
  concentration: PositionRow[];
  repo: RepoSummary;
  warnings: { key: string; message: string }[];
}
export interface PmuPositions {
  as_of: string;
  from: string;
  to: string;
  rows: PositionRow[];
  swaps: SwapView[];
  headline: PmuHeadline;
}
export interface NamedIncome {
  key: string;
  label: string;
  income: PmuIncomeParts;
}
export interface PmuIncome {
  from: string;
  to: string;
  total: PmuIncomeParts;
  daily: { date: string; income: PmuIncomeParts }[];
  by_bond: NamedIncome[];
  by_book: NamedIncome[];
}
export interface BookYear {
  book: string;
  actual: PmuIncomeParts;
  forecast: PmuIncomeParts;
}
export interface PmuForecast {
  as_of: string;
  reinvest: boolean;
  full_year: PmuIncomeParts;
  year_to_date: PmuIncomeParts;
  rest_of_year: PmuIncomeParts;
  monthly: MonthIncome[];
  periods: { label: string; from: string; to: string; partial: boolean; income: PmuIncomeParts }[];
  by_book: BookYear[];
  year_end_face_bhd: number;
  year_end_hedged_face_bhd: number;
}
export interface PmuRisk {
  as_of: string;
  hedge_pairs: HedgePair[];
  unhedged: PositionRow[];
  dv01_buckets: { label: string; bond: number; irs: number; net: number }[];
  books: string[];
  shocks: { shift_bp: number; by_book: number[]; total: number }[];
  by_book: BookFigure[];
  yield_by_year: { book: string | null; year: number; face_bhd: number; yield_all: number; yield_unhedged: number | null }[];
  hedged_by_ccy: { ccy: string; hedged_bhd: number; unhedged_bhd: number }[];
  maturity_now: HedgeBucket[];
  maturity_year_end: HedgeBucket[];
  headline: PmuHeadline;
  year_end_face_bhd: number;
  year_end_hedged_face_bhd: number;
}
export interface RepoView {
  id: string;
  counterparty: string;
  kondor_id: string | null;
  ccy: string;
  start: string;
  end: string;
  cash_bhd: number;
  rate: number;
  haircut_pct: number;
  days_left: number;
  collateral_face_bhd: number;
  isins: string[];
}
export interface MonthFlow {
  month: string;
  a: number;
  b: number;
}
export interface PmuFunding {
  as_of: string;
  summary: RepoSummary;
  repos: RepoView[];
  by_remaining: { label: string; amount: number }[];
  funding_flows: MonthFlow[];
  collateral_by_book: { book: string; pledged_bhd: number; free_bhd: number }[];
  quotes: { counterparty: string; tenor: string; tenor_days: number; spread_bp: number; as_of: string; age_days: number; sofr: number | null }[];
  receipts: MonthFlow[];
  hedge_cash: MonthFlow[];
  by_book_year: BookYear[];
}
export interface PmuBondDetail {
  bond: {
    isin: string;
    name: string;
    issuer: string;
    issuer_type: string;
    ccy: string;
    coupon: number;
    coupon_freq: number;
    day_count: string;
    issue_date: string;
    maturity: string;
    rate_type: 'FIXED' | 'FLOAT';
    float_index: string | null;
    float_spread: number;
    hqla_level: string | null;
  };
  lots: PositionRow[];
  swaps: SwapView[];
  repos: RepoView[];
  cash_flows: { date: string; kind: string; amount_local: number; amount_bhd: number }[];
}
