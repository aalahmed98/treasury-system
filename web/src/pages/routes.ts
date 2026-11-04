// The pages that load on demand. The Overview ships with the shell; the trading pages and the ALM
// workbench are their own chunks, fetched when a tab is hovered or opened so the first paint
// stays small on a desk PC.
export const load = {
  blotter: () => import('./Blotter'),
  breaks: () => import('./Breaks'),
  liquidity: () => import('./Ladder'),
  alm: () => import('./alm/Alm'),
};

export type PageKey = keyof typeof load;

export const pageFor = (path: string): PageKey | null => (path.startsWith('/blotter') ? 'blotter' : path.startsWith('/breaks') ? 'breaks' : path.startsWith('/liquidity') ? 'liquidity' : path.startsWith('/alm') ? 'alm' : null);

const warmed = new Set<PageKey>();
/** Fetches a page's chunk ahead of the click; a second call is free. */
export function warm(key: PageKey | null) {
  if (!key || warmed.has(key)) return;
  warmed.add(key);
  void load[key]().catch(() => warmed.delete(key));
}
