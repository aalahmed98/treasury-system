// Saved scenarios on the rate shock bar: name the simulation on screen, open one again, share it
// with the desk, and put a shared one in the ALCO pack.
import { useCallback, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ApiError, endpoints } from '../api/client';
import type { AlmScenario } from '../api/types';
import { useOutsideClose } from '../components/ui';
import { useToast } from '../components/feedback';
import { useAlm } from './state';
import { bodySummary, sameBody } from './scenario';

export function ScenarioMenu() {
  const { simulating, currentBody, loadScenario, loaded, setLoaded, clearSimulation } = useAlm();
  const qc = useQueryClient();
  const toast = useToast();
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const ref = useOutsideClose<HTMLDivElement>(open, close);
  const [name, setName] = useState('');
  const list = useQuery({ queryKey: ['alm', 'scenarios'], queryFn: endpoints.almScenarios, staleTime: 30_000 });
  const all = list.data ?? [];
  const mine = all.filter((s) => s.mine);
  const others = all.filter((s) => !s.mine);
  const current = loaded ? all.find((s) => s.id === loaded.id) : undefined;
  const body = currentBody();
  const edited = current ? !sameBody(current.body, body) : false;

  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ['alm', 'scenarios'] });
    void qc.invalidateQueries({ queryKey: ['alm', 'compare'] });
  };
  const failed = (title: string) => (e: unknown) => toast({ tone: 'bad', title, detail: e instanceof ApiError ? e.message : undefined });
  const save = useMutation({
    mutationFn: (v: { name: string; id?: string }) => endpoints.almScenarioSave(v.name, body, v.id),
    onSuccess: (s) => {
      setLoaded({ id: s.id, name: s.name });
      setName('');
      refresh();
      toast({ tone: 'good', title: `Saved “${s.name}”`, detail: s.shared ? 'The desk sees the new version.' : 'Only you see it until you share it.' });
    },
    onError: failed('The scenario was not saved'),
  });
  const share = useMutation({ mutationFn: (v: { id: string; on: boolean }) => endpoints.almScenarioShare(v.id, v.on), onSuccess: refresh, onError: failed('Sharing did not change') });
  const pack = useMutation({ mutationFn: (v: { id: string; on: boolean }) => endpoints.almScenarioPack(v.id, v.on), onSuccess: refresh, onError: failed('The ALCO pack did not change') });
  const remove = useMutation({
    mutationFn: (s: AlmScenario) => endpoints.almScenarioDelete(s.id),
    onSuccess: (_, s) => {
      if (loaded?.id === s.id) setLoaded(null);
      refresh();
    },
    onError: failed('The scenario was not deleted'),
  });

  const row = (s: AlmScenario) => (
    <div key={s.id} className={`scenario-row${loaded?.id === s.id ? ' on' : ''}`}>
      <button
        type="button"
        className="open"
        onClick={() => {
          loadScenario(s);
          close();
        }}
        title="open this scenario: its shock, assumptions and hedges replace the ones on screen"
      >
        <span className="t-medium">{s.name}</span>
        <span className="t-small">
          {s.mine ? '' : `${s.owner_name} · `}
          {bodySummary(s.body)}
        </span>
      </button>
      <span className="flags">
        {s.mine && (
          <label className="row" style={{ gap: 4 }} title="let the ALM desk and the treasury head see and open it">
            <input type="checkbox" checked={s.shared} onChange={(e) => share.mutate({ id: s.id, on: e.target.checked })} aria-label={`share ${s.name}`} />
            <span className="t-small">shared</span>
          </label>
        )}
        {s.shared && (
          <label className="row" style={{ gap: 4 }} title="list it under Interest rate risk in the ALCO pack, marked as a simulation">
            <input type="checkbox" checked={s.in_pack} onChange={(e) => pack.mutate({ id: s.id, on: e.target.checked })} aria-label={`${s.name} in the ALCO pack`} />
            <span className="t-small">in pack</span>
          </label>
        )}
        {s.mine && (
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => remove.mutate(s)} aria-label={`delete ${s.name}`} title="delete this scenario">
            ✕
          </button>
        )}
      </span>
    </div>
  );

  return (
    <div className="anchor" ref={ref}>
      <button type="button" className="btn btn-sm" onClick={() => setOpen((o) => !o)} aria-expanded={open} aria-haspopup="dialog" data-testid="scenarios-button" title="saved scenarios: keep this simulation, or open one">
        {loaded ? `Scenario: ${loaded.name}${edited ? ' (edited)' : ''}` : 'Scenarios'} ▾
      </button>
      {open && (
        <div className="popup scenario-panel" role="dialog" aria-label="Saved scenarios" style={{ left: 0, right: 'auto' }}>
          <div className="t-eyebrow" style={{ marginBottom: 4 }}>
            Keep this simulation
          </div>
          <div className="t-small" style={{ marginBottom: 6 }} data-testid="scenario-current">
            {simulating ? bodySummary(body) : 'Nothing is being simulated: there is no shock, hedge or changed assumption to keep.'}
          </div>
          <form
            className="row"
            style={{ gap: 6 }}
            onSubmit={(e) => {
              e.preventDefault();
              if (name.trim()) save.mutate({ name: name.trim() });
            }}
          >
            <input className="input input-sm" style={{ flex: 1 }} value={name} onChange={(e) => setName(e.target.value)} placeholder="Name, e.g. Rates up, hedged" maxLength={80} aria-label="Scenario name" />
            <button type="submit" className="btn btn-sm btn-primary" disabled={!name.trim() || !simulating || save.isPending}>
              Save as new
            </button>
          </form>
          {current?.mine && edited && (
            <button type="button" className="btn btn-sm" style={{ marginTop: 6 }} onClick={() => save.mutate({ name: current.name, id: current.id })} disabled={save.isPending}>
              Update “{current.name}”
            </button>
          )}
          <div className="t-eyebrow" style={{ margin: '12px 0 4px' }}>
            My scenarios
          </div>
          <div data-testid="scenarios-mine">{mine.length ? mine.map(row) : <div className="t-muted t-small">None yet. They are yours alone until you share them.</div>}</div>
          <div className="t-eyebrow" style={{ margin: '12px 0 4px' }}>
            Shared by the desk
          </div>
          <div data-testid="scenarios-shared">{others.length ? others.map(row) : <div className="t-muted t-small">Nobody else has shared one.</div>}</div>
          {list.error && <div className="t-bad t-small">{list.error.message}</div>}
          <div className="row" style={{ marginTop: 10, gap: 8 }}>
            <button type="button" className="btn btn-ghost btn-sm" onClick={clearSimulation} disabled={!simulating && !loaded} title="no shock, no hedges, the bank's own assumptions">
              Clear the simulation
            </button>
            <span className="t-small">Compare them on the Stress tab.</span>
          </div>
        </div>
      )}
    </div>
  );
}
