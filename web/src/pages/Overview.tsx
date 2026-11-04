// The page after login: what matters on the user's desks right now, built from the data the page
// already holds plus the server's balance-sheet block for ALM and treasury-head users.
import { useNavigate } from 'react-router';
import { useActivity, useAlmOverview, useBreaks, useDeals, useFeeds, useLadder, usePositions } from '../api/queries';
import { useSession } from '../api/session';
import type { ActivityRow, BreakView, Deal } from '../api/types';
import { PRODUCT_LABEL } from '../api/types';
import { fmtAmount, fmtDate, fmtDateLong, fmtMoneyShort, fmtPct, fmtTime, titleCase } from '../lib/fmt';
import { limitRows, overviewCells, standingTone } from '../lib/alm';
import { Card, Dot, Gauge, Section, StatStrip, type StatCell, type Tone } from '../components/ui';
import { Bars, BarsPair, LineChart } from '../components/charts';
import { feedHealth, referenceNow } from '../components/shell';

const KINDS = ['MISSING', 'MISMATCH', 'ORPHAN', 'AMBIGUOUS', 'DUPLICATE'] as const;
const MATCH = ['UNMATCHED', 'PARTIAL', 'MATCHED', 'BREAK'] as const;

interface Activity {
  when: string;
  kind: string;
  tone: Tone;
  text: string;
  at: string;
  to?: string;
}

function fromServer(a: ActivityRow): Activity {
  const k = a.kind;
  const tone: Tone = k === 'cancelled' ? 'bad' : k.startsWith('break ') && !k.includes('resolved') && !k.includes('ack') ? 'bad' : k === 'booked' || k === 'parameters' ? 'good' : 'neutral';
  const to = a.break_id !== null ? `/breaks?break=${a.break_id}` : a.deal_id ? `/blotter?deal=${a.deal_id}` : undefined;
  return { when: fmtTime(a.at), kind: k, tone, text: `${a.summary} · ${a.actor} · ${a.desk_code}`, at: a.at, to };
}

/** What the page can say on its own until the server's activity feed arrives. */
function localActivity(deals: Deal[], breaks: BreakView[]): Activity[] {
  const rows: Activity[] = [];
  const recent = [...deals].sort((a, b) => b.updated_at.localeCompare(a.updated_at)).slice(0, 12);
  for (const d of recent) {
    const pair = d.ccy2 ? `${d.ccy1}/${d.ccy2}` : d.ccy1;
    rows.push({
      when: fmtTime(d.updated_at),
      kind: d.version <= 1 ? 'booked' : d.status.toLowerCase(),
      tone: d.status === 'CANCELLED' ? 'bad' : d.status === 'CONFIRMED' || d.status === 'SETTLED' ? 'good' : 'neutral',
      text: `${d.deal_ref} ${PRODUCT_LABEL[d.product]} ${pair} ${fmtAmount(d.notional)} · ${d.match_status.toLowerCase()}`,
      at: d.updated_at,
      to: `/blotter?deal=${d.id}`,
    });
  }
  for (const b of breaks.slice(0, 12)) {
    rows.push({
      when: fmtTime(b.updated_at),
      kind: `break ${b.status.toLowerCase()}`,
      tone: b.status === 'RESOLVED' ? 'good' : 'bad',
      text: `#${b.id} ${b.kind} ${b.deal_ref ?? '—'}${b.field ? ` · ${b.field}` : ''}`,
      at: b.updated_at,
      to: `/breaks?break=${b.id}`,
    });
  }
  return rows.sort((a, b) => b.at.localeCompare(a.at)).slice(0, 14);
}

