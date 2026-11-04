// Breaks: what the reconciler could not settle on its own, and the actions a user may take. The
// page works as a queue: resolving or acknowledging one moves on to the next in the same order.
import { useCallback, useDeferredValue, useEffect, useMemo, useState } from 'react';
import { useSearchParams } from 'react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ApiError, endpoints } from '../api/client';
import { useBreaks, useDeals } from '../api/queries';
import { useSession } from '../api/session';
import { useLive } from '../api/live';
import type { BreakKind, BreakView, ResolveRequest } from '../api/types';
import { BREAK_EXPLANATION, breakKindClass, breakRow, type BreakRow } from '../lib/deals';
import { fmtLocal } from '../lib/fmt';
import { Chip } from '../components/ui';
import { DataTable, byNumber, byString, exportCsv, sortRows, useSort, useTableView, type Col } from '../components/table';
import { SideToggle, WithSide } from '../components/positions';
import { Empty, Kbd, useToast } from '../components/feedback';

const KINDS: BreakKind[] = ['MISSING', 'MISMATCH', 'ORPHAN', 'AMBIGUOUS', 'DUPLICATE'];

const COLS: Col<BreakRow>[] = [
  { id: 'id', header: '#', width: 64, fixed: true, sort: byNumber((r) => r.view.id), cell: (r) => <span className="t-muted">{r.view.id}</span>, csv: (r) => String(r.view.id) },
  { id: 'kind', header: 'Kind', width: 96, fixed: true, sort: byString((r) => r.kind), cell: (r) => <span className={`t-medium ${breakKindClass(r.view.kind)}`}>{r.kind.toLowerCase()}</span>, csv: (r) => r.kind },
  { id: 'deal', header: 'Deal', width: 96, sort: byString((r) => r.dealRef), cell: (r) => r.dealRef },
  { id: 'source', header: 'Source', width: 124, cell: (r) => r.source },
  { id: 'field', header: 'Field', width: 90, cell: (r) => r.field },
  { id: 'disagreement', header: 'Expected → observed', width: 220, title: 'the golden record against what the source sent', cell: (r) => (r.expected || r.observed ? <span>{r.expected || '—'} <span className="t-muted">→</span> <span className="t-bad">{r.observed || '—'}</span></span> : ''), csv: (r) => (r.expected || r.observed ? `${r.expected || '-'} -> ${r.observed || '-'}` : '') },
  { id: 'desk', header: 'Desk', width: 52, cell: (r) => r.view.desk_code },
  { id: 'detected', header: 'Detected', width: 130, sort: byString((r) => r.view.detected_at), cell: (r) => <span className="t-muted">{r.detected}</span>, csv: (r) => r.detected },
  { id: 'status', header: 'Status', width: 110, sort: byString((r) => r.status), cell: (r) => <span className={r.view.status === 'OPEN' ? 't-warn' : r.view.status === 'RESOLVED' ? 't-muted' : ''}>{r.status}</span>, csv: (r) => r.status },
];

