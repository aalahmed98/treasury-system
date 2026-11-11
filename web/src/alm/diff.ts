// Differences between two versions of a plain JSON document (ALM parameters, FX limits): finding
// them, putting one back, and carrying them as the changes a saved scenario holds.
import type { ParamChange } from '../api/types';

export type Leaf = string | number | boolean | null;
export interface Change {
  path: string[];
  bank: Leaf;
  now: Leaf;
}

function leaves(v: unknown, path: string[], out: Map<string, { path: string[]; value: Leaf }>) {
  if (v !== null && typeof v === 'object' && !Array.isArray(v)) {
    for (const [k, x] of Object.entries(v as Record<string, unknown>)) leaves(x, [...path, k], out);
  } else {
    const value: Leaf = Array.isArray(v) ? JSON.stringify(v) : (v as Leaf);
    out.set(path.join('\u0000'), { path, value });
  }
}

/** Every leaf where the what-if differs from the bank's set. */
/** Every leaf that differs between two versions of a document: changed, added (bank null) or
 * removed (now null). Works on any plain JSON document: ALM parameters, FX limits. */
export function diffParams(bank: object, working: object): Change[] {
  const a = new Map<string, { path: string[]; value: Leaf }>();
  const b = new Map<string, { path: string[]; value: Leaf }>();
  leaves(bank, [], a);
  leaves(working, [], b);
  const out: Change[] = [];
  for (const [k, w] of b) {
    const base = a.get(k);
    if (!base || base.value !== w.value) out.push({ path: w.path, bank: base?.value ?? null, now: w.value });
  }
  for (const [k, base] of a) if (!b.has(k)) out.push({ path: base.path, bank: base.value, now: null });
  return out;
}

export function setPath(obj: unknown, path: string[], value: Leaf) {
  let cur = obj as Record<string, unknown>;
  for (const k of path.slice(0, -1)) {
    if (cur[k] === undefined || cur[k] === null) cur[k] = {};
    cur = cur[k] as Record<string, unknown>;
  }
  const last = path[path.length - 1];
  // Reverting something the bank's version does not have removes it.
  if (value === null) delete cur[last];
  else cur[last] = typeof value === 'string' && value.startsWith('[') ? JSON.parse(value) : value;
}


/** A leaf as it travels to the server: lists as lists, not as the text the diff compares them by. */
const raw = (v: Leaf): unknown => (typeof v === 'string' && v.startsWith('[') ? JSON.parse(v) : v);

/** The what-if as the changes a saved scenario holds: each changed leaf and its new value (null removes it). */
export function toParamChanges(bank: object, working: object): ParamChange[] {
  return diffParams(bank, working).map((c) => ({ path: c.path, value: raw(c.now) }));
}

/** Apply a saved scenario's changes to a parameter document in place. */
export function applyChanges(doc: object, changes: ParamChange[]) {
  for (const c of changes) {
    let cur = doc as Record<string, unknown>;
    for (const k of c.path.slice(0, -1)) {
      if (cur[k] === undefined || cur[k] === null || typeof cur[k] !== 'object') cur[k] = {};
      cur = cur[k] as Record<string, unknown>;
    }
    const last = c.path[c.path.length - 1];
    if (c.value === null || c.value === undefined) delete cur[last];
    else cur[last] = c.value;
  }
}
