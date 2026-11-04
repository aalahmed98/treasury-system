// Formatting, ported one to one from crates/client/src/ui/fmt.rs so both clients print the same.

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const pad = (n: number) => String(n).padStart(2, '0');

/** 1_234_567_890 -> "1.23bn", 45_600_000 -> "45.6m", 1_200 -> "1.20k"; sign kept. */
export function fmtMoneyShort(v: number): string {
  if (!Number.isFinite(v)) return '—';
  const a = Math.abs(v);
  let num = v;
  let suffix = '';
  if (a >= 1e9) {
    num = v / 1e9;
    suffix = 'bn';
  } else if (a >= 1e6) {
    num = v / 1e6;
    suffix = 'm';
  } else if (a >= 1e3) {
    num = v / 1e3;
    suffix = 'k';
  }
  const an = Math.abs(num);
  const digits = suffix === '' || an >= 100 ? num.toFixed(0) : an >= 10 ? num.toFixed(1) : num.toFixed(2);
  return `${digits}${suffix}`;
}

/** "+1.2m" / "-3.4m" / "0". */
export function signed(v: number): string {
  if (v > 0) return `+${fmtMoneyShort(v)}`;
  return fmtMoneyShort(v);
}

/** A decimal string such as "1234567.50" -> "1,234,567.5"; trailing zeros dropped like Decimal::normalize. */
export function fmtAmount(d: string | number): string {
  let s = typeof d === 'number' ? String(d) : d.trim();
  if (s === '' || s === 'NaN') return '—';
  let neg = false;
  if (s.startsWith('-')) {
    neg = true;
    s = s.slice(1);
  }
  let [int, frac = ''] = s.split('.');
  int = int.replace(/^0+(?=\d)/, '');
  frac = frac.replace(/0+$/, '');
  if (int === '' ) int = '0';
  const grouped = int.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  const body = frac ? `${grouped}.${frac}` : grouped;
  return neg && body !== '0' ? `-${body}` : body;
}

export function fmtPct(v: number | null | undefined, dp = 1): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return '—';
  return `${v.toFixed(dp)}%`;
}

export function fmtAge(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  if (secs < 86_400) return `${Math.floor(secs / 3600)}h`;
  return `${Math.floor(secs / 86_400)}d`;
}

/** HH:MM:SS in the user's local time. */
export function fmtTime(iso: string): string {
  const t = new Date(iso);
  return `${pad(t.getHours())}:${pad(t.getMinutes())}:${pad(t.getSeconds())}`;
}

/** dd-Mon HH:MM:SS, local. */
export function fmtLocal(iso: string): string {
  const t = new Date(iso);
  return `${pad(t.getDate())}-${MONTHS[t.getMonth()]} ${fmtTime(iso)}`;
}

/** A plain date "YYYY-MM-DD" -> "Wed 04 Nov". */
export function fmtDate(d: string): string {
  const [y, m, day] = d.split('-').map(Number);
  const t = new Date(Date.UTC(y, m - 1, day));
  return `${DAYS[t.getUTCDay()]} ${pad(day)} ${MONTHS[m - 1]}`;
}

/** "YYYY-MM-DD" -> "04-Nov-26". */
export function fmtDateShort(d: string): string {
  const [y, m, day] = d.split('-').map(Number);
  return `${pad(day)}-${MONTHS[m - 1]}-${String(y).slice(2)}`;
}

/** "YYYY-MM-DD" -> "04 Nov 2026". */
export function fmtDateLong(d: string): string {
  const [y, m, day] = d.split('-').map(Number);
  return `${pad(day)} ${MONTHS[m - 1]} ${y}`;
}

/** "YYYY-MM-DD" -> "Nov 26" (chart tick) or "November 2026". */
export function fmtMonth(d: string, long = false): string {
  const [y, m] = d.split('-').map(Number);
  if (long) {
    const full = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
    return `${full[m - 1]} ${y}`;
  }
  return `${MONTHS[m - 1]} ${String(y).slice(2)}`;
}

export function secondsBetween(later: string, earlier: string): number {
  return Math.max(0, Math.floor((new Date(later).getTime() - new Date(earlier).getTime()) / 1000));
}

export function titleCase(s: string): string {
  return s.length ? s[0].toUpperCase() + s.slice(1).toLowerCase() : s;
}

/** "RET_LESS_STABLE" -> "ret less stable". */
export function nice(s: string): string {
  return s.replace(/_/g, ' ').toLowerCase();
}
