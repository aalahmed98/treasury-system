// The deal blotter: every deal the user's desks may see, with match status and provenance, a
// detail pane, booking in a side sheet and amendment. The selected deal lives in the address, so a
// link to it can be shared and the back button returns to it.
import { useCallback, useDeferredValue, useEffect, useMemo, useState, type FormEvent } from 'react';
import { useSearchParams } from 'react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ApiError, endpoints } from '../api/client';
import { useDeals, useRiskLimits } from '../api/queries';
import { useSession } from '../api/session';
import { useLive } from '../api/live';
import type { Deal, DeskAccess, NewDeal, Product, Side } from '../api/types';
import { PRODUCT_LABEL, isFx } from '../api/types';
import { fmtAmount, fmtDateShort, fmtLocal, titleCase } from '../lib/fmt';
import { breakKindClass, dealRow, fmtRate, isTerminal, matchClass, matchLabel, statusClass, type DealRow } from '../lib/deals';
import { marketOutright } from '../lib/ird';
import { Badge, Chip } from '../components/ui';
import { DataTable, byNumber, byString, exportCsv, useSort, useTableView, type Col } from '../components/table';
import { SideToggle, WithSide } from '../components/positions';
import { Empty, Kbd, useToast } from '../components/feedback';

const COLS: Col<DealRow>[] = [
  { id: 'ref', header: 'Ref', width: 96, fixed: true, sort: byString((r) => r.ref), cell: (r) => r.ref },
  { id: 'desk', header: 'Desk', width: 52, sort: byString((r) => r.desk), cell: (r) => r.desk },
  { id: 'match', header: 'Match', width: 92, sort: byString((r) => r.match), cell: (r) => <span className={matchClass(r.deal.match_status)}>{r.match}</span>, csv: (r) => r.match },
  { id: 'cf', header: 'Conf.', width: 48, title: 'confirmed by the back office', cell: (r) => (r.deal.confirmed ? <span className="t-good">✔</span> : <span className="t-muted">·</span>), csv: (r) => (r.deal.confirmed ? 'yes' : 'no') },
  { id: 'src', header: 'Sources', width: 64, title: 'which systems have seen the deal: K Kondor, B Bloomberg, Q Quantum, I internal booking', cell: (r) => r.sources },
  { id: 'product', header: 'Product', width: 70, sort: byString((r) => r.product), cell: (r) => r.product },
  { id: 'side', header: 'Side', width: 60, cell: (r) => r.side },
  { id: 'pair', header: 'Pair', width: 80, sort: byString((r) => r.pair), cell: (r) => r.pair },
  { id: 'notional', header: 'Notional', width: 112, num: true, sort: byNumber((r) => Number(r.deal.notional)), cell: (r) => r.notional },
  { id: 'rate', header: 'Rate', width: 84, num: true, cell: (r) => r.rate },
  { id: 'trade', header: 'Trade', width: 94, sort: byString((r) => r.deal.trade_date), cell: (r) => r.trade },
  { id: 'value', header: 'Value', width: 94, sort: byString((r) => r.deal.value_date), cell: (r) => r.value },
  { id: 'cpty', header: 'Counterparty', width: 190, sort: byString((r) => r.counterparty), cell: (r) => r.counterparty },
  { id: 'status', header: 'Status', width: 96, sort: byString((r) => r.status), cell: (r) => <span className={statusClass(r.deal.status)}>{r.status.toLowerCase()}</span>, csv: (r) => r.status },
  { id: 'updated', header: 'Updated', width: 130, sort: byString((r) => r.deal.updated_at), cell: (r) => <span className="t-muted">{r.updated}</span>, csv: (r) => r.updated },
];

type Filter = 'breaks' | 'provisional' | 'unmatched';
const FILTERS: [Filter, string, string][] = [
  ['breaks', 'with breaks', 'deals with an open break'],
  ['provisional', 'provisional', 'not yet confirmed by the back office'],
  ['unmatched', 'unmatched', 'not yet seen in every expected source'],
];
const passes = (r: DealRow, f: Filter) => (f === 'breaks' ? r.deal.match_status === 'BREAK' : f === 'provisional' ? !r.deal.confirmed && r.deal.status !== 'CANCELLED' : r.deal.match_status === 'UNMATCHED' || r.deal.match_status === 'PARTIAL');