export function Overview() {
  const navigate = useNavigate();
  const { canSeeAlm } = useSession();
  const deals = useDeals();
  const breaks = useBreaks(false);
  const feeds = useFeeds();
  const positions = usePositions();
  const ladder = useLadder();
  const activity = useActivity(20);
  const alm = useAlmOverview();
  const now = referenceNow(feeds.data);
  const ready = deals.data !== undefined && breaks.data !== undefined && (!canSeeAlm || alm.data !== undefined || alm.isError);

  const rows = deals.data?.deals ?? [];
  const open = (breaks.data ?? []).filter((b) => b.status !== 'RESOLVED');

  const cells: StatCell[] = [];
  if (deals.data && breaks.data) {
    const latest = rows.reduce<string | null>((m, d) => (m === null || d.trade_date > m ? d.trade_date : m), null);
    const onLatest = rows.filter((d) => d.trade_date === latest).length;
    const provisional = rows.filter((d) => !d.confirmed && d.status !== 'CANCELLED').length;
    const unmatched = rows.filter((d) => d.match_status === 'UNMATCHED' || d.match_status === 'PARTIAL').length;
    const withBreaks = rows.filter((d) => d.match_status === 'BREAK').length;
    const sla = open.filter((b) => b.kind === 'MISSING').length;
    const kinds = KINDS.map((k) => [k, open.filter((b) => b.kind === k).length] as const).filter(([, n]) => n > 0);
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
      cells.push({ label: 'Feeds', value: `${healthy}/${total}`, sub: r ? (r.pending > 0 ? `fresh · ${r.pending} events pending` : 'fresh · reconciler idle') : 'reconciler down', tone: healthy === total && r ? 'good' : 'bad' });
    }
  }

  const breaksByKind: [string, number][] = KINDS.map((k) => [titleCase(k), open.filter((b) => b.kind === k).length]);
  const matchCounts: [string, number][] = MATCH.map((m) => [titleCase(m), rows.filter((d) => d.match_status === m).length]);
  const today = now.slice(0, 10);
  const ladderBars: [string, number][] = (ladder.data?.rows ?? [])
    .filter((r) => r.date >= today)
    .slice(0, 5)
    .map((r) => [fmtDate(r.date).slice(4), Number(r.net_expected)]);
  const ladderTitle = ladder.data ? `Expected net cash ${ladder.data.ccy} · next five value dates` : 'Expected net cash';

  const acts: Activity[] = activity.data && activity.data.length ? activity.data.map(fromServer) : localActivity(rows, breaks.data ?? []);
  const largest = [...(positions.data ?? [])].sort((a, b) => Math.abs(Number(b.net_notional)) - Math.abs(Number(a.net_notional))).slice(0, 8);

  return (
    <div className="page" data-ready={ready ? '1' : undefined}>
      <div className="page-title">
        <h1 className="t-title">Overview</h1>
        <span className="t-small">as of {fmtTime(now)}</span>
      </div>
      {deals.data && breaks.data ? <StatStrip cells={cells} testId="trading-strip" /> : <p className="t-muted">loading…</p>}

      {canSeeAlm && <AlmBlock />}

      <Section title="Operations" />
      <div className="row-wrap">
        <Card title="Open breaks by kind" width={300}>
          <Bars data={breaksByKind} color="var(--s2)" fmt={(v) => v.toFixed(0)} />
        </Card>
        <Card title="Match status" width={300}>
          <Bars data={matchCounts} color="var(--s1)" fmt={(v) => v.toFixed(0)} />
        </Card>
        <Card title={ladderTitle} width={380}>
          <Bars data={ladderBars} color="var(--s3)" fmt={fmtMoneyShort} />
          <button type="button" className="btn btn-ghost btn-sm" style={{ color: 'var(--info)', marginTop: 4 }} onClick={() => navigate('/liquidity')}>
            open the liquidity ladder
          </button>
        </Card>
      </div>

      <div className="row-wrap" style={{ marginTop: 10 }}>
        <Card title="Recent activity" width={560}>
          {acts.length === 0 && <div className="t-muted">nothing yet</div>}
          <div className="activity">
            {acts.map((a, i) => (
              <div key={i} className="row-a" role={a.to ? 'link' : undefined} tabIndex={a.to ? 0 : undefined} onClick={() => a.to && navigate(a.to)} onKeyDown={(e) => a.to && e.key === 'Enter' && navigate(a.to)}>
                <span className="t-small">{a.when}</span>
                <span className={`t-medium t-${a.tone === 'neutral' ? 'secondary' : a.tone}`}>{a.kind}</span>
                <span className="text">{a.text}</span>
              </div>
            ))}
          </div>
        </Card>
        <Card title="Largest positions" width={300}>
          {largest.length === 0 && <div className="t-muted">no positions</div>}
          <div className="kv">
            {largest.map((p) => (
              <div key={`${p.desk_code}-${p.ccy}`} style={{ display: 'contents' }}>
                <span className="k">
                  <span className="t-small">{p.desk_code}</span> <span className="t-medium" style={{ color: 'var(--text)' }}>{p.ccy}</span>
                </span>
                <span className={`v ${Number(p.net_notional) < 0 ? 't-bad' : 't-good'}`}>{fmtAmount(p.net_notional)}</span>
              </div>
            ))}
          </div>
        </Card>
      </div>
      <div style={{ height: 12 }} />
    </div>
  );
}

