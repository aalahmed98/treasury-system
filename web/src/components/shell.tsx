// The frame around every page: the top bar (wordmark, desk, tabs, the search box that opens the
// command palette, feed health, the attention bell, the theme switch, the user menu), the
// connection banner, and the status bar with how fresh the data is.
import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { NavLink, useNavigate } from 'react-router';
import { useQueryClient } from '@tanstack/react-query';
import { useSession } from '../api/session';
import { useLive } from '../api/live';
import { useAttention, useBreaks, useDeals, useFeeds } from '../api/queries';
import type { AttentionItem, AttentionTarget, FeedsResponse } from '../api/types';
import { SOURCE_SHORT } from '../api/types';
import { fmtAge, fmtLocal, fmtTime, secondsBetween } from '../lib/fmt';
import { Badge, Dot, type Tone, useOutsideClose } from './ui';
import { Kbd, SkeletonStrip, useAgeSeconds } from './feedback';
import { usePalette, warmFor } from './palette';

export function targetPath(t: AttentionTarget): string {
  switch (t.kind) {
    case 'deal':
      return `/blotter?deal=${t.id}`;
    case 'break':
      return `/breaks?break=${t.id}`;
    case 'breaks':
      return '/breaks';
    case 'provisional':
      return '/blotter?filter=provisional';
    case 'overview':
      return '/';
    case 'alm':
      return `/alm/${t.tool}`;
  }
}

export function severityTone(s: AttentionItem['severity']): Tone {
  return s === 'CRITICAL' ? 'bad' : s === 'WARNING' ? 'warn' : 'info';
}

export const feedHealth = (age: number | null): Tone => (age === null ? 'neutral' : age < 180 ? 'good' : age < 300 ? 'warn' : 'bad');
export const reconHealth = (lag: number): Tone => (lag < 30 ? 'good' : lag < 120 ? 'warn' : 'bad');

/** The reconciler's published clock when it is there, else the server's, like the native client. */
export function referenceNow(f: FeedsResponse | undefined): string {
  return f?.recon?.now ?? f?.server_time ?? new Date().toISOString();
}

export function Shell({ children }: { children: ReactNode }) {
  return (
    <div className="app">
      <TopBar />
      <ConnectionBanner />
      <main id="main">{children}</main>
      <StatusBar />
    </div>
  );
}

/** What a page looks like for the moment its chunk or its first data is on the way. */
export function PageSkeleton() {
  return (
    <div className="page" aria-busy>
      <div className="page-title">
        <span className="skeleton" style={{ width: 160, height: 20 }} />
      </div>
      <SkeletonStrip cells={6} />
    </div>
  );
}

function TopBar() {
  const { homeDesk, canSeeAlm } = useSession();
  const deals = useDeals();
  const breaks = useBreaks(false);
  const open = (breaks.data ?? []).filter((b) => b.status !== 'RESOLVED').length;
  return (
    <header className="topbar">
      <a className="skip" href="#main">
        Skip to content
      </a>
      <div className="wordmark">
        <span className="brand">NBB</span>
        <span>Treasury</span>
      </div>
      {homeDesk && <Badge title="home desk">{homeDesk.code}</Badge>}
      <nav className="tabs" aria-label="Pages">
        <Tab to="/" end label="Overview" />
        <Tab to="/blotter" label="Blotter" count={deals.data ? String(deals.data.deals.length) : undefined} />
        <Tab to="/breaks" label="Breaks" count={open > 0 ? String(open) : undefined} tone={open > 0 ? 'bad' : 'neutral'} />
        <Tab to="/liquidity" label="Liquidity" />
        {canSeeAlm && <Tab to="/alm" label="ALM" />}
      </nav>
      <div className="cluster">
        <SearchButton />
        <Feeds />
        <Bell />
        <ThemeSwitch />
        <UserMenu />
      </div>
    </header>
  );
}

function Tab({ to, label, count, tone = 'neutral', end }: { to: string; label: string; count?: string; tone?: Tone; end?: boolean }) {
  return (
    <NavLink to={to} end={end} className={({ isActive }) => `tab${isActive ? ' active' : ''}`} onPointerEnter={warmFor(to)} onFocus={warmFor(to)}>
      {label}
      {count !== undefined && <Badge tone={tone}>{count}</Badge>}
    </NavLink>
  );
}

