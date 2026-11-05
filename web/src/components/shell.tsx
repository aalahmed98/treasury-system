// The frame around every page: the top bar (wordmark, desk, tabs, the search box that opens the
// command palette, one feeds pill, the attention bell that counts what is new, the user menu with
// the theme and the server), the connection banner, and the status bar with how fresh the data is.
import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { NavLink, useNavigate } from 'react-router';
import { useQueryClient } from '@tanstack/react-query';
import { useSession } from '../api/session';
import { useLive } from '../api/live';
import { useAttention, useBreaks, useDeals, useFeeds, useFtp, useFxRisk } from '../api/queries';
import type { AttentionItem, AttentionTarget, FeedsResponse } from '../api/types';
import { fmtAge, fmtDateShort, fmtLocal, fmtTime, secondsBetween, titleCase } from '../lib/fmt';
import { readPref, writePref } from '../lib/prefs';
import { splitAttention } from '../lib/attention';
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
    case 'risk':
      return '/risk';
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

/** The identity of an attention item for "seen" bookkeeping: the same title and detail is the same item. */
export const attentionKey = (it: AttentionItem) => `${it.title}|${it.detail}`;

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
  const { homeDesk, canSeeAlm, canSeeRisk } = useSession();
  const deals = useDeals();
  const breaks = useBreaks(false);
  const risk = useFxRisk(canSeeRisk);
  const open = (breaks.data ?? []).filter((b) => b.status !== 'RESOLVED').length;
  const breached = (risk.data?.limits ?? []).filter((l) => l.status === 'BREACH').length;
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
        {canSeeRisk && <Tab to="/risk" label="Risk" count={breached > 0 ? String(breached) : undefined} tone="bad" />}
        {canSeeAlm && <Tab to="/alm" label="ALM" />}
      </nav>
      <div className="cluster">
        <SearchButton />
        <FeedsPill />
        <Bell />
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

interface FeedLine {
  name: string;
  tone: Tone;
  age: string;
  detail: string;
}

function feedLines(f: FeedsResponse): FeedLine[] {
  const reference = referenceNow(f);
  const lines: FeedLine[] = [];
  for (const feed of f.feeds) {
    if (feed.source === 'INTERNAL' || feed.source === 'USER') continue;
    const age = feed.last_heartbeat_at ? secondsBetween(reference, feed.last_heartbeat_at) : null;
    lines.push({ name: titleCase(feed.source), tone: feedHealth(age), age: age === null ? 'never' : `${fmtAge(age)} ago`, detail: `${feed.events_total} events · ${feed.duplicates_dropped} duplicates dropped · last event ${feed.last_event_at ? fmtLocal(feed.last_event_at) : 'never'}` });
  }
  if (f.recon) {
    const lag = secondsBetween(f.server_time, f.recon.last_tick_at);
    lines.push({ name: 'Reconciler', tone: reconHealth(lag), age: `${fmtAge(lag)} ago`, detail: `${f.recon.pending > 0 ? `${f.recon.pending} events pending · ` : 'idle · '}clock ${fmtLocal(f.recon.now)}` });
  } else {
    lines.push({ name: 'Reconciler', tone: 'bad', age: 'never', detail: 'the reconciliation worker has never reported' });
  }
  return lines;
}

/** One pill for the health of every source and the reconciler; the detail is a click away. */
function FeedsPill() {
  const { data: f } = useFeeds();
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const ref = useOutsideClose<HTMLDivElement>(open, close);
  const lines = useMemo(() => (f ? feedLines(f) : []), [f]);
  const worst: Tone = lines.some((l) => l.tone === 'bad') ? 'bad' : lines.some((l) => l.tone === 'warn') ? 'warn' : lines.length ? 'good' : 'neutral';
  const stale = lines.filter((l) => l.tone !== 'good').length;
  const label = !f ? 'feeds' : stale === 0 ? 'feeds ok' : `${stale} ${stale === 1 ? 'feed' : 'feeds'} stale`;
  return (
    <div className="anchor" ref={ref}>
      <button type="button" className="pill" onClick={() => setOpen((o) => !o)} aria-expanded={open} aria-haspopup="dialog" title="source feeds and the reconciler">
        <Dot tone={worst} />
        {label}
      </button>
      {open && (
        <div className="popup" role="dialog" aria-label="Feeds" style={{ minWidth: 320 }}>
          <div className="t-heading" style={{ marginBottom: 4 }}>
            Feeds
          </div>
          {lines.map((l) => (
            <div key={l.name} className="feed-line">
              <Dot tone={l.tone} />
              <span className="t-medium">{l.name}</span>
              <span className="t-small">{l.age}</span>
              <span className="t-small feed-detail">{l.detail}</span>
            </div>
          ))}
          {lines.length === 0 && <div className="t-muted">no feed status yet</div>}
        </div>
      )}
    </div>
  );
}

function BellIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M4 11V7a4 4 0 0 1 8 0v4l1.2 1.5H2.8L4 11Z" />
      <path d="M6.5 13.5a1.5 1.5 0 0 0 3 0" />
    </svg>
  );
}

