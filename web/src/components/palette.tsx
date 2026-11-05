// The command palette (Ctrl+K or /): pages, ALM tools, actions, any deal or break the user can
// see, and the keyboard shortcuts. Also the global keys: `g` then a letter jumps to a page.
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useNavigate } from 'react-router';
import { useQueryClient } from '@tanstack/react-query';
import { useSession } from '../api/session';
import type { BreakView, DealsResponse, LadderResponse } from '../api/types';
import { PRODUCT_LABEL } from '../api/types';
import { fmtAmount } from '../lib/fmt';
import { Kbd } from './feedback';
import { Dot, type Tone } from './ui';
import { pageFor, warm } from '../pages/routes';

type Mode = 'commands' | 'shortcuts';

interface PaletteApi {
  open: (mode?: Mode) => void;
  close: () => void;
}

const Ctx = createContext<PaletteApi>({ open: () => {}, close: () => {} });

export const usePalette = () => useContext(Ctx);

interface Item {
  id: string;
  group: string;
  label: string;
  detail?: string;
  hint?: string;
  tone?: Tone | 'violet' | 'magenta';
  keywords: string;
  run: () => void;
}

export const SHORTCUTS: [string[], string][] = [
  [['Ctrl', 'K'], 'Open the command palette'],
  [['/'], 'Search the list on this page, or open the palette'],
  [['?'], 'This list'],
  [['g', 'o'], 'Overview'],
  [['g', 'b'], 'Blotter'],
  [['g', 'k'], 'Breaks'],
  [['g', 'l'], 'Liquidity ladder'],
  [['g', 'r'], 'FX risk'],
  [['g', 'a'], 'ALM workbench'],
  [['g', 'p'], 'Bond portfolio (PMU)'],
  [['↑', '↓'], 'Move the selection in a table'],
  [['Enter'], 'Open the selected row'],
  [['Esc'], 'Close the detail, the ticket or a popup'],
  [['n'], 'Book a deal (blotter)'],
  [['a'], 'Acknowledge the selected break'],
];

const isTyping = (t: EventTarget | null) => {
  const el = t as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable;
};

export function PaletteProvider({ children }: { children: ReactNode }) {
  const [mode, setMode] = useState<Mode | null>(null);
  const navigate = useNavigate();
  const { canSeeAlm, canSeeRisk, canSeePmu } = useSession();
  const open = useCallback((m: Mode = 'commands') => setMode(m), []);
  const close = useCallback(() => setMode(null), []);
  const pending = useRef<number | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        setMode((m) => (m ? null : 'commands'));
        return;
      }
      if (e.ctrlKey || e.metaKey || e.altKey || isTyping(e.target)) return;
      if (pending.current !== null) {
        window.clearTimeout(pending.current);
        pending.current = null;
        const to = e.key === 'o' ? '/' : e.key === 'b' ? '/blotter' : e.key === 'k' ? '/breaks' : e.key === 'l' ? '/liquidity' : e.key === 'r' && canSeeRisk ? '/risk' : e.key === 'a' && canSeeAlm ? '/alm' : e.key === 'p' && canSeePmu ? '/pmu' : null;
        if (to) {
          e.preventDefault();
          navigate(to);
          return;
        }
      }
      if (e.key === 'g') {
        pending.current = window.setTimeout(() => (pending.current = null), 900);
      } else if (e.key === '/') {
        e.preventDefault();
        // On a page with a search box, "/" goes there; elsewhere it opens the palette.
        const search = document.querySelector<HTMLInputElement>('input[data-search]');
        if (search) {
          search.focus();
          search.select();
        } else setMode('commands');
      } else if (e.key === '?') {
        e.preventDefault();
        setMode('shortcuts');
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [navigate, canSeeAlm, canSeeRisk, canSeePmu]);

  const api = useMemo(() => ({ open, close }), [open, close]);
  return (
    <Ctx.Provider value={api}>
      {children}
      {mode === 'commands' && <Palette onClose={close} />}
      {mode === 'shortcuts' && <Shortcuts onClose={close} />}
    </Ctx.Provider>
  );
}

