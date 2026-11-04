// The page after login: what matters on the user's desks right now.
import { useNavigate } from 'react-router';
import { useBreaks, useDeals, useFeeds } from '../api/queries';
import { fmtDate, fmtTime } from '../lib/fmt';
import { StatStrip, type StatCell } from '../components/ui';
import { feedHealth, referenceNow } from '../components/shell';

export function Overview() {
  const navigate = useNavigate();
  const deals = useDeals();
  const breaks = useBreaks(false);
  const feeds = useFeeds();
  const ready = deals.data !== undefined && breaks.data !== undefined;
  const now = referenceNow(feeds.data);

  const cells: StatCell[] = [];
  if (deals.data && breaks.data) {
    const rows = deals.data.deals;
    const latest = rows.reduce<string | null>((m, d) => (m === null || d.trade_date > m ? d.trade_date : m), null);
    const onLatest = rows.filter((d) => d.trade_date === latest).length;
    const provisional = rows.filter((d) => !d.confirmed && d.status !== 'CANCELLED').length;
    const unmatched = rows.filter((d) => d.match_status === 'UNMATCHED' || d.match_status === 'PARTIAL').length;
    const withBreaks = rows.filter((d) => d.match_status === 'BREAK').length;
    const open = breaks.data.filter((b) => b.status !== 'RESOLVED');
    const sla = open.filter((b) => b.kind === 'MISSING').length;
    const kinds = ['MISSING', 'MISMATCH', 'ORPHAN', 'AMBIGUOUS', 'DUPLICATE'].map((k) => [k, open.filter((b) => b.kind === k).length] as const).filter(([, n]) => n > 0);
    cells.push(
      { label: latest ? `Deals on ${fmtDate(latest)}` : 'Deals', value: String(onLatest), sub: `${rows.length} in the blotter`, onClick: () => navigate('/blotter') },
      { label: 'Provisional', value: String(provisional), sub: 'awaiting back-office confirmation', tone: provisional > 0 ? 'warn' : 'good', onClick: () => navigate('/blotter?filter=provisional') },
      { label: 'Unmatched', value: String(unmatched), sub: `${withBreaks} with breaks`, tone: withBreaks > 0 ? 'bad' : unmatched > 0 ? 'warn' : 'good', onClick: () => navigate('/blotter') },
      { label: 'Open breaks', value: String(open.length), sub: kinds.length ? kinds.map(([k, n]) => `${n} ${k.toLowerCase()}`).join(' · ') : 'none open', tone: open.length > 0 ? 'bad' : 'good', onClick: () => navigate('/breaks') },
      { label: 'SLA breaches', value: String(sla), sub: 'sources missing past their expected lag', tone: sla > 0 ? 'bad' : 'good', onClick: () => navigate('/breaks') },
    );
    if (feeds.data) {
      let healthy = 0;
      let total = 0;
      for (const f of feeds.data.feeds) {
        if (f.source === 'INTERNAL' || f.source === 'USER') continue;
        total += 1;
        const age = f.last_heartbeat_at ? Math.max(0, Math.floor((new Date(now).getTime() - new Date(f.last_heartbeat_at).getTime()) / 1000)) : null;
        if (feedHealth(age) === 'good') healthy += 1;
      }
      const r = feeds.data.recon;
      cells.push({
        label: 'Feeds',
        value: `${healthy}/${total}`,
        sub: r ? (r.pending > 0 ? `fresh · ${r.pending} events pending` : 'fresh · reconciler idle') : 'reconciler down',
        tone: healthy === total && r ? 'good' : 'bad',
      });
    }
  }

  return (
    <div className="page" data-ready={ready ? '1' : undefined}>
      <div className="page-title">
        <h1 className="t-title">Overview</h1>
        <span className="t-small">as of {fmtTime(now)}</span>
      </div>
      {ready ? <StatStrip cells={cells} testId="trading-strip" /> : <p className="t-muted">loading…</p>}
    </div>
  );
}