export function Blotter() {
  const { desks, bookableDesks } = useSession();
  const deals = useDeals();
  const live = useLive();
  const toast = useToast();
  const [params, setParams] = useSearchParams();
  const [desk, setDesk] = useState<number | null>(null);
  const [search, setSearch] = useState('');
  const needle = useDeferredValue(search.trim().toLowerCase());
  const [filters, setFilters] = useState<Set<Filter>>(() => new Set());
  const [sort, setSort] = useSort({ id: 'updated', desc: true });
  const [ticket, setTicket] = useState(false);
  const { columns, rowHeight, menu } = useTableView('blotter', COLS);
  const selected = params.get('deal');

  const select = useCallback(
    (id: string | null) => {
      setParams(
        (p) => {
          const n = new URLSearchParams(p);
          if (id) n.set('deal', id);
          else n.delete('deal');
          return n;
        },
        { replace: true },
      );
    },
    [setParams],
  );

  // One-shot instructions in the address: a filter from the overview, the ticket from the palette.
  useEffect(() => {
    const filter = params.get('filter') as Filter | null;
    const book = params.get('book');
    if (filter || book) {
      if (filter && FILTERS.some(([f]) => f === filter)) setFilters(new Set([filter]));
      if (book) setTicket(true);
      setParams(
        (p) => {
          const n = new URLSearchParams(p);
          n.delete('filter');
          n.delete('book');
          return n;
        },
        { replace: true },
      );
    }
  }, [params, setParams]);

  // `n` books a deal when the user is not typing.
  useEffect(() => {
    if (!bookableDesks.length) return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (e.key !== 'n' || e.ctrlKey || e.metaKey || e.altKey || (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT'))) return;
      e.preventDefault();
      setTicket(true);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [bookableDesks.length]);

  const rows = useMemo(() => (deals.data?.deals ?? []).map(dealRow), [deals.data]);
  const counts = useMemo(() => Object.fromEntries(FILTERS.map(([f]) => [f, rows.filter((r) => passes(r, f)).length])) as Record<Filter, number>, [rows]);
  const filtered = useMemo(() => rows.filter((r) => (desk === null || r.deal.desk_id === desk) && (!needle || r.search.includes(needle)) && [...filters].every((f) => passes(r, f))), [rows, desk, needle, filters]);
  const current = selected ? rows.find((r) => r.deal.id === selected) : undefined;
  const toggleFilter = (f: Filter) =>
    setFilters((s) => {
      const n = new Set(s);
      if (n.has(f)) n.delete(f);
      else n.add(f);
      return n;
    });
  const clear = () => {
    setFilters(new Set());
    setSearch('');
    setDesk(null);
  };
  const narrowed = filters.size > 0 || needle !== '' || desk !== null;

  return (
    <WithSide
      page="blotter"
      ready={deals.data !== undefined}
      sheet={
        ticket ? (
          <Ticket
            desks={bookableDesks}
            onClose={() => setTicket(false)}
            onBooked={(d) => {
              setTicket(false);
              select(d.id);
              toast({ tone: 'good', title: `Booked ${d.deal_ref}`, detail: `${PRODUCT_LABEL[d.product]} ${d.side} ${d.ccy2 ? `${d.ccy1}/${d.ccy2}` : d.ccy1} ${fmtAmount(d.notional)} with ${d.counterparty}. It is in the reconciliation log now.` });
            }}
          />
        ) : undefined
      }
    >
      <div className="toolbar">
        <h1 className="t-title" style={{ marginRight: 4 }}>
          Deal blotter
        </h1>
        <select className="input input-sm" value={desk ?? ''} onChange={(e) => setDesk(e.target.value === '' ? null : Number(e.target.value))} aria-label="Desk">
          <option value="">All desks</option>
          {desks.map((d) => (
            <option key={d.desk_id} value={d.desk_id}>
              {d.code}
            </option>
          ))}
        </select>
        <input className="input input-sm" style={{ width: 250 }} placeholder="search ref, pair, counterparty, match…  /" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Search deals" data-search />
        {FILTERS.map(([f, label, title]) => (
          <Chip key={f} outlined selected={filters.has(f)} onClick={() => toggleFilter(f)} title={title}>
            {label}
            {counts[f] > 0 && <span className="t-small" style={{ marginLeft: 5, color: 'inherit', opacity: 0.75 }}>{counts[f]}</span>}
          </Chip>
        ))}
        {bookableDesks.length > 0 && (
          <button type="button" className="btn btn-primary" disabled={ticket} onClick={() => setTicket(true)} title="open a booking ticket (n)">
            Book deal
          </button>
        )}
        <span className="spacer" />
        {menu}
        <button type="button" className="btn btn-ghost btn-sm" onClick={() => exportCsv('blotter', columns, filtered)} disabled={filtered.length === 0}>
          Export CSV
        </button>
        <SideToggle />
      </div>
      <DataTable
        columns={columns}
        rows={filtered}
        rowKey={(r) => r.deal.id}
        selected={selected}
        onSelect={(r, via) => select(via === 'click' && selected === r.deal.id ? null : r.deal.id)}
        onEscape={() => select(null)}
        rowClass={(r) => (live.flash.deals.has(r.deal.id) ? 'flash' : '')}
        sort={sort}
        onSort={setSort}
        rowHeight={rowHeight}
        testId="blotter"
        loading={deals.isLoading}
        empty={
          deals.isError ? (
            <Empty tone="warn" title="The deals could not be loaded" detail={deals.error instanceof Error ? deals.error.message : 'the server did not answer'}>
              <button type="button" className="btn btn-sm" onClick={() => void deals.refetch()}>
                Try again
              </button>
            </Empty>
          ) : narrowed ? (
            <Empty title="No deals match" detail="Widen the search or drop a filter.">
              <button type="button" className="btn btn-sm" onClick={clear}>
                Clear filters
              </button>
            </Empty>
          ) : (
            <Empty title="No deals on your desks yet" detail={bookableDesks.length ? 'Book the first one with the button above, or press n.' : 'Deals arrive from Kondor, Bloomberg and Quantum as the reconciler sees them.'} />
          )
        }
      />
      {current ? (
        <Detail row={current} onClose={() => select(null)} />
      ) : (
        <div className="hint t-small">
          <span className="t-secondary">
            {filtered.length} of {rows.length} deals
          </span>
          {' · '}
          <Kbd>↑</Kbd> <Kbd>↓</Kbd> move · <Kbd>Enter</Kbd> or click opens a deal · <Kbd>Esc</Kbd> closes it
        </div>
      )}
    </WithSide>
  );
}

type DetailTab = 'summary' | 'sources' | 'history';

function Detail({ row, onClose }: { row: DealRow; onClose: () => void }) {
  const [tab, setTab] = useState<DetailTab>('summary');
  const d = row.deal;
  return (
    <div className="detail" data-testid="deal-detail">
      <div className="head">
        <span className="t-heading">{row.ref}</span>
        <span>
          {row.product} {row.side} {row.pair}
        </span>
        <span className="t-medium">{row.notional}</span>
        <span className="t-muted">@</span>
        <span>{row.rate}</span>
        <Badge tone={d.status === 'CONFIRMED' ? 'good' : d.status === 'CANCELLED' ? 'bad' : d.status === 'EXECUTED' ? 'warn' : 'info'} dot>
          {row.status.toLowerCase()}
        </Badge>
        <Badge tone={d.match_status === 'MATCHED' ? 'good' : d.match_status === 'BREAK' ? 'bad' : d.match_status === 'PARTIAL' ? 'warn' : 'neutral'} dot>
          {matchLabel(d.match_status)}
        </Badge>
        {d.confirmed ? <Badge tone="good">confirmed</Badge> : <Badge tone="warn">provisional</Badge>}
        <span className="t-small" title="the systems that have seen this deal">{d.sources.length ? d.sources.map((s) => titleCase(s)).join(', ') : 'no source yet'}</span>
        <span className="t-small">origin {d.origin.toLowerCase()}</span>
        <span className="t-small">v{d.version}</span>
        <span style={{ flex: 1 }} />
        <div className="chips">
          {(['summary', 'sources', 'history'] as DetailTab[]).map((t) => (
            <Chip key={t} small selected={tab === t} onClick={() => setTab(t)}>
              {t === 'summary' ? 'Summary' : t === 'sources' ? 'Sources' : 'History'}
            </Chip>
          ))}
        </div>
        <button type="button" className="btn btn-ghost btn-sm" onClick={onClose} aria-label="close" title="close (Esc)">
          ✕
        </button>
      </div>
      {tab === 'summary' && <Summary row={row} />}
      {tab === 'sources' && <Sources id={d.id} />}
      {tab === 'history' && <History id={d.id} />}
    </div>
  );
}

function Summary({ row }: { row: DealRow }) {
  const d = row.deal;
  return (
    <>
      <div className="facts">
        <span>
          <span className="k">Trader</span>
          {row.trader || '—'}
        </span>
        <span>
          <span className="k">Counterparty</span>
          {row.counterparty}
        </span>
        <span>
          <span className="k">Trade / value / maturity</span>
          {row.trade} / {row.value} / {row.maturity || '—'}
        </span>
        <span>
          <span className="k">Created</span>
          {fmtLocal(d.created_at)}
        </span>
        <span>
          <span className="k">Updated</span>
          {row.updated}
        </span>
        <span className="t-small">
          <span className="k">Id</span>
          {d.id}
        </span>
      </div>
      <Amend deal={d} />
    </>
  );
}

function Amend({ deal }: { deal: Deal }) {
  const { canBook } = useSession();
  const qc = useQueryClient();
  const toast = useToast();
  const [open, setOpen] = useState(false);
  const [form, setForm] = useState({ counterparty: '', rate: '', notional: '', value_date: '', maturity_date: '', confirmCancel: false });
  const [error, setError] = useState<string | null>(null);
  const editable = deal.origin === 'INTERNAL' && canBook(deal.desk_id) && !isTerminal(deal.status);
  const amend = useMutation({
    mutationFn: (req: Parameters<typeof endpoints.amend>[1]) => endpoints.amend(deal.id, req),
    onSuccess: (d) => {
      setOpen(false);
      void qc.invalidateQueries({ queryKey: ['deals'] });
      toast({ tone: 'good', title: `Amended ${d.deal_ref}`, detail: `Now version ${d.version}; the reconciler re-checks it against its sources.` });
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'the amendment failed'),
  });
  const cancel = useMutation({
    mutationFn: () => endpoints.cancel(deal.id),
    onSuccess: (d) => {
      setOpen(false);
      void qc.invalidateQueries({ queryKey: ['deals'] });
      toast({ tone: 'warn', title: `Cancelled ${d.deal_ref}` });
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'the cancellation failed'),
  });
  if (!editable) {
    if (deal.origin !== 'INTERNAL') return <div className="t-small">created by reconciliation from an external source; its economics follow that source</div>;
    if (isTerminal(deal.status)) return <div className="t-small">{deal.status.toLowerCase()} deals cannot be changed</div>;
    return null;
  }
  if (!open) {
    return (
      <button
        type="button"
        className="btn btn-sm"
        onClick={() => {
          setForm({ counterparty: deal.counterparty, rate: String(Number(deal.rate)), notional: String(Number(deal.notional)), value_date: deal.value_date, maturity_date: deal.maturity_date ?? '', confirmCancel: false });
          setError(null);
          setOpen(true);
        }}
      >
        Amend
      </button>
    );
  }
  const save = () => {
    const req: Record<string, string> = {};
    if (form.counterparty.trim() !== deal.counterparty) req.counterparty = form.counterparty.trim();
    if (!/^-?\d+(\.\d+)?$/.test(form.rate.trim())) return setError('rate is not a number');
    if (Number(form.rate) !== Number(deal.rate)) req.rate = form.rate.trim();
    const notional = form.notional.trim().replace(/,/g, '');
    if (!/^\d+(\.\d+)?$/.test(notional)) return setError('notional is not a number');
    if (Number(notional) !== Number(deal.notional)) req.notional = notional;
    if (!/^\d{4}-\d{2}-\d{2}$/.test(form.value_date)) return setError('value date must be YYYY-MM-DD');
    if (form.value_date !== deal.value_date) req.value_date = form.value_date;
    if (form.maturity_date.trim()) {
      if (!/^\d{4}-\d{2}-\d{2}$/.test(form.maturity_date)) return setError('maturity date must be YYYY-MM-DD');
      if (form.maturity_date !== deal.maturity_date) req.maturity_date = form.maturity_date;
    }
    if (Object.keys(req).length === 0) return setError('nothing changed');
    setError(null);
    amend.mutate(req);
  };
  return (
    <div className="stack" style={{ marginTop: 4 }}>
      <div className="row-wrap" style={{ alignItems: 'center' }}>
        <label className="row">
          <span className="t-secondary">Counterparty</span>
          <input className="input input-sm" style={{ width: 200 }} value={form.counterparty} onChange={(e) => setForm({ ...form, counterparty: e.target.value })} />
        </label>
        <label className="row">
          <span className="t-secondary">Rate</span>
          <input className="input input-sm input-num" value={form.rate} onChange={(e) => setForm({ ...form, rate: e.target.value })} />
        </label>
        <label className="row">
          <span className="t-secondary">Notional</span>
          <input className="input input-sm" style={{ width: 120 }} value={form.notional} onChange={(e) => setForm({ ...form, notional: e.target.value })} />
        </label>
        <label className="row">
          <span className="t-secondary">Value date</span>
          <input className="input input-sm" type="date" value={form.value_date} onChange={(e) => setForm({ ...form, value_date: e.target.value })} />
        </label>
        {!isFx(deal.product) && (
          <label className="row">
            <span className="t-secondary">Maturity</span>
            <input className="input input-sm" type="date" value={form.maturity_date} onChange={(e) => setForm({ ...form, maturity_date: e.target.value })} />
          </label>
        )}
      </div>
      <div className="row">
        <button type="button" className="btn btn-primary btn-sm" onClick={save} disabled={amend.isPending}>
          Save amendment
        </button>
        <button type="button" className="btn btn-sm" onClick={() => setOpen(false)}>
          Discard
        </button>
        <span style={{ width: 8 }} />
        <label className="row">
          <input type="checkbox" checked={form.confirmCancel} onChange={(e) => setForm({ ...form, confirmCancel: e.target.checked })} /> I want to cancel this deal
        </label>
        <button type="button" className="btn btn-sm t-bad" disabled={!form.confirmCancel || cancel.isPending} onClick={() => cancel.mutate()}>
          Cancel deal
        </button>
      </div>
      {error && (
        <div className="notice bad" role="alert">
          {error}
        </div>
      )}
    </div>
  );
}

function Sources({ id }: { id: string }) {
  const q = useQuery({ queryKey: ['provenance', id], queryFn: () => endpoints.provenance(id) });
  if (!q.data) return <Retry q={q} what="the sources" />;
  const g = q.data.deal;
  return (
    <>
      <table className="table">
        <thead>
          <tr>
            {['Source', 'Ref', 'Lifecycle', 'Link', 'Notional', 'Rate', 'Value', 'Seen', 'Counterparty (as sent → canonical)'].map((h, i) => (
              <th key={h} className={i === 4 || i === 5 ? 'num' : ''}>
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {q.data.sources.map((s) => {
            const notionalOk = Math.abs(Number(s.notional) - Number(g.notional)) <= Math.abs(Number(g.notional)) * 1e-4;
            const rateOk = Math.abs(Number(s.rate) - Number(g.rate)) <= (isFx(g.product) ? 5e-5 : 1e-2);
            const valueOk = s.value_date === g.value_date;
            const cancelDisagree = (s.lifecycle === 'CANCELLED') !== (g.status === 'CANCELLED');
            const same = s.counterparty === g.counterparty.toUpperCase();
            return (
              <tr key={`${s.source}-${s.source_ref}`}>
                <td className="t-medium">{s.source}</td>
                <td>{s.source_ref}</td>
                <td className={cancelDisagree ? 't-bad' : statusClass(s.lifecycle)}>{s.lifecycle}</td>
                <td className="t-muted">{s.link_kind ?? '—'}</td>
                <td className={`num ${notionalOk ? '' : 't-bad'}`}>{fmtAmount(s.notional)}</td>
                <td className={`num ${rateOk ? '' : 't-bad'}`}>{fmtRate(s.rate, isFx(s.product))}</td>
                <td className={valueOk ? '' : 't-bad'}>{fmtDateShort(s.value_date)}</td>
                <td className="t-muted">{fmtLocal(s.first_seen_at)}</td>
                <td className={same ? '' : 't-warn'}>
                  {s.counterparty_raw} → {s.counterparty}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {q.data.breaks.length === 0 ? (
        <div className="t-muted" style={{ marginTop: 4 }}>
          no breaks on this deal
        </div>
      ) : (
        <div className="stack" style={{ marginTop: 6 }}>
          {q.data.breaks.map((b) => (
            <div key={b.id} className="row">
              <span className={`t-medium ${breakKindClass(b.kind)}`}>{b.kind}</span>
              <span className="t-muted">#{b.id}</span>
              {b.source && (
                <span>
                  {b.source} {b.source_ref ?? ''}
                </span>
              )}
              {b.field && (
                <span>
                  {b.field}: expected {String(b.details.expected ?? '?')} observed {String(b.details.observed ?? '?')}
                </span>
              )}
              <span className="t-muted">{b.status.toLowerCase()}</span>
              {b.resolution && <span className="t-muted">{b.resolution.toLowerCase()}</span>}
            </div>
          ))}
        </div>
      )}
    </>
  );
}

function History({ id }: { id: string }) {
  const q = useQuery({ queryKey: ['history', id], queryFn: () => endpoints.history(id) });
  if (!q.data) return <Retry q={q} what="the history" />;
  const role = (r: string) => (r === 'treasury_recon' ? 'reconciliation' : r === 'treasury_app' ? 'a user' : r === 'treasury_owner' ? 'administrator' : 'system');
  return (
    <div className="stack">
      {[...q.data].reverse().map((e) => (
        <div key={e.version} className="row-wrap" style={{ gap: '2px 8px', alignItems: 'baseline' }}>
          <span className="t-medium">v{e.version}</span>
          <span className="t-muted">{e.op.toLowerCase()}</span>
          <span className="t-muted">{fmtLocal(e.changed_at)}</span>
          <span className={e.changed_by ? '' : 't-muted'}>{e.changed_by ?? role(e.changed_by_role)}</span>
          {e.changes.length === 0 && <span className="t-small">{e.op === 'INSERT' ? 'booked' : 'no tracked field changed'}</span>}
          {e.changes.map((c) => (
            <span key={c.field} className="t-small" style={{ color: 'var(--text-2)' }}>
              {c.field}: {c.from ?? '—'} → {c.to ?? '—'}
            </span>
          ))}
        </div>
      ))}
    </div>
  );
}

/** A loading line, or the failure with a way to try again. */
function Retry({ q, what }: { q: { isError: boolean; error: unknown; refetch: () => unknown }; what: string }) {
  if (!q.isError) return <div className="t-muted">loading {what}…</div>;
  return (
    <div className="row">
      <span className="t-bad">{what} could not be loaded{q.error instanceof Error ? `: ${q.error.message}` : ''}</span>
      <button type="button" className="btn btn-sm" onClick={() => void q.refetch()}>
        Try again
      </button>
    </div>
  );
}

const PRODUCTS: Product[] = ['FX_SPOT', 'FX_FWD', 'MM_DEPO', 'MM_LOAN'];
/** The currencies the book deals in; anything else is almost certainly a typo. */
const CURRENCIES = new Set(['BHD', 'USD', 'EUR', 'GBP', 'SAR', 'AED', 'KWD', 'QAR', 'OMR', 'JPY', 'CHF', 'INR', 'CNY', 'AUD', 'CAD', 'SGD', 'HKD', 'SEK', 'NOK', 'DKK', 'TRY', 'EGP', 'JOD', 'MYR', 'NZD', 'ZAR', 'PKR']);

/** The booking ticket, in the side column so the blotter stays in view while it is filled in. */
function Ticket({ desks, onClose, onBooked }: { desks: DeskAccess[]; onClose: () => void; onBooked: (d: Deal) => void }) {
  const qc = useQueryClient();
  const today = new Date().toISOString().slice(0, 10);
  const plus = (days: number) => new Date(Date.now() + days * 86_400_000).toISOString().slice(0, 10);
  const first = desks[0];
  const mm = first?.code === 'MM';
  const [f, setF] = useState({
    desk_id: first?.desk_id ?? 0,
    product: (mm ? 'MM_DEPO' : 'FX_SPOT') as Product,
    side: (mm ? 'LEND' : 'BUY') as Side,
    ccy1: mm ? 'BHD' : 'USD',
    ccy2: mm ? '' : 'BHD',
    notional: '1000000',
    rate: mm ? '5.25' : '0.37600',
    trade_date: today,
    value_date: plus(2),
    maturity_date: mm ? plus(32) : '',
    counterparty: '',
  });
  const [error, setError] = useState<string | null>(null);
  const [offMarketOk, setOffMarketOk] = useState(false);
  const market = useQuery({ queryKey: ['market', 'rates'], queryFn: endpoints.marketRates, staleTime: 60_000 });
  const limits = useRiskLimits();
  // The market for the deal's value date, and how far the rate typed is from it.
  const check = useMemo(() => {
    if (!isFx(f.product) || !market.data) return null;
    const c1 = f.ccy1.trim().toUpperCase();
    const c2 = f.ccy2.trim().toUpperCase();
    const rate = Number(f.rate);
    if (c1.length !== 3 || c2.length !== 3 || !Number.isFinite(rate) || rate <= 0) return null;
    const days = Math.round((Date.parse(f.value_date) - Date.parse(today)) / 86_400_000);
    const mid = marketOutright(market.data, c1, c2, Number.isFinite(days) ? days : 2);
    if (mid === null) return null;
    const bp = (rate / mid - 1) * 10_000;
    const tolerance = f.product === 'FX_FWD' ? (limits.data?.limits.off_market_bp_forward ?? 100) : (limits.data?.limits.off_market_bp_spot ?? 50);
    return { mid, bp, tolerance, off: Math.abs(bp) > tolerance };
  }, [f.product, f.ccy1, f.ccy2, f.rate, f.value_date, today, market.data, limits.data]);
  useEffect(() => setOffMarketOk(false), [check?.off]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  const book = useMutation({
    mutationFn: (deal: NewDeal) => endpoints.book(deal),
    onSuccess: (d) => {
      void qc.invalidateQueries({ queryKey: ['deals'] });
      void qc.invalidateQueries({ queryKey: ['positions'] });
      onBooked(d);
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'the booking failed'),
  });
  const fx = isFx(f.product);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const ccy = (s: string, name: string) => {
      const c = s.trim().toUpperCase();
      if (!/^[A-Z]{3}$/.test(c)) throw new Error(`${name} must be a 3-letter code such as USD`);
      if (!CURRENCIES.has(c)) throw new Error(`${c} is not a currency the book deals in; expected BHD, USD, EUR, GBP, SAR, AED or another ISO code`);
      return c;
    };
    try {
      const notional = f.notional.trim().replace(/,/g, '');
      if (!/^\d+(\.\d+)?$/.test(notional) || Number(notional) <= 0) throw new Error('notional must be a positive number');
      if (!/^-?\d+(\.\d+)?$/.test(f.rate.trim())) throw new Error('rate is not a number');
      if (f.value_date < f.trade_date) throw new Error('value date is before the trade date');
      let maturity: string | null = null;
      if (!fx) {
        if (!/^\d{4}-\d{2}-\d{2}$/.test(f.maturity_date)) throw new Error('maturity date must be YYYY-MM-DD');
        if (f.maturity_date < f.value_date) throw new Error('maturity date is before the value date');
        maturity = f.maturity_date;
      }
      if (!f.counterparty.trim()) throw new Error('counterparty is required');
      const deal: NewDeal = {
        desk_id: f.desk_id,
        product: f.product,
        side: f.side,
        ccy1: ccy(f.ccy1, 'ccy1'),
        ccy2: fx ? ccy(f.ccy2, 'ccy2') : null,
        notional,
        rate: f.rate.trim(),
        trade_date: f.trade_date,
        value_date: f.value_date,
        maturity_date: maturity,
        counterparty: f.counterparty.trim(),
      };
      setError(null);
      book.mutate(deal);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };
  const sides: Side[] = fx ? ['BUY', 'SELL'] : ['LEND', 'BORROW'];
  return (
    <section className="sheet" role="dialog" aria-label="Book a deal" data-testid="ticket">
      <form onSubmit={submit} noValidate>
        <div className="head">
          <span className="t-heading">Book a deal</span>
          <span className="grow" />
          <button type="button" className="btn btn-ghost btn-sm" onClick={onClose} aria-label="close" title="close (Esc)">
            ✕
          </button>
        </div>
        <div className="form-grid">
          <span className="k">Desk</span>
          <select className="input input-sm" value={f.desk_id} onChange={(e) => setF({ ...f, desk_id: Number(e.target.value) })} aria-label="Desk for the deal">
            {desks.map((d) => (
              <option key={d.desk_id} value={d.desk_id}>
                {d.code}
              </option>
            ))}
          </select>
          <span className="k">Product</span>
          <select
            className="input input-sm"
            value={f.product}
            aria-label="Product"
            onChange={(e) => {
              const p = e.target.value as Product;
              setF({ ...f, product: p, side: isFx(p) ? 'BUY' : 'LEND', ccy2: isFx(p) ? f.ccy2 || 'BHD' : '' });
            }}
          >
            {PRODUCTS.map((p) => (
              <option key={p} value={p}>
                {PRODUCT_LABEL[p]}
              </option>
            ))}
          </select>
          <span className="k">Side</span>
          <div className="chips">
            {sides.map((s) => (
              <Chip key={s} small outlined selected={f.side === s} onClick={() => setF({ ...f, side: s })}>
                {s}
              </Chip>
            ))}
          </div>
          <span className="k">{fx ? 'Pair' : 'Currency'}</span>
          <div className="row">
            <input className="input input-sm" style={{ width: 56 }} maxLength={3} value={f.ccy1} onChange={(e) => setF({ ...f, ccy1: e.target.value })} aria-label="ccy1" />
            {fx && (
              <>
                <span>/</span>
                <input className="input input-sm" style={{ width: 56 }} maxLength={3} value={f.ccy2} onChange={(e) => setF({ ...f, ccy2: e.target.value })} aria-label="ccy2" />
              </>
            )}
          </div>
          <span className="k">{fx ? 'Notional (ccy1)' : 'Principal'}</span>
          <input className="input input-sm" value={f.notional} onChange={(e) => setF({ ...f, notional: e.target.value })} aria-label="Notional" />
          <span className="k">{fx ? 'Rate' : 'Rate (%)'}</span>
          <input className="input input-sm" value={f.rate} onChange={(e) => setF({ ...f, rate: e.target.value })} aria-label="Rate" />
          {check && (
            <>
              <span />
              <span className={`t-small ${check.off ? 't-bad' : ''}`} data-testid="market-check">
                market {check.mid.toFixed(5)} for the value date · {check.bp > 0 ? '+' : ''}
                {check.bp.toFixed(0)} bp{check.off ? `, beyond the ${check.tolerance} bp tolerance` : ''}
              </span>
            </>
          )}
          <span className="k">Trade date</span>
          <input className="input input-sm" type="date" value={f.trade_date} onChange={(e) => setF({ ...f, trade_date: e.target.value })} aria-label="Trade date" />
          <span className="k">Value date</span>
          <input className="input input-sm" type="date" value={f.value_date} onChange={(e) => setF({ ...f, value_date: e.target.value })} aria-label="Value date" />
          {!fx && (
            <>
              <span className="k">Maturity date</span>
              <input className="input input-sm" type="date" value={f.maturity_date} onChange={(e) => setF({ ...f, maturity_date: e.target.value })} aria-label="Maturity date" />
            </>
          )}
          <span className="k">Counterparty</span>
          <input className="input input-sm" value={f.counterparty} onChange={(e) => setF({ ...f, counterparty: e.target.value })} aria-label="Counterparty" autoFocus />
        </div>
        {check?.off && (
          <label className="row notice bad" style={{ marginTop: 8, alignItems: 'flex-start' }}>
            <input type="checkbox" checked={offMarketOk} onChange={(e) => setOffMarketOk(e.target.checked)} />
            <span>Book at this rate although it is {Math.abs(check.bp).toFixed(0)} bp from the market. The deal will show as off-market on the risk page.</span>
          </label>
        )}
        {error && (
          <div className="notice bad" role="alert" style={{ marginTop: 8 }}>
            {error}
          </div>
        )}
        <div className="row" style={{ marginTop: 12 }}>
          <button type="submit" className="btn btn-primary" disabled={book.isPending || (check?.off === true && !offMarketOk)}>
            {book.isPending ? 'Booking…' : 'Book'}
          </button>
          <button type="button" className="btn" onClick={onClose}>
            Discard
          </button>
        </div>
        <div className="t-small" style={{ marginTop: 8 }}>
          Booked deals enter the reconciliation log immediately.
        </div>
      </form>
    </section>
  );
}