function score(item: Item, q: string): number {
  if (!q) return 1;
  const label = item.label.toLowerCase();
  if (label.startsWith(q)) return 4;
  if (label.includes(q)) return 3;
  const words = q.split(/\s+/).filter(Boolean);
  return words.every((w) => item.keywords.includes(w)) ? 2 : 0;
}

function useItems(q: string): Item[] {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const { canSeeAlm, canSeeRisk, canSeePmu, bookableDesks, logout } = useSession();
  const withRecords = q.length >= 2;
  return useMemo(() => {
    const go = (to: string) => () => navigate(to);
    const items: Item[] = [
      { id: 'p-overview', group: 'Pages', label: 'Overview', hint: 'g o', keywords: 'overview home dashboard', run: go('/') },
      { id: 'p-blotter', group: 'Pages', label: 'Deal blotter', hint: 'g b', keywords: 'blotter deals trades', run: go('/blotter') },
      { id: 'p-breaks', group: 'Pages', label: 'Breaks', hint: 'g k', keywords: 'breaks reconciliation exceptions', run: go('/breaks') },
      { id: 'p-ladder', group: 'Pages', label: 'Liquidity ladder', hint: 'g l', keywords: 'liquidity ladder cash nostro', run: go('/liquidity') },
      { id: 'p-ftp', group: 'Pages', label: 'FTP curves', keywords: 'ftp funds transfer pricing curves funding rate', run: go('/ftp') },
    ];
    if (canSeeRisk) {
      items.push(
        { id: 'p-risk', group: 'Pages', label: 'FX risk', hint: 'g r', keywords: 'risk fx position nop pnl p&l var limits counterparty', run: go('/risk') },
        { id: 'p-limits', group: 'Pages', label: 'FX limits', keywords: 'limits stop-loss lines counterparty var nop', run: go('/risk/limits') },
      );
    }
    if (canSeePmu) {
      const pages: [string, string, string, string?][] = [
        ['', 'Bond portfolio overview', 'pmu bond portfolio overview carry dv01 yield', 'g p'],
        ['positions', 'Bond positions', 'pmu positions register lots isin kondor tree holdings'],
        ['income', 'Bond income and forecast', 'pmu income accrual coupon amortisation ftp carry nii forecast'],
        ['risk', 'Bond risk and hedges', 'pmu risk hedges dv01 swaps irs shock duration unhedged'],
        ['funding', 'Bond funding and repo', 'pmu repo funding collateral quotes sofr receipts'],
      ];
      for (const [pg, label, kw, hint] of pages) items.push({ id: `pmu-${pg || 'overview'}`, group: 'PMU', label, keywords: kw, hint, run: go(pg ? `/pmu/${pg}` : '/pmu') });
    }
    if (canSeeAlm) {
      const tools: [string, string, string][] = [
        ['', 'ALM dashboard', 'alm dashboard balance sheet'],
        ['lcr', 'LCR', 'liquidity coverage ratio lcr'],
        ['nsfr', 'NSFR', 'net stable funding ratio nsfr'],
        ['funding', 'Funding concentration', 'funding depositors wholesale'],
        ['gap', 'Repricing gap', 'gap repricing buckets'],
        ['nii', 'NII and NIM', 'nii nim income margin'],
        ['stress', 'Stress and EVE', 'stress eve scenarios survival'],
        ['ird', 'Interest rate differentials', 'ird differential forward points parity'],
        ['attribution', 'Variance attribution', 'attribution variance waterfall'],
        ['ladder', 'Liquidity maturity ladder', 'liquidity gap maturity ladder mismatch behavioural contractual'],
        ['ewi', 'Early warning and contingency funding', 'early warning ewi contingency funding plan cfp'],
        ['pack', 'ALCO pack', 'alco pack committee paper sign-off commentary'],
        ['ftp', 'Publish FTP curves', 'ftp funds transfer pricing publish workbook excel upload curves'],
        ['setup', 'ALM setup', 'setup parameters versions limits'],
      ];
      for (const [t, label, kw] of tools) items.push({ id: `t-${t || 'dash'}`, group: 'ALM', label, keywords: kw, hint: t === '' ? 'g a' : undefined, run: go(t ? `/alm/${t}` : '/alm') });
    }
    if (bookableDesks.length) items.push({ id: 'a-book', group: 'Actions', label: 'Book a deal', hint: 'n', keywords: 'book new deal ticket', run: go('/blotter?book=1') });
    items.push(
      { id: 'a-refresh', group: 'Actions', label: 'Refresh everything', keywords: 'refresh reload data', run: () => void qc.invalidateQueries() },
      { id: 'a-print', group: 'Actions', label: 'Print this page', keywords: 'print pdf', run: () => window.print() },
      { id: 'a-theme-light', group: 'Actions', label: 'Theme: light', keywords: 'theme light', run: () => setTheme('light') },
      { id: 'a-theme-dark', group: 'Actions', label: 'Theme: dark', keywords: 'theme dark', run: () => setTheme('dark') },
      { id: 'a-theme-auto', group: 'Actions', label: 'Theme: follow the system', keywords: 'theme auto system', run: () => setTheme('auto') },
      { id: 'a-logout', group: 'Actions', label: 'Log out', keywords: 'log out sign out', run: () => void logout() },
    );
    const ladder = qc.getQueryData<LadderResponse>(['ladder', '']);
    for (const c of ladder?.currencies ?? []) items.push({ id: `c-${c}`, group: 'Ladder', label: `${c} ladder`, keywords: `${c.toLowerCase()} ladder currency`, run: go(`/liquidity?ccy=${c}`) });
    if (withRecords) {
      const deals = qc.getQueryData<DealsResponse>(['deals'])?.deals ?? [];
      for (const d of deals) {
        const pair = d.ccy2 ? `${d.ccy1}/${d.ccy2}` : d.ccy1;
        items.push({
          id: `d-${d.id}`,
          group: 'Deals',
          label: d.deal_ref,
          detail: `${PRODUCT_LABEL[d.product]} ${d.side} ${pair} ${fmtAmount(d.notional)} · ${d.counterparty}`,
          tone: d.match_status === 'BREAK' ? 'bad' : d.match_status === 'MATCHED' ? 'good' : 'neutral',
          keywords: `${d.deal_ref} ${d.counterparty} ${pair} ${d.desk_code} ${d.status} ${d.match_status}`.toLowerCase(),
          run: go(`/blotter?deal=${d.id}`),
        });
      }
      const breaks = qc.getQueryData<BreakView[]>(['breaks', false]) ?? [];
      for (const b of breaks) {
        items.push({
          id: `b-${b.id}`,
          group: 'Breaks',
          label: `#${b.id} ${b.kind}`,
          detail: `${b.deal_ref ?? ''} ${b.source ?? ''} ${b.source_ref ?? ''} · ${b.desk_code}`.trim(),
          tone: b.kind === 'MISMATCH' ? 'bad' : b.kind === 'MISSING' ? 'warn' : b.kind === 'ORPHAN' ? 'violet' : b.kind === 'DUPLICATE' ? 'magenta' : 'info',
          keywords: `#${b.id} ${b.id} ${b.kind} ${b.deal_ref ?? ''} ${b.source_ref ?? ''} ${b.field ?? ''}`.toLowerCase(),
          run: go(`/breaks?break=${b.id}`),
        });
      }
    }
    return items;
  }, [navigate, qc, canSeeAlm, canSeeRisk, canSeePmu, bookableDesks.length, logout, withRecords]);
}