function SearchButton() {
  const { open } = usePalette();
  return (
    <button type="button" className="search-btn" onClick={() => open()} title="Jump to a page, a deal or a break (Ctrl K)">
      <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" aria-hidden>
        <circle cx="7" cy="7" r="4.5" />
        <path d="M10.5 10.5 14 14" />
      </svg>
      <span>Jump to…</span>
      <Kbd>Ctrl K</Kbd>
    </button>
  );
}

function Feeds() {
  const { data: f } = useFeeds();
  if (!f) return <span className="feeds t-small">feeds —</span>;
  const reference = referenceNow(f);
  const items: ReactNode[] = [];
  for (const feed of f.feeds) {
    if (feed.source === 'INTERNAL' || feed.source === 'USER') continue;
    const age = feed.last_heartbeat_at ? secondsBetween(reference, feed.last_heartbeat_at) : null;
    items.push(
      <span key={feed.source} className="feed" title={`${feed.source}: ${feed.events_total} events, ${feed.duplicates_dropped} duplicates dropped, last event ${feed.last_event_at ? fmtLocal(feed.last_event_at) : 'never'}`}>
        <Dot tone={feedHealth(age)} />
        {SOURCE_SHORT[feed.source]}
        <span className="age">{age === null ? '—' : fmtAge(age)}</span>
      </span>,
    );
  }
  if (f.recon) {
    const lag = secondsBetween(f.server_time, f.recon.last_tick_at);
    const pending = f.recon.pending > 0 ? `pending ${f.recon.pending} · ` : '';
    items.push(
      <span key="recon" className="feed" title={`reconciler clock ${fmtLocal(f.recon.now)}, last tick ${fmtLocal(f.recon.last_tick_at)}`}>
        <Dot tone={reconHealth(lag)} />
        recon
        <span className="age">
          {pending}
          {fmtAge(lag)}
        </span>
      </span>,
    );
  } else {
    items.push(
      <span key="recon" className="feed" title="the reconciliation worker has never reported">
        <Dot tone="bad" />
        recon
        <span className="age">down</span>
      </span>,
    );
  }
  return <div className="feeds">{items}</div>;
}

function BellIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M4 11V7a4 4 0 0 1 8 0v4l1.2 1.5H2.8L4 11Z" />
      <path d="M6.5 13.5a1.5 1.5 0 0 0 3 0" />
    </svg>
  );
}

