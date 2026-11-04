// Breaks: what the reconciler could not settle on its own, and the actions a user may take.
import { useEffect, useMemo, useState } from 'react';
import { useSearchParams } from 'react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ApiError, endpoints } from '../api/client';
import { useBreaks, useDeals } from '../api/queries';
import { useSession } from '../api/session';
import { useLive } from '../api/live';
import type { BreakKind, BreakView, ResolveRequest } from '../api/types';
import { BREAK_EXPLANATION, breakKindClass, breakRow, type BreakRow } from '../lib/deals';
import { Chip } from '../components/ui';
import { DataTable, byNumber, byString, exportCsv, useSort, type Col } from '../components/table';
import { WithPositions } from '../components/positions';

const KINDS: BreakKind[] = ['MISSING', 'MISMATCH', 'ORPHAN', 'AMBIGUOUS', 'DUPLICATE'];

const COLS: Col<BreakRow>[] = [
  { id: 'id', header: '#', width: 64, sort: byNumber((r) => r.view.id), cell: (r) => <span className="t-muted">{r.view.id}</span>, csv: (r) => String(r.view.id) },
  { id: 'kind', header: 'Kind', width: 96, sort: byString((r) => r.kind), cell: (r) => <span className={`t-medium ${breakKindClass(r.view.kind)}`}>{r.kind}</span>, csv: (r) => r.kind },
  { id: 'deal', header: 'Deal', width: 96, sort: byString((r) => r.dealRef), cell: (r) => r.dealRef },
  { id: 'source', header: 'Source', width: 124, cell: (r) => r.source },
  { id: 'field', header: 'Field', width: 90, cell: (r) => r.field },
  { id: 'expected', header: 'Expected', width: 124, cell: (r) => r.expected },
  { id: 'observed', header: 'Observed', width: 124, cell: (r) => r.observed },
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
  const [selected, setSelected] = useState<number | null>(params.get('break') ? Number(params.get('break')) : null);
  const [sort, setSort] = useSort({ id: 'detected', desc: true });

  useEffect(() => {
    const id = params.get('break');
    if (id) {
      setSelected(Number(id));
      setParams({}, { replace: true });
    }
  }, [params, setParams]);

  const rows = useMemo(() => (breaks.data ?? []).map(breakRow), [breaks.data]);
  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    return rows.filter((r) => !q || r.search.includes(q));
  }, [rows, search]);
  const counts = KINDS.map((k) => [k, rows.filter((r) => r.view.kind === k && r.view.status !== 'RESOLVED').length] as const).filter(([, n]) => n > 0);
  const current = selected !== null ? rows.find((r) => r.view.id === selected)?.view : undefined;

  return (
    <WithPositions ready={breaks.data !== undefined}>
      <div className="toolbar">
        <h1 className="t-title" style={{ marginRight: 4 }}>
          Breaks
        </h1>
        <input className="input input-sm" style={{ width: 260 }} placeholder="search kind, deal, source, field…" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Search breaks" />
        <Chip outlined selected={showResolved} onClick={() => setShowResolved((v) => !v)}>
          show resolved
        </Chip>
        <span className="spacer" />
        <button type="button" className="btn btn-ghost btn-sm" onClick={() => exportCsv('breaks', COLS, filtered)}>
          Export CSV
        </button>
        {counts.map(([k, n]) => (
          <span key={k} className={`t-medium ${breakKindClass(k)}`}>
            {k} {n}
          </span>
        ))}
      </div>
      <DataTable
        columns={COLS}
        rows={filtered}
        rowKey={(r) => String(r.view.id)}
        selected={selected === null ? null : String(selected)}
        onSelect={(r) => setSelected((s) => (s === r.view.id ? null : r.view.id))}
        rowClass={(r) => (live.flash.breaks.has(r.view.id) ? 'flash' : '')}
        sort={sort}
        onSort={setSort}
        testId="breaks"
        empty={breaks.isLoading ? 'loading…' : 'no breaks'}
      />
      {current && <Actions view={current} key={current.id} />}
    </WithPositions>
  );
}