/** The bell counts what is new since the user last opened it; the full list lives on the Overview. */
function Bell() {
  const { data } = useAttention();
  const items = data ?? [];
  const [open, setOpen] = useState(false);
  const [seen, setSeen] = useState<Set<string>>(() => new Set(readPref<string[]>('bell-seen', [])));
  const close = useCallback(() => setOpen(false), []);
  const ref = useOutsideClose<HTMLDivElement>(open, close);
  const navigate = useNavigate();
  const fresh = items.filter((it) => !seen.has(attentionKey(it)));
  const { decisions, system } = splitAttention(items);
  const tone: Tone = fresh.length ? severityTone(fresh[0].severity) : 'neutral';
  const reference = new Date().toISOString();
  const toggle = () => {
    if (!open) {
      const keys = items.map(attentionKey);
      writePref('bell-seen', keys);
      // Marked seen once the popup closes, so the "new" tags are still there while it is open.
      window.setTimeout(() => setSeen(new Set(keys)), 0);
    }
    setOpen((o) => !o);
  };
  const go = (it: AttentionItem) => {
    setOpen(false);
    navigate(targetPath(it.target));
  };
  return (
    <div className="anchor" ref={ref}>
      <button type="button" className={`bell ${fresh.length ? tone : ''}`} onClick={toggle} aria-expanded={open} aria-haspopup="dialog" title={items.length ? `${items.length} outstanding, ${fresh.length} new since you last looked` : 'nothing needs attention on your desks'}>
        <BellIcon />
        {fresh.length > 0 && <span>{fresh.length}</span>}
      </button>
      {open && (
        <div className="popup popup-wide" role="dialog" aria-label="Needs attention">
          <div className="row">
            <span className="t-heading">Needs attention</span>
            {items.length > 0 && <Badge tone={items.length ? severityTone(items[0].severity) : 'neutral'}>{items.length}</Badge>}
            {fresh.length > 0 && <span className="t-small">{fresh.length} new since you last looked</span>}
          </div>
          {items.length === 0 ? (
            <p className="t-muted" style={{ marginTop: 6 }}>
              Nothing outstanding for your desks.
            </p>
          ) : (
            <div className="popup-list">
              {decisions.map((it, i) => (
                <div
                  key={i}
                  className="popup-row"
                  role="button"
                  tabIndex={0}
                  onClick={() => go(it)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      go(it);
                    }
                  }}
                >
                  <Dot tone={severityTone(it.severity)} />
                  <div>
                    <div className="t-medium">
                      {it.title}
                      {!seen.has(attentionKey(it)) && (
                        <Badge tone="info" title="appeared since you last opened the bell">
                          new
                        </Badge>
                      )}
                    </div>
                    <div className="t-small">{it.detail}</div>
                  </div>
                  <span className="t-small">{it.since ? fmtAge(secondsBetween(reference, it.since)) : '—'}</span>
                </div>
              ))}
              {system.length > 0 && (
                <div className="popup-system">
                  <div className="t-eyebrow">System health</div>
                  {system.map((s, i) => (
                    <div key={i} className="popup-row static">
                      <Dot tone={severityTone(s.severity)} />
                      <div>
                        <div className="t-medium">{s.title}</div>
                        <div className="t-small">{s.detail}</div>
                      </div>
                      <span className="t-small">{s.since ? fmtAge(secondsBetween(reference, s.since)) : '—'}</span>
                    </div>
                  ))}
                </div>
              )}
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
        <div className="popup" role="menu" style={{ minWidth: 260 }}>
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
          <div className="row" style={{ justifyContent: 'space-between' }}>
            <span className="t-eyebrow">Theme</span>
            <ThemeSwitch />
          </div>
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
            Keyboard shortcuts <Kbd>?</Kbd>
          </button>
          <button type="button" className="menu-item" role="menuitem" onClick={() => void logout()}>
            Log out
          </button>
          <div className="t-small" style={{ marginTop: 6 }} title="the server these pages come from">
            Server {window.location.host}
          </div>
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

/** The FTP curves in force, for every desk: when they were published and how many; amber once a
 * business day has passed without a new publication. */
function FtpChip() {
  const ftp = useFtp();
  const navigate = useNavigate();
  const c = ftp.data;
  if (!c) return null;
  const cur = c.current;
  const text = cur ? `FTP ${fmtDateShort(cur.as_of)} · ${cur.curves.length} ${cur.curves.length === 1 ? 'curve' : 'curves'}` : 'FTP not published';
  const title = cur
    ? `${c.stale ? `Not published for ${fmtDateShort(c.date)}; ` : ''}published ${fmtLocal(cur.published_at)} by ${cur.published_by ?? 'the application'}`
    : 'ALM has not published FTP curves yet';
  return (
    <button type="button" className="btn btn-ghost btn-sm ftp-chip" data-testid="ftp-chip" onClick={() => navigate('/ftp')} title={title} onMouseEnter={() => warmFor('/ftp')}>
      <Dot tone={!cur ? 'bad' : c.stale ? 'warn' : 'good'} />
      {text}
    </button>
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
        <FtpChip />
        <button type="button" className="btn btn-ghost btn-sm" onClick={() => void qc.invalidateQueries()} title="fetch everything again">
          Refresh
        </button>
      </span>
    </footer>
  );
}