function Bell() {
  const { data } = useAttention();
  const items = data ?? [];
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const ref = useOutsideClose<HTMLDivElement>(open, close);
  const navigate = useNavigate();
  const tone: Tone = items.length ? severityTone(items[0].severity) : 'neutral';
  const reference = new Date().toISOString();
  return (
    <div className="anchor" ref={ref}>
      <button type="button" className={`bell ${items.length ? tone : ''}`} onClick={() => setOpen((o) => !o)} aria-expanded={open} aria-haspopup="dialog" title="what needs attention on your desks">
        <BellIcon />
        {items.length > 0 && <span>{items.length}</span>}
      </button>
      {open && (
        <div className="popup popup-wide" role="dialog" aria-label="Needs attention">
          <div className="row">
            <span className="t-heading">Needs attention</span>
            {items.length > 0 && <Badge tone={tone}>{items.length}</Badge>}
          </div>
          {items.length === 0 ? (
            <p className="t-muted" style={{ marginTop: 6 }}>
              Nothing outstanding for your desks.
            </p>
          ) : (
            <div className="popup-list">
              {items.map((it, i) => (
                <div
                  key={i}
                  className="popup-row"
                  role="button"
                  tabIndex={0}
                  onClick={() => {
                    setOpen(false);
                    navigate(targetPath(it.target));
                  }}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      setOpen(false);
                      navigate(targetPath(it.target));
                    }
                  }}
                >
                  <Dot tone={severityTone(it.severity)} />
                  <div>
                    <div className="t-medium">{it.title}</div>
                    <div className="t-small">{it.detail}</div>
                  </div>
                  <span className="t-small">{it.since ? fmtAge(secondsBetween(reference, it.since)) : '—'}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

type ThemePref = 'auto' | 'light' | 'dark';

function readTheme(): ThemePref {
  const t = document.documentElement.dataset.theme;
  return t === 'light' || t === 'dark' ? t : 'auto';
}

function ThemeSwitch() {
  const [pref, setPref] = useState<ThemePref>(readTheme);
  useEffect(() => {
    // The palette can change the theme too; follow it.
    const sync = () => setPref(readTheme());
    window.addEventListener('nbb-theme', sync);
    return () => window.removeEventListener('nbb-theme', sync);
  }, []);
  const choose = (p: ThemePref) => {
    if (p === 'auto') delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = p;
    try {
      if (p === 'auto') localStorage.removeItem('nbb-theme');
      else localStorage.setItem('nbb-theme', p);
    } catch {
      // ignore
    }
    setPref(p);
  };
  return (
    <div className="segmented" role="radiogroup" aria-label="Theme">
      {(['auto', 'light', 'dark'] as ThemePref[]).map((p) => (
        <button key={p} type="button" role="radio" aria-checked={pref === p} className={`chip chip-sm${pref === p ? ' selected' : ''}`} onClick={() => choose(p)}>
          {p === 'auto' ? 'Auto' : p === 'light' ? 'Light' : 'Dark'}
        </button>
      ))}
    </div>
  );
}

function UserMenu() {
  const { current, desks, logout } = useSession();
  const { open: openPalette } = usePalette();
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const ref = useOutsideClose<HTMLDivElement>(open, close);
  if (!current) return null;
  return (
    <div className="anchor" ref={ref}>
      <button type="button" className="btn btn-ghost" onClick={() => setOpen((o) => !o)} aria-expanded={open} aria-haspopup="menu">
        {current.user.display_name} <span aria-hidden>▾</span>
      </button>
      {open && (
        <div className="popup" role="menu">
          <div className="t-medium">{current.user.display_name}</div>
          <div className="t-small">{current.user.username}</div>
          <hr className="hr" />
          <div className="t-eyebrow">Desks</div>
          {desks.map((d) => (
            <div key={d.desk_id} className="row" style={{ padding: '3px 0' }}>
              <span className="t-medium">{d.code}</span>
              <span className="t-small">{d.can_book ? 'book and resolve' : d.can_resolve ? 'read, resolve breaks' : 'read only'}</span>
            </div>
          ))}
          <hr className="hr" />
          <button
            type="button"
            className="menu-item"
            role="menuitem"
            onClick={() => {
              setOpen(false);
              openPalette('shortcuts');
            }}
          >
            Keyboard shortcuts <span className="t-small">?</span>
          </button>
          <button type="button" className="menu-item" role="menuitem" onClick={() => void logout()}>
            Log out
          </button>
        </div>
      )}
    </div>
  );
}

/** Shown only while the live stream is down for more than a moment. */
function ConnectionBanner() {
  const live = useLive();
  const [stale, setStale] = useState(false);
  useEffect(() => {
    if (live.state === 'connected') {
      setStale(false);
      return;
    }
    const t = window.setTimeout(() => setStale(true), 4000);
    return () => window.clearTimeout(t);
  }, [live.state]);
  if (!stale) return null;
  return (
    <div className="banner" role="status">
      <Dot tone="warn" />
      <span className="t-medium">Reconnecting to the server.</span>
      <span>Figures on screen are from {live.lastEvent ? fmtTime(live.lastEvent.toISOString()) : 'the last load'} and will catch up on their own.</span>
    </div>
  );
}

function StatusBar() {
  const live = useLive();
  const qc = useQueryClient();
  const deals = useDeals();
  const age = useAgeSeconds(live.lastEvent);
  const tone: Tone = live.state === 'connected' ? 'good' : live.state === 'connecting' ? 'warn' : 'bad';
  const label = live.state === 'connected' ? 'live' : live.state;
  return (
    <footer className="statusbar">
      <span className="state">
        <Dot tone={tone} />
        {label}
      </span>
      {age !== null && <span>updated {age < 5 ? 'just now' : `${fmtAge(age)} ago`}</span>}
      {deals.data && <span title="server time of the last full load">snapshot {fmtLocal(deals.data.as_of)}</span>}
      <span className="right">
        <span>{window.location.origin}</span>
        <button type="button" className="btn btn-ghost btn-sm" onClick={() => void qc.invalidateQueries()}>
          Refresh
        </button>
      </span>
    </footer>
  );
}
