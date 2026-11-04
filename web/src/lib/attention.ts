// What the attention list means to a reader: items that need a decision, and system items about
// the plumbing (a stale feed, the reconciler) which fold into one line per fact. Shared by the
// Overview's list and the bell so the two never disagree.
import type { AttentionItem } from '../api/types';
import { titleCase } from './fmt';

/** A system item is about the plumbing rather than a decision. */
export const isSystem = (it: AttentionItem) => it.target.kind === 'overview' || /feed stale|reconciler/i.test(it.title);

export interface SystemLine {
  severity: AttentionItem['severity'];
  title: string;
  detail: string;
  since: string | null;
}

/** Same-kind system items fold into one line, so three stale feeds read as one fact. */
export function systemLines(items: AttentionItem[]): SystemLine[] {
  const feeds = items.filter((it) => /feed stale/i.test(it.title));
  const rest = items.filter((it) => !/feed stale/i.test(it.title));
  const out: SystemLine[] = [];
  if (feeds.length) {
    const names = feeds.map((f) => titleCase(f.title.replace(/ feed stale.*/i, '')));
    const since = feeds.map((f) => f.since).filter((s): s is string => s !== null).sort()[0] ?? null;
    out.push({
      severity: feeds[0].severity,
      title: feeds.length === 1 ? `${names[0]} feed stale` : `${feeds.length} feeds stale`,
      detail: `${names.join(', ')}; deals from ${feeds.length === 1 ? 'this source' : 'these sources'} may be missing until ${feeds.length === 1 ? 'it' : 'they'} report again`,
      since,
    });
  }
  for (const it of rest) out.push({ severity: it.severity, title: it.title, detail: it.detail, since: it.since });
  return out;
}

export function splitAttention(items: AttentionItem[]) {
  return { decisions: items.filter((it) => !isSystem(it)), system: systemLines(items.filter(isSystem)) };
}