export function Breaks() {
  const [showResolved, setShowResolved] = useState(false);
  const breaks = useBreaks(showResolved);
  const live = useLive();
  const [params, setParams] = useSearchParams();
  const [search, setSearch] = useState('');
  const needle = useDeferredValue(search.trim().toLowerCase());
  const [kind, setKind] = useState<BreakKind | null>(null);
  const [sort, setSort] = useSort({ id: 'detected', desc: true });
  const { columns, rowHeight, menu } = useTableView('breaks', COLS);
  const selected = params.get('break') ? Number(params.get('break')) : null;

  const select = useCallback(
    (id: number | null) => {
      setParams(
        (p) => {
          const n = new URLSearchParams(p);
          if (id !== null) n.set('break', String(id));
          else n.delete('break');
          return n;
        },
        { replace: true },
      );
    },
    [setParams],
  );

  // A kind filter from the overview, used once.
  useEffect(() => {
    const k = params.get('kind') as BreakKind | null;
    if (k && KINDS.includes(k)) {
      setKind(k);
      setParams(
        (p) => {
          const n = new URLSearchParams(p);
          n.delete('kind');
          return n;
        },
        { replace: true },
      );
    }
  }, [params, setParams]);

  const rows = useMemo(() => (breaks.data ?? []).map(breakRow), [breaks.data]);
  const filtered = useMemo(() => rows.filter((r) => (!needle || r.search.includes(needle)) && (kind === null || r.view.kind === kind)), [rows, needle, kind]);
  const counts = KINDS.map((k) => [k, rows.filter((r) => r.view.kind === k && r.view.status !== 'RESOLVED').length] as const).filter(([, n]) => n > 0);
  const current = selected !== null ? rows.find((r) => r.view.id === selected)?.view : undefined;

  /** After an action, the next open break in the order on screen; the queue keeps moving. */
  const advance = useCallback(
    (from: number) => {
      const ordered = sortRows(filtered, columns, sort);
      const i = ordered.findIndex((r) => r.view.id === from);
      const next = [...ordered.slice(i + 1), ...ordered.slice(0, Math.max(0, i))].find((r) => r.view.status !== 'RESOLVED' && r.view.id !== from);
      select(next ? next.view.id : null);
    },
    [filtered, columns, sort, select],
  );

  const narrowed = needle !== '' || kind !== null;
  return (
    <WithSide page="breaks" ready={breaks.data !== undefined}>
      <div className="toolbar">
        <h1 className="t-title" style={{ marginRight: 4 }}>
          Breaks
        </h1>
        <input className="input input-sm" style={{ width: 240 }} placeholder="search kind, deal, source, field…  /" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Search breaks" data-search />
        {counts.map(([k, n]) => (
          <Chip key={k} outlined selected={kind === k} onClick={() => setKind((c) => (c === k ? null : k))} title={BREAK_EXPLANATION[k]}>
            <span className={kind === k ? '' : breakKindClass(k)}>{k.toLowerCase()}</span>
            <span className="t-small" style={{ marginLeft: 5, color: 'inherit', opacity: 0.75 }}>
              {n}
            </span>
          </Chip>
        ))}
        <Chip outlined selected={showResolved} onClick={() => setShowResolved((v) => !v)}>
          show resolved
        </Chip>
        <span className="spacer" />
        {menu}
        <button type="button" className="btn btn-ghost btn-sm" onClick={() => exportCsv('breaks', columns, filtered)} disabled={filtered.length === 0}>
          Export CSV
        </button>
        <SideToggle />
        <span className="t-small">
          {filtered.length} of {rows.length}
        </span>
      </div>
      <DataTable
        columns={columns}
        rows={filtered}
        rowKey={(r) => String(r.view.id)}
        selected={selected === null ? null : String(selected)}
        onSelect={(r, via) => select(via === 'click' && selected === r.view.id ? null : r.view.id)}
        onEscape={() => select(null)}
        rowClass={(r) => (live.flash.breaks.has(r.view.id) ? 'flash' : '')}
        sort={sort}
        onSort={setSort}
        rowHeight={rowHeight}
        testId="breaks"
        loading={breaks.isLoading}
        empty={
          breaks.isError ? (
            <Empty tone="warn" title="The breaks could not be loaded" detail={breaks.error instanceof Error ? breaks.error.message : 'the server did not answer'}>
              <button type="button" className="btn btn-sm" onClick={() => void breaks.refetch()}>
                Try again
              </button>
            </Empty>
          ) : narrowed ? (
            <Empty title="No breaks match" detail="Widen the search or drop the kind filter.">
              <button
                type="button"
                className="btn btn-sm"
                onClick={() => {
                  setSearch('');
                  setKind(null);
                }}
              >
                Clear filters
              </button>
            </Empty>
          ) : (
            <Empty tone="good" title="No open breaks on your desks" detail="Every deal the reconciler can see agrees with its sources. Resolved ones are behind the chip above." />
          )
        }
      />
      {current ? (
        <Actions view={current} key={current.id} onDone={() => advance(current.id)} />
      ) : (
        <div className="hint t-small">
          <Kbd>↑</Kbd> <Kbd>↓</Kbd> move · <Kbd>Enter</Kbd> or click opens a break · <Kbd>a</Kbd> acknowledges it · after an action the next open one is selected
        </div>
      )}
    </WithSide>
  );
}

