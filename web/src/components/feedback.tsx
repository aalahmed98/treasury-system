// What the page says back: toasts for the outcome of an action, skeletons while the first data
// loads, empty states that say what the absence means, and the keyboard hint.
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useNavigate } from 'react-router';
import { Dot, type Tone } from './ui';

export interface ToastInput {
  title: string;
  detail?: string;
  tone?: Tone;
  /** A link shown on the toast, e.g. to the deal just booked. */
  action?: { label: string; to: string };
}

interface Toast extends ToastInput {
  id: number;
}

const Ctx = createContext<(t: ToastInput) => void>(() => {});

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const next = useRef(1);
  const dismiss = useCallback((id: number) => setToasts((ts) => ts.filter((t) => t.id !== id)), []);
  const push = useCallback(
    (t: ToastInput) => {
      const id = next.current++;
      setToasts((ts) => [...ts.slice(-3), { ...t, id }]);
      window.setTimeout(() => dismiss(id), t.tone === 'bad' ? 10_000 : 6_000);
    },
    [dismiss],
  );
  return (
    <Ctx.Provider value={push}>
      {children}
      <Toasts toasts={toasts} dismiss={dismiss} />
    </Ctx.Provider>
  );
}

/** `toast({ title, detail, tone, action })` from any signed-in page. */
export function useToast() {
  return useContext(Ctx);
}

function Toasts({ toasts, dismiss }: { toasts: Toast[]; dismiss: (id: number) => void }) {
  const navigate = useNavigate();
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className="toast" data-testid="toast">
          <Dot tone={t.tone ?? 'neutral'} className="toast-dot" />
          <div className="toast-body">
            <div className="t-medium">{t.title}</div>
            {t.detail && <div className="t-small">{t.detail}</div>}
            {t.action && (
              <button
                type="button"
                className="toast-action"
                onClick={() => {
                  dismiss(t.id);
                  navigate(t.action!.to);
                }}
              >
                {t.action.label}
              </button>
            )}
          </div>
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => dismiss(t.id)} aria-label="dismiss">
            ✕
          </button>
        </div>
      ))}
    </div>
  );
}

/** A still block where text will be; no shimmer, nothing moves on load. */
export function Skeleton({ w = '100%', h = 12, style }: { w?: number | string; h?: number; style?: React.CSSProperties }) {
  return <span className="skeleton" style={{ width: w, height: h, ...style }} aria-hidden />;
}

export function SkeletonLines({ n = 4, widths = ['92%', '70%', '84%', '55%'] }: { n?: number; widths?: string[] }) {
  return (
    <div className="stack" aria-hidden>
      {Array.from({ length: n }, (_, i) => (
        <Skeleton key={i} w={widths[i % widths.length]} />
      ))}
    </div>
  );
}

/** The stat strip's silhouette while its figures load. */
export function SkeletonStrip({ cells = 6, testId }: { cells?: number; testId?: string }) {
  return (
    <div className="strip" aria-busy data-testid={testId}>
      {Array.from({ length: cells }, (_, i) => (
        <div key={i} className="cell">
          <Skeleton w="45%" h={10} />
          <Skeleton w="30%" h={22} style={{ marginTop: 4 }} />
          <Skeleton w="70%" h={10} style={{ marginTop: 4 }} />
        </div>
      ))}
    </div>
  );
}

/** Rows of a table while the list loads; the real header is already there above it. */
export function SkeletonRows({ n = 12, height = 22 }: { n?: number; height?: number }) {
  return (
    <div className="skeleton-rows" aria-busy>
      {Array.from({ length: n }, (_, i) => (
        <div key={i} className="skeleton-row" style={{ height }}>
          <Skeleton w={`${38 + ((i * 29) % 45)}%`} h={10} />
        </div>
      ))}
    </div>
  );
}

/** The page is working: nothing is wrong, nothing is here, and this is why. */
export function Empty({ title, detail, children, tone = 'neutral', testId }: { title: string; detail?: string; children?: ReactNode; tone?: Tone; testId?: string }) {
  return (
    <div className="empty" data-testid={testId}>
      <div className="row">
        <Dot tone={tone} />
        <span className="t-medium">{title}</span>
      </div>
      {detail && <div className="t-small">{detail}</div>}
      {children && <div className="row" style={{ marginTop: 6 }}>{children}</div>}
    </div>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="kbd">{children}</kbd>;
}

/** Seconds since `at`, ticking once a second while mounted. */
export function useAgeSeconds(at: Date | string | null): number | null {
  const [, tick] = useState(0);
  useEffect(() => {
    const t = window.setInterval(() => tick((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, []);
  return useMemo(() => (at === null ? null : Math.max(0, Math.floor((Date.now() - new Date(at).getTime()) / 1000))), [at]);
}
