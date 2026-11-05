// The shared vocabulary as React components: chips, badges, dots, sections, cards, the stat strip,
// the limit gauge and a hook for popups that close on an outside click or Escape.
import { useEffect, useRef, type CSSProperties, type ReactNode } from 'react';

export type Tone = 'neutral' | 'good' | 'warn' | 'bad' | 'info';

export function Chip({ selected, outlined, small, onClick, children, title, disabled }: { selected?: boolean; outlined?: boolean; small?: boolean; onClick?: () => void; children: ReactNode; title?: string; disabled?: boolean }) {
  const cls = ['chip', selected ? 'selected' : '', outlined ? 'outlined' : '', small ? 'chip-sm' : ''].filter(Boolean).join(' ');
  return (
    <button type="button" className={cls} aria-pressed={selected} onClick={onClick} title={title} disabled={disabled}>
      {children}
    </button>
  );
}

export function Badge({ tone = 'neutral', dot, children, title }: { tone?: Tone; dot?: boolean; children: ReactNode; title?: string }) {
  return (
    <span className={`badge ${tone}`} title={title}>
      {dot && <span className="dot" aria-hidden />}
      {children}
    </span>
  );
}

export function Dot({ tone, className }: { tone: Tone | 'violet' | 'magenta'; className?: string }) {
  return <span className={`dot ${tone} ${className ?? ''}`} aria-hidden />;
}

export function Section({ title, meta }: { title: string; meta?: string }) {
  return (
    <div className="section">
      <span className="t-eyebrow">{title}</span>
      {meta && <span className="t-small">{meta}</span>}
    </div>
  );
}

export function Card({ title, width, children, className, style }: { title?: ReactNode; width?: number | string; children: ReactNode; className?: string; style?: CSSProperties }) {
  return (
    <div className={`card ${className ?? ''}`} style={{ width, ...style }}>
      {title && <div className="t-heading">{title}</div>}
      {children}
    </div>
  );
}

export interface StatCell {
  label: string;
  value: string;
  sub: string;
  tone?: Tone | null;
  onClick?: () => void;
}

/** One surface with hairlines between the cells instead of a grid of identical tiles. */
export function StatStrip({ cells, testId, hero }: { cells: StatCell[]; testId?: string; /** The numbers someone signs: 32 px values. */ hero?: boolean }) {
  return (
    <div className={`strip${hero ? ' hero' : ''}`} data-testid={testId}>
      {cells.map((c, i) => {
        const body = (
          <>
            <span className="label">{c.label}</span>
            <span className="value">{c.value}</span>
            <span className="sub" title={c.sub}>
              {c.tone && <Dot tone={c.tone} />}
              {c.sub}
            </span>
          </>
        );
        return c.onClick ? (
          <button type="button" key={i} className="cell clickable" onClick={c.onClick}>
            {body}
          </button>
        ) : (
          <div key={i} className="cell">
            {body}
          </div>
        );
      })}
    </div>
  );
}

/** A bar from zero to `max` filled to `value`, with the hard limit and an optional trigger mark. For a
 * floor the value must stay above the limit; for a ceiling below. */
export function Gauge({ value, limit, trigger, max, floor, width = 100 }: { value: number; limit: number; trigger: number | null; max: number; floor: boolean; width?: number | string }) {
  const top = Math.max(max, value, limit, 1e-9);
  const x = (v: number) => `${Math.min(100, Math.max(0, (v / top) * 100))}%`;
  const breach = floor ? value < limit : value > limit;
  const warn = floor ? trigger !== null && value < trigger : value > limit * 0.9;
  const fill = breach ? 'var(--bad)' : warn ? 'var(--warn)' : 'var(--good)';
  const tip = trigger !== null ? `value ${value.toFixed(1)} · trigger ${trigger.toFixed(1)} · limit ${limit.toFixed(1)}` : `value ${value.toFixed(1)} · limit ${limit.toFixed(1)}`;
  return (
    <div className="gauge" style={{ width }} title={tip} role="img" aria-label={tip}>
      <div className="track" />
      <div className="fill" style={{ width: `max(4px, ${x(value)})`, background: fill }} />
      {trigger !== null && <div className="trigger" style={{ left: x(trigger) }} />}
      <div className="limit" style={{ left: x(limit) }} />
    </div>
  );
}

/** Closes a popup on a click outside its container or on Escape. */
export function useOutsideClose<T extends HTMLElement>(open: boolean, onClose: () => void) {
  const ref = useRef<T>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [open, onClose]);
  return ref;
}

export function Spinner() {
  return (
    <span className="t-small" role="status" aria-live="polite">
      …
    </span>
  );
}