function setTheme(p: 'auto' | 'light' | 'dark') {
  if (p === 'auto') delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = p;
  try {
    if (p === 'auto') localStorage.removeItem('nbb-theme');
    else localStorage.setItem('nbb-theme', p);
  } catch {
    // ignore
  }
  window.dispatchEvent(new Event('nbb-theme'));
}

function Palette({ onClose }: { onClose: () => void }) {
  const [q, setQ] = useState('');
  const [cursor, setCursor] = useState(0);
  const items = useItems(q.trim().toLowerCase());
  const listRef = useRef<HTMLDivElement>(null);
  const needle = q.trim().toLowerCase();
  const shown = useMemo(() => {
    const scored = items.map((i) => [score(i, needle), i] as const).filter(([s]) => s > 0);
    scored.sort((a, b) => b[0] - a[0]);
    return scored.slice(0, 14).map(([, i]) => i);
  }, [items, needle]);
  useEffect(() => setCursor(0), [needle]);
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
  }, [cursor]);
  const run = (i: Item) => {
    onClose();
    i.run();
  };
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setCursor((c) => Math.min(shown.length - 1, c + 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setCursor((c) => Math.max(0, c - 1));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (shown[cursor]) run(shown[cursor]);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    }
  };
  let lastGroup = '';
  return (
    <div className="palette-backdrop" onMouseDown={onClose}>
      <div className="palette" role="dialog" aria-label="Command palette" onMouseDown={(e) => e.stopPropagation()} onKeyDown={onKey}>
        <input className="palette-input" placeholder="Jump to a page, a deal reference, a counterparty, a break…" value={q} onChange={(e) => setQ(e.target.value)} autoFocus aria-label="Command" role="combobox" aria-expanded aria-controls="palette-list" aria-activedescendant={shown[cursor]?.id} />
        <div className="palette-list" role="listbox" id="palette-list" ref={listRef}>
          {shown.length === 0 && <div className="t-muted" style={{ padding: '10px 12px' }}>Nothing matches. Deals and breaks match by reference, counterparty or pair once the lists have loaded.</div>}
          {shown.map((i, idx) => {
            const head = i.group !== lastGroup ? <div className="t-eyebrow palette-group">{i.group}</div> : null;
            lastGroup = i.group;
            return (
              <div key={i.id}>
                {head}
                <div id={i.id} role="option" aria-selected={idx === cursor} className={`palette-row${idx === cursor ? ' active' : ''}`} onMouseEnter={() => setCursor(idx)} onClick={() => run(i)}>
                  {i.tone && <Dot tone={i.tone} />}
                  <span className="t-medium">{i.label}</span>
                  {i.detail && <span className="t-small palette-detail">{i.detail}</span>}
                  <span className="grow" />
                  {i.hint && (
                    <span className="palette-hint">
                      {i.hint.split(' ').map((k) => (
                        <Kbd key={k}>{k}</Kbd>
                      ))}
                    </span>
                  )}
                </div>
              </div>
            );
          })}
        </div>
        <div className="palette-foot t-small">
          <Kbd>↑</Kbd>
          <Kbd>↓</Kbd> move · <Kbd>Enter</Kbd> open · <Kbd>Esc</Kbd> close · <Kbd>?</Kbd> shortcuts
        </div>
      </div>
    </div>
  );
}

function Shortcuts({ onClose }: { onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  return (
    <div className="palette-backdrop" onMouseDown={onClose}>
      <div className="palette" role="dialog" aria-label="Keyboard shortcuts" onMouseDown={(e) => e.stopPropagation()} style={{ padding: 14 }}>
        <div className="row" style={{ marginBottom: 8 }}>
          <span className="t-heading">Keyboard shortcuts</span>
          <span className="grow" />
          <button type="button" className="btn btn-ghost btn-sm" onClick={onClose} aria-label="close" autoFocus>
            ✕
          </button>
        </div>
        <div className="shortcuts">
          {SHORTCUTS.map(([keys, what]) => (
            <div key={what + keys.join()} className="shortcut">
              <span>
                {keys.map((k, i) => (
                  <span key={i}>
                    {i > 0 && <span className="t-muted"> </span>}
                    <Kbd>{k}</Kbd>
                  </span>
                ))}
              </span>
              <span className="t-secondary">{what}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

/** Warms a page's chunk when the pointer rests on its tab. */
export const warmFor = (to: string) => () => warm(pageFor(to));
