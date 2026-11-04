// Pre-formatted deal and break rows, and the colour each status wears, ported from the native
// client's state.rs and ui/mod.rs.
import type { BreakKind, BreakView, Deal, DealStatus, MatchStatus, Source } from '../api/types';
import { PRODUCT_LABEL, isFx } from '../api/types';
import { fmtAmount, fmtDateShort, fmtLocal } from './fmt';

export const SOURCES_ORDER: Source[] = ['KONDOR', 'BLOOMBERG', 'QUANTUM', 'INTERNAL'];
export const SOURCE_LETTER: Record<Source, string> = { KONDOR: 'K', BLOOMBERG: 'B', QUANTUM: 'Q', INTERNAL: 'I', USER: 'U' };

export function matchLabel(m: MatchStatus): string {
  return m === 'BREAK' ? 'BREAK' : m.toLowerCase();
}
export function matchClass(m: MatchStatus): string {
  return m === 'MATCHED' ? 't-good' : m === 'PARTIAL' ? 't-warn' : m === 'BREAK' ? 't-bad' : 't-muted';
}
export function statusClass(s: DealStatus): string {
  return s === 'EXECUTED' ? 't-warn' : s === 'BOOKED' ? 't-info' : s === 'CONFIRMED' ? 't-good' : s === 'SETTLED' ? 't-muted' : 't-bad';
}
export function breakKindClass(k: BreakKind): string {
  return k === 'MISSING' ? 't-warn' : k === 'MISMATCH' ? 't-bad' : k === 'ORPHAN' ? 't-violet' : k === 'AMBIGUOUS' ? 't-info' : 't-magenta';
}
export const isTerminal = (s: DealStatus) => s === 'SETTLED' || s === 'CANCELLED';

export function fmtRate(rate: string, fx: boolean): string {
  const n = Number(rate);
  return fx ? n.toFixed(5) : `${n.toFixed(4)}%`;
}

export interface DealRow {
  deal: Deal;
  ref: string;
  desk: string;
  match: string;
  sources: string;
  product: string;
  side: string;
  pair: string;
  notional: string;
  rate: string;
  trade: string;
  value: string;
  maturity: string;
  counterparty: string;
  status: string;
  updated: string;
  trader: string;
  search: string;
}

export function dealRow(d: Deal): DealRow {
  const pair = d.ccy2 ? `${d.ccy1}/${d.ccy2}` : d.ccy1;
  const sources = SOURCES_ORDER.map((s) => (d.sources.includes(s) ? SOURCE_LETTER[s] : '·')).join('');
  const row: DealRow = {
    deal: d,
    ref: d.deal_ref,
    desk: d.desk_code,
    match: matchLabel(d.match_status),
    sources,
    product: PRODUCT_LABEL[d.product],
    side: d.side,
    pair,
    notional: fmtAmount(d.notional),
    rate: fmtRate(d.rate, isFx(d.product)),
    trade: fmtDateShort(d.trade_date),
    value: fmtDateShort(d.value_date),
    maturity: d.maturity_date ? fmtDateShort(d.maturity_date) : '',
    counterparty: d.counterparty,
    status: d.status,
    updated: fmtLocal(d.updated_at),
    trader: d.trader_name,
    search: '',
  };
  row.search = `${row.ref} ${row.pair} ${row.counterparty} ${row.match} ${row.product} ${row.status} ${row.desk}`.toLowerCase();
  return row;
}

export interface BreakRow {
  view: BreakView;
  kind: string;
  dealRef: string;
  source: string;
  field: string;
  expected: string;
  observed: string;
  detected: string;
  status: string;
  search: string;
}

const detail = (b: BreakView, key: string) => {
  const v = b.details[key];
  return v === undefined || v === null ? '' : typeof v === 'string' ? v : JSON.stringify(v);
};

export function breakRow(b: BreakView): BreakRow {
  const row: BreakRow = {
    view: b,
    kind: b.kind,
    dealRef: b.deal_ref ?? '—',
    source: b.source ? `${SOURCE_LETTER[b.source]} ${b.source_ref ?? ''}`.trim() : '',
    field: b.field ?? '',
    expected: detail(b, 'expected'),
    observed: detail(b, 'observed'),
    detected: fmtLocal(b.detected_at),
    status: b.status.toLowerCase(),
    search: '',
  };
  row.search = `${row.kind} ${row.dealRef} ${row.source} ${row.field} ${row.status} ${b.desk_code}`.toLowerCase();
  return row;
}

export const BREAK_EXPLANATION: Record<BreakKind, string> = {
  MISSING: 'expected in this source by the deadline and not seen; it resolves itself when the source delivers',
  MISMATCH: 'the source disagrees with the golden record on this field; dismiss to accept the difference',
  ORPHAN: 'a source record with no golden deal; link it to the deal it belongs to',
  AMBIGUOUS: 'several golden deals fit this record; pick the right one',
  DUPLICATE: 'a second record from a source already linked to this deal; link it elsewhere or dismiss',
};
