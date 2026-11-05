// The PMU headline figures as stat cells, shared by the PMU overview and the main Overview (kept
// out of the PMU chunk so the main page does not load the whole tab for six numbers).
import type { PmuHeadline } from '../../api/types';
import type { StatCell } from '../../components/ui';
import { fmtMoneyShort, signed } from '../../lib/fmt';

const bhd = fmtMoneyShort;
const pct = (v: number) => (Number.isFinite(v) ? `${v.toFixed(2)}%` : '—');

function since(now: number, prev: number | undefined): string {
  if (prev === undefined) return '';
  const d = now - prev;
  if (Math.abs(d) < 0.5) return 'unchanged since yesterday';
  return `${signed(d)} since yesterday`;
}

export function pmuHeadlineCells(h: PmuHeadline, p: PmuHeadline | undefined, go: (path: string) => void): StatCell[] {
  return [
    { label: 'Portfolio size', value: bhd(h.portfolio_face_bhd), sub: `face held · market value ${bhd(h.market_value_bhd)}`, onClick: () => go('/pmu/positions') },
    { label: 'Net MTM', value: bhd(h.net_mtm_bhd), sub: `bonds and swaps · ${since(h.net_mtm_bhd, p?.net_mtm_bhd)}`, tone: h.net_mtm_bhd < 0 ? 'warn' : 'good', onClick: () => go('/pmu/risk') },
    { label: 'Bond carry, month to date', value: bhd(h.mtd_carry_bhd), sub: `after FTP · ${bhd(h.daily_carry_bhd)} today`, tone: h.mtd_carry_bhd < 0 ? 'bad' : 'good', onClick: () => go('/pmu/income') },
    { label: 'Net DV01', value: signed(h.net_dv01_bhd), sub: `BHD per bp, after hedges · ${since(h.net_dv01_bhd, p?.net_dv01_bhd)}`, onClick: () => go('/pmu/risk') },
    { label: 'NII, month to date', value: bhd(h.mtd_nii_bhd), sub: 'carry after FTP plus swap carry', tone: h.mtd_nii_bhd < 0 ? 'bad' : 'good', onClick: () => go('/pmu/income') },
    { label: 'Portfolio yield', value: pct(h.portfolio_yield), sub: `effective rate on book value · duration ${h.mod_duration.toFixed(2)}`, onClick: () => go('/pmu/risk') },
  ];
}