function Actions({ view }: { view: BreakView }) {
  const { canResolve } = useSession();
  const qc = useQueryClient();
  const deals = useDeals();
  const [note, setNote] = useState('');
  const [linkRef, setLinkRef] = useState('');
  const [error, setError] = useState<string | null>(null);
  const can = canResolve(view.desk_id);
  const resolved = view.status === 'RESOLVED';
  const done = () => {
    void qc.invalidateQueries({ queryKey: ['breaks'] });
    void qc.invalidateQueries({ queryKey: ['deals'] });
    void qc.invalidateQueries({ queryKey: ['attention'] });
  };
  const ack = useMutation({ mutationFn: () => endpoints.ackBreak(view.id), onSuccess: done, onError: (e) => setError(e instanceof ApiError ? e.message : 'failed') });
  const resolve = useMutation({ mutationFn: (req: ResolveRequest) => endpoints.resolveBreak(view.id, req), onSuccess: done, onError: (e) => setError(e instanceof ApiError ? e.message : 'failed') });
  const candidates = Array.isArray(view.details.candidates) ? (view.details.candidates as { deal_id?: string; deal_ref?: string; score?: number }[]).slice(0, 6) : [];
  const typed = linkRef.trim().toUpperCase();
  const found = typed ? deals.data?.deals.find((d) => d.deal_ref === typed) : undefined;
  const link = (dealId: string, r: string) => resolve.mutate({ action: 'LINK', deal_id: dealId, note: note.trim() || `linked to ${r}` });

  return (
    <div className="detail" data-testid="break-actions">
      <div className="head">
        <span className={`t-medium ${breakKindClass(view.kind)}`}>{view.kind}</span>
        <span className="t-muted">#{view.id}</span>
        {view.deal_ref && <span className="t-medium">{view.deal_ref}</span>}
        {view.source && (
          <span>
            {view.source} {view.source_ref ?? ''}
          </span>
        )}
        <span className="t-small">{BREAK_EXPLANATION[view.kind]}</span>
      </div>
      {!can && <div className="t-muted">you can see this break but not act on it (no resolve right on this desk)</div>}
      {resolved ? (
        <div className="t-muted">
          resolved: {view.resolution?.toLowerCase() ?? ''}
          {view.note ? ` · ${view.note}` : ''}
        </div>
      ) : (
        <div className="stack">
          <div className="row">
            <button type="button" className="btn btn-sm" disabled={!can || view.status !== 'OPEN' || ack.isPending} onClick={() => ack.mutate()}>
              Acknowledge
            </button>
            <span style={{ width: 8 }} />
            <input className="input input-sm" style={{ width: 280 }} placeholder="note (required to dismiss)" value={note} onChange={(e) => setNote(e.target.value)} aria-label="Resolution note" />
            <button type="button" className="btn btn-sm" disabled={!can || !note.trim() || resolve.isPending} title="accept as-is; recorded with your note" onClick={() => resolve.mutate({ action: 'DISMISS', deal_id: null, note: note.trim() })}>
              Dismiss
            </button>
          </div>
          {(view.kind === 'ORPHAN' || view.kind === 'AMBIGUOUS' || view.kind === 'DUPLICATE') && (
            <div className="row-wrap" style={{ alignItems: 'center' }}>
              <span className="t-secondary">Link to:</span>
              {candidates.map((c) =>
                c.deal_id && c.deal_ref ? (
                  <button key={c.deal_id} type="button" className="btn btn-sm" disabled={!can || resolve.isPending} onClick={() => link(c.deal_id!, c.deal_ref!)}>
                    {c.deal_ref} ({c.score ?? 0})
                  </button>
                ) : null,
              )}
              <input className="input input-sm" style={{ width: 150 }} placeholder="deal ref, e.g. FX-000123" value={linkRef} onChange={(e) => setLinkRef(e.target.value)} aria-label="Deal reference to link" />
              <button type="button" className="btn btn-sm" disabled={!can || !found || resolve.isPending} onClick={() => found && link(found.id, found.deal_ref)}>
                Link
              </button>
              {typed && !found && <span className="t-warn">no such deal in your blotter</span>}
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
  );
}