function Actions({ view, onDone }: { view: BreakView; onDone: () => void }) {
  const { canResolve } = useSession();
  const qc = useQueryClient();
  const toast = useToast();
  const deals = useDeals();
  const [note, setNote] = useState('');
  const [linkRef, setLinkRef] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const can = canResolve(view.desk_id);
  const resolved = view.status === 'RESOLVED';
  const done = (what: string) => {
    void qc.invalidateQueries({ queryKey: ['breaks'] });
    void qc.invalidateQueries({ queryKey: ['deals'] });
    void qc.invalidateQueries({ queryKey: ['attention'] });
    toast({ tone: 'good', title: `#${view.id} ${view.kind.toLowerCase()} ${what}` });
    onDone();
  };
  const ack = useMutation({ mutationFn: () => endpoints.ackBreak(view.id), onSuccess: () => done('acknowledged'), onError: (e) => setError(e instanceof ApiError ? e.message : 'the acknowledgement failed; the break is unchanged') });
  const canAck = can && view.status === 'OPEN' && !ack.isPending;
  // `a` acknowledges the selected break when the user is not typing.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (e.key !== 'a' || e.ctrlKey || e.metaKey || e.altKey || (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT'))) return;
      if (!canAck) return;
      e.preventDefault();
      ack.mutate();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [canAck, ack]);
  // A dismiss asks once more, then forgets the question after a few seconds.
  useEffect(() => {
    if (!confirming) return;
    const t = window.setTimeout(() => setConfirming(false), 5000);
    return () => window.clearTimeout(t);
  }, [confirming]);
  const resolve = useMutation({
    mutationFn: (req: ResolveRequest) => endpoints.resolveBreak(view.id, req),
    onSuccess: (_, req) => done(req.action === 'LINK' ? 'linked' : 'dismissed'),
    onError: (e) => setError(e instanceof ApiError ? e.message : 'the resolution failed; the break is unchanged'),
  });
  const candidates = Array.isArray(view.details.candidates) ? (view.details.candidates as { deal_id?: string; deal_ref?: string; score?: number }[]).slice(0, 6) : [];
  const typed = linkRef.trim().toUpperCase();
  const found = typed ? deals.data?.deals.find((d) => d.deal_ref === typed) : undefined;
  const link = (dealId: string, r: string) => resolve.mutate({ action: 'LINK', deal_id: dealId, note: note.trim() || `linked to ${r}` });
  const linkable = view.kind === 'ORPHAN' || view.kind === 'AMBIGUOUS' || view.kind === 'DUPLICATE';
  const expected = view.details.expected;
  const observed = view.details.observed;

  return (
    <div className="detail" data-testid="break-actions">
      <div className="head">
        <span className={`t-heading ${breakKindClass(view.kind)}`}>{view.kind}</span>
        <span className="t-muted">#{view.id}</span>
        {view.deal_ref && <span className="t-medium">{view.deal_ref}</span>}
        {view.source && (
          <span>
            {view.source} {view.source_ref ?? ''}
          </span>
        )}
        <span className="t-small">{view.desk_code} · detected {fmtLocal(view.detected_at)}</span>
        <span className="t-small">{view.status.toLowerCase()}</span>
      </div>
      <div className="two-up" style={{ gridTemplateColumns: 'minmax(0, 1fr) minmax(0, 1.3fr)' }}>
        <div className="stack">
          <div className="t-secondary">{BREAK_EXPLANATION[view.kind]}</div>
          {view.field && (
            <div className="facts">
              <span>
                <span className="k">Field</span>
                {view.field}
              </span>
              <span>
                <span className="k">Expected</span>
                {expected === undefined || expected === null ? '—' : String(expected)}
              </span>
              <span>
                <span className="k">Observed</span>
                <span className="t-bad">{observed === undefined || observed === null ? '—' : String(observed)}</span>
              </span>
            </div>
          )}
          {resolved && (
            <div className="t-muted">
              resolved: {view.resolution?.toLowerCase() ?? ''}
              {view.note ? ` · ${view.note}` : ''}
            </div>
          )}
          {!can && !resolved && <div className="t-small">You can see this break but not act on it: no resolve right on the {view.desk_code} desk.</div>}
        </div>
        {!resolved && (
          <div className="stack">
            <div className="row">
              <button type="button" className="btn btn-sm" disabled={!canAck} onClick={() => ack.mutate()} title="mark it seen; it stays open (a)">
                Acknowledge
              </button>
              <input className="input input-sm grow" placeholder="note (required to dismiss)" value={note} onChange={(e) => setNote(e.target.value)} aria-label="Resolution note" />
              {confirming ? (
                <button type="button" className="btn btn-sm t-bad" disabled={resolve.isPending} title="this closes the break for good with your note" onClick={() => resolve.mutate({ action: 'DISMISS', deal_id: null, note: note.trim() })}>
                  Confirm dismiss
                </button>
              ) : (
                <button type="button" className="btn btn-sm" disabled={!can || !note.trim() || resolve.isPending} title="accept as-is; recorded with your note. Cannot be undone." onClick={() => setConfirming(true)}>
                  Dismiss
                </button>
              )}
            </div>
            {linkable && (
              <div className="row-wrap" style={{ alignItems: 'center' }}>
                <span className="t-secondary">Link to</span>
                {candidates.map((c) =>
                  c.deal_id && c.deal_ref ? (
                    <button key={c.deal_id} type="button" className="btn btn-sm" disabled={!can || resolve.isPending} onClick={() => link(c.deal_id!, c.deal_ref!)} title={`candidate score ${c.score ?? 0}`}>
                      {c.deal_ref} <span className="t-small">{c.score ?? 0}</span>
                    </button>
                  ) : null,
                )}
                {candidates.length === 0 && <span className="t-small">no candidates scored</span>}
                <input className="input input-sm" style={{ width: 150 }} placeholder="or a deal ref…" value={linkRef} onChange={(e) => setLinkRef(e.target.value)} aria-label="Deal reference to link" />
                <button type="button" className="btn btn-sm" disabled={!can || !found || resolve.isPending} onClick={() => found && link(found.id, found.deal_ref)}>
                  Link
                </button>
                {typed && !found && <span className="t-warn t-small">no such deal in your blotter</span>}
              </div>
            )}
            {error && (
              <div className="notice bad" role="alert">
                {error}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