/** The balance-sheet section: the ALM strip, limits and charts. Shared with the ALM dashboard. */
export function AlmBlock() {
  const navigate = useNavigate();
  const alm = useAlmOverview();
  const v = alm.data;
  if (!v) {
    return (
      <>
        <Section title="Balance sheet" />
        <p className="t-muted">{alm.isError ? 'the balance-sheet view is not available' : 'loading the balance-sheet view…'}</p>
      </>
    );
  }
  const cells = overviewCells(v).map((c) => ({ ...c, onClick: () => navigate(`/alm/${c.tool}`) }));
  const limits = limitRows(v);
  const trend = v.lcr_trend.map((p, i) => [i, p.value] as const).filter((p): p is readonly [number, number] => p[1] !== null).map(([x, y]) => [x, y] as [number, number]);
  const labels = v.lcr_trend.map((p) => fmtDate(p.as_of).slice(4));
  const nii: [string, number, number][] = v.nii_months.map((m) => [fmtDate(m.period).slice(7), m.actual, m.budget]);
  const gap: [string, number][] = v.gap_buckets.map((b) => [b.label, b.gap]);
  return (
    <>
      <Section title="Balance sheet" meta={`as of ${fmtDateLong(v.as_of)} · parameters v${v.params_version} · in ${v.reporting_ccy}`} />
      <StatStrip cells={cells} testId="alm-strip" />
      <div className="row-wrap" style={{ marginTop: 10 }}>
        <Card title="Limits" width={470}>
          <table className="table limits">
            <tbody>
              {limits.map((l) => (
                <tr key={l.metric}>
                  <td style={{ width: 18 }}>
                    <Dot tone={standingTone(l.standing)} />
                  </td>
                  <td className="t-secondary">{l.metric}</td>
                  <td className="num t-medium">{l.value}</td>
                  <td className="t-small">{l.limit}</td>
                  <td>
                    <Gauge value={l.gaugeValue} limit={l.gaugeLimit} trigger={l.gaugeTrigger} max={l.gaugeMax} floor={l.floor} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
        <Card title="LCR, last 30 snapshots" width={380}>
          <LineChart series={[{ name: 'LCR', points: trend }]} yFmt={(y) => fmtPct(y, 0)} xFmt={(x) => labels[Math.round(x)] ?? ''} />
        </Card>
        <Card title="NII by month, actual against budget" width={420}>
          <BarsPair data={nii} names={['actual', 'budget']} fmt={fmtMoneyShort} />
        </Card>
        <Card title="Repricing gap by bucket" width={380}>
          <Bars data={gap} color="var(--s3)" fmt={fmtMoneyShort} />
        </Card>
      </div>
    </>
  );
}
