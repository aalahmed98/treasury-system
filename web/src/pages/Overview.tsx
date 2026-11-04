// The page after login. It answers one question first, "does anything need a decision from me",
// then shows the numbers the user signs, the desk's figures, the operations picture and what
// changed recently. Everything on it is built from data the page already holds.
import { useState } from 'react';
import { useNavigate } from 'react-router';
import { useActivity, useAlmOverview, useAttention, useBreaks, useDeals, useFeeds, useLadder, usePositions } from '../api/queries';
import { useSession } from '../api/session';
import type { ActivityRow, BreakView, Deal } from '../api/types';
import { PRODUCT_LABEL } from '../api/types';
import { fmtAge, fmtAmount, fmtDate, fmtDateLong, fmtMoneyShort, fmtPct, fmtTime, secondsBetween, titleCase } from '../lib/fmt';
import { splitAttention } from '../lib/attention';
import { limitRows, overviewCells, standingTone } from '../lib/alm';
import { Badge, Dot, Gauge, Section, StatStrip, type StatCell, type Tone } from '../components/ui';
import { Bars, BarsPair, LineChart } from '../components/charts';
import { Empty, SkeletonLines, SkeletonStrip } from '../components/feedback';
import { feedHealth, referenceNow, severityTone, targetPath } from '../components/shell';

const KINDS = ['MISSING', 'MISMATCH', 'ORPHAN', 'AMBIGUOUS', 'DUPLICATE'] as const;
const MATCH = ['UNMATCHED', 'PARTIAL', 'MATCHED', 'BREAK'] as const;

interface Activity {
  when: string;
  kind: string;
  tone: Tone;
  text: string;
  at: string;
  to?: string;
  actor: string;
}

function fromServer(a: ActivityRow): Activity {
  const k = a.kind;
  const tone: Tone = k === 'cancelled' ? 'bad' : k.startsWith('break ') && !k.includes('resolved') && !k.includes('ack') ? 'bad' : k === 'booked' || k === 'parameters' ? 'good' : 'neutral';
  const to = a.break_id !== null ? `/breaks?break=${a.break_id}` : a.deal_id ? `/blotter?deal=${a.deal_id}` : undefined;
  return { when: fmtTime(a.at), kind: k, tone, text: `${a.summary} · ${a.actor} · ${a.desk_code}`, at: a.at, to, actor: a.actor };
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
      actor: d.trader_name,
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
      actor: 'reconciler',
    });
  }
  return rows.sort((a, b) => b.at.localeCompare(a.at)).slice(0, 14);
}

/** Runs of the same kind by the same actor become one line: seventeen orphan breaks raised by the
 * reconciler in one tick are one event to the reader. */
function groupActivity(rows: Activity[]): Activity[] {
  const out: Activity[] = [];
  let i = 0;
  while (i < rows.length) {
    let j = i;
    while (j + 1 < rows.length && rows[j + 1].kind === rows[i].kind && rows[j + 1].actor === rows[i].actor) j += 1;
    const n = j - i + 1;
    if (n >= 3) {
      const first = rows[i];
      out.push({ ...first, text: `${n} in a row, for example ${first.text}`, to: first.kind.startsWith('break') ? '/breaks' : first.to });
    } else for (let k = i; k <= j; k++) out.push(rows[k]);
    i = j + 1;
  }
  return out;
}

const SHOW = 6;

/** The first thing on the page: what needs a decision, worst first, each row a link; the plumbing
 * below it in one line per fact. */
function Attention({ now }: { now: string }) {
  const navigate = useNavigate();
  const { data, isLoading, isError, refetch } = useAttention();
  const [all, setAll] = useState(false);
  const items = data ?? [];
  if (isLoading) {
    return (
      <div className="attention" aria-busy>
        <div className="attention-head">
          <span className="t-heading">Needs attention</span>
        </div>
        <div style={{ padding: '10px 14px' }}>
          <SkeletonLines n={3} />
        </div>
      </div>
    );
  }
  if (isError) {
    return (
      <div className="attention" data-testid="attention">
        <Empty tone="warn" title="The attention list could not be loaded" detail="The rest of the page is live; this list refreshes on its own every 15 seconds.">
          <button type="button" className="btn btn-sm" onClick={() => void refetch()}>
            Try again
          </button>
        </Empty>
      </div>
    );
  }
  const { decisions, system } = splitAttention(items);
  const age = (since: string | null) => (since ? fmtAge(secondsBetween(now, since)) : '');
  if (decisions.length === 0 && system.length === 0) {
    return (
      <div className="attention" data-testid="attention">
        <Empty tone="good" title="Nothing needs you right now" detail={`Checked at ${fmtTime(now)}. Breaks, limits outside their triggers, stale feeds and deals waiting on the back office appear here as they arise.`} />
      </div>
    );
  }
  const worst = decisions.length ? severityTone(decisions[0].severity) : 'good';
  const critical = decisions.filter((i) => i.severity === 'CRITICAL').length;
  const shown = all ? decisions : decisions.slice(0, SHOW);
  return (
    <div className="attention" data-testid="attention">
      <div className="attention-head">
        <span className="t-heading">{decisions.length ? 'Needs a decision' : 'Nothing needs a decision'}</span>
        {decisions.length > 0 && <Badge tone={worst}>{decisions.length}</Badge>}
        {critical > 0 && <span className="t-small">{critical} critical</span>}
        <span className="grow" />
        {decisions.length > SHOW && (
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => setAll((a) => !a)}>
            {all ? 'Show fewer' : `Show all ${decisions.length}`}
          </button>
        )}
      </div>
      {shown.map((it, i) => (
        <button key={i} type="button" className="attention-row" onClick={() => navigate(targetPath(it.target))}>
          <Dot tone={severityTone(it.severity)} />
          <span className="what">
            <span className="t-medium">{it.title}</span>
            <span className="t-small">{it.detail}</span>
          </span>
          <span className="t-small">{age(it.since)}</span>
          <span className="arrow" aria-hidden>
            ›
          </span>
        </button>
      ))}
      {system.length > 0 && (
        <div className="attention-system" data-testid="attention-system">
          <span className="t-eyebrow">System health</span>
          {system.map((s, i) => (
            <div key={i} className="attention-sys-row">
              <Dot tone={severityTone(s.severity)} />
              <span className="what">
                <span className="t-medium">{s.title}</span>
                <span className="t-small">{s.detail}</span>
              </span>
              <span className="t-small">{age(s.since)}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/** Horizontal bars with a label and a count: one series, so no legend, the count is the label. */
function HBars({ data, color }: { data: [string, number][]; color: string }) {
  const max = Math.max(1, ...data.map((d) => d[1]));
  return (
    <div className="hbars">
      {data.map(([k, n]) => (
        <div key={k} style={{ display: 'contents' }}>
          <span className="k">{k}</span>
          <span>
            <span className="bar" style={{ width: `${(n / max) * 100}%`, background: n > 0 ? color : 'var(--hairline)', display: 'block' }} />
          </span>
          <span className="n">{n}</span>
        </div>
      ))}
    </div>
  );
}

export function Overview() {
  const navigate = useNavigate();
  const { canSeeAlm, desks } = useSession();
  // Someone who books deals reads their desk first; the balance sheet leads for everyone else.
  const tradesFirst = desks.some((d) => d.can_book);
  const deals = useDeals();
  const breaks = useBreaks(false);
  const feeds = useFeeds();
  const positions = usePositions();
  const ladder = useLadder();
  const activity = useActivity(24);
  const alm = useAlmOverview();
  const now = referenceNow(feeds.data);
  const ready = deals.data !== undefined && breaks.data !== undefined && (!canSeeAlm || alm.data !== undefined || alm.isError);

  const rows = deals.data?.deals ?? [];
  const open = (breaks.data ?? []).filter((b) => b.status !== 'RESOLVED');
  const today = now.slice(0, 10);

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
      { label: latest === today ? 'Traded today' : latest ? `Last traded ${fmtDate(latest)}` : 'Traded', value: String(onLatest), sub: `${rows.length} deals in the blotter`, onClick: () => navigate('/blotter') },
      { label: 'Provisional', value: String(provisional), sub: 'awaiting back-office confirmation', tone: provisional > 0 ? 'warn' : 'good', onClick: () => navigate('/blotter?filter=provisional') },
      { label: 'Unmatched', value: String(unmatched), sub: `${withBreaks} with breaks`, tone: withBreaks > 0 ? 'bad' : unmatched > 0 ? 'warn' : 'good', onClick: () => navigate('/blotter?filter=unmatched') },
      { label: 'Open breaks', value: String(open.length), sub: kinds.length ? kinds.map(([k, n]) => `${n} ${k.toLowerCase()}`).join(' · ') : 'none open', tone: open.length > 0 ? 'bad' : 'good', onClick: () => navigate('/breaks') },
      { label: 'SLA breaches', value: String(sla), sub: 'sources missing past their expected lag', tone: sla > 0 ? 'bad' : 'good', onClick: () => navigate('/breaks?kind=MISSING') },
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
  const ladderBars: [string, number][] = (ladder.data?.rows ?? [])
    .filter((r) => r.date >= today)
    .slice(0, 5)
    .map((r) => [fmtDate(r.date).slice(4), Number(r.net_expected)]);
  const ladderTitle = ladder.data ? `Expected net cash ${ladder.data.ccy} · next five value dates` : 'Expected net cash';

  const acts: Activity[] = groupActivity(activity.data && activity.data.length ? activity.data.map(fromServer) : localActivity(rows, breaks.data ?? [])).slice(0, 10);
  const largest = [...(positions.data ?? [])].sort((a, b) => Math.abs(Number(b.net_notional)) - Math.abs(Number(a.net_notional))).slice(0, 8);

  return (
    <div className="page" data-ready={ready ? '1' : undefined}>
      <div className="page-title">
        <h1 className="t-title">Overview</h1>
        <span className="t-small">as of {fmtTime(now)}</span>
      </div>
      <Attention now={now} />

      {canSeeAlm && !tradesFirst && <AlmBlock />}

      <Section title="Trading" meta="your desks, from the blotter and the breaks" />
      {deals.data && breaks.data ? <StatStrip cells={cells} testId="trading-strip" /> : <SkeletonStrip cells={6} />}

      {canSeeAlm && tradesFirst && <AlmBlock />}

      <Section title="Operations" />
      <div className="panel-row ov-ops">
        <div className="panel">
          <div className="t-heading">Reconciliation</div>
          <div className="two-up">
            <div>
              <div className="t-label" style={{ marginBottom: 6 }}>
                Open breaks by kind
              </div>
              <HBars data={breaksByKind} color="var(--s2)" />
            </div>
            <div>
              <div className="t-label" style={{ marginBottom: 6 }}>
                Match status, all deals
              </div>
              <HBars data={matchCounts} color="var(--s1)" />
            </div>
          </div>
        </div>
        <div className="panel">
          <div className="t-heading">{ladderTitle}</div>
          {ladder.data ? <Bars data={ladderBars} color="var(--s3)" fmt={fmtMoneyShort} /> : <SkeletonLines n={4} />}
          <button type="button" className="btn btn-ghost btn-sm" style={{ color: 'var(--info)', marginTop: 4 }} onClick={() => navigate('/liquidity')}>
            open the liquidity ladder
          </button>
        </div>
      </div>

      <div className="ov-grid ov-bottom">
        <div>
          <Section title="Recent activity" meta="the last changes on your desks; a run of the same event is one line" />
          {activity.isLoading && acts.length === 0 && <SkeletonLines n={5} />}
          {!activity.isLoading && acts.length === 0 && <div className="t-muted">nothing yet</div>}
          <div className="activity">
            {acts.map((a, i) => (
              <div key={i} className="row-a" role={a.to ? 'link' : undefined} tabIndex={a.to ? 0 : undefined} onClick={() => a.to && navigate(a.to)} onKeyDown={(e) => a.to && e.key === 'Enter' && navigate(a.to)}>
                <span className="t-small">{a.when}</span>
                <span className={`t-medium t-${a.tone === 'neutral' ? 'secondary' : a.tone}`}>{a.kind}</span>
                <span className="text">{a.text}</span>
              </div>
            ))}
          </div>
        </div>
        <div>
          <Section title="Largest positions" meta="net notional" />
          {positions.isLoading && <SkeletonLines n={5} />}
          {positions.data && largest.length === 0 && <div className="t-muted">no positions on your desks</div>}
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
        </div>
      </div>
      <div style={{ height: 12 }} />
    </div>
  );
}

const SIGNED = new Set(['lcr', 'nsfr', 'stress', 'funding']);
const STANDING_WORD: Record<string, string> = { good: 'within limits', warn: 'watch', bad: 'breach', neutral: 'no data', info: '' };

/** The balance-sheet section: the four numbers the head signs as a hero row, the income figures
 * beside them, then limits and charts. Shared with the ALM dashboard. */
export function AlmBlock() {
  const navigate = useNavigate();
  const alm = useAlmOverview();
  const v = alm.data;
  if (!v) {
    return (
      <>
        <Section title="Balance sheet" />
        {alm.isError ? (
          <div className="row">
            <span className="t-muted">the balance-sheet view is not available</span>
            <button type="button" className="btn btn-sm" onClick={() => void alm.refetch()}>
              Try again
            </button>
          </div>
        ) : (
          <SkeletonStrip cells={4} />
        )}
      </>
    );
  }
  const all = overviewCells(v).map((c) => ({ ...c, onClick: () => navigate(`/alm/${c.tool}`) }));
  const p = v.previous;
  const since = (now: number | null, then: number | null | undefined, unit: string) => {
    if (!p || now === null || then === null || then === undefined) return '';
    const d = now - then;
    return ` · ${Math.abs(d) < 0.05 ? 'unchanged' : `${d > 0 ? '+' : ''}${d.toFixed(1)} ${unit}`} since ${fmtDate(p.as_of).slice(4)}`;
  };
  const delta: Record<string, string> = {
    lcr: since(v.lcr, p?.lcr, 'pp'),
    nsfr: since(v.nsfr, p?.nsfr, 'pp'),
    stress: since(v.worst_eve_loss_pct_tier1, p?.worst_eve_loss_pct_tier1, 'pp'),
    funding: since(v.top10_pct, p?.top10_pct, 'pp'),
  };
  const signedCells = all.filter((c) => SIGNED.has(c.tool)).map((c) => ({ ...c, sub: `${STANDING_WORD[c.tone ?? 'neutral']} · ${c.sub}${delta[c.tool] ?? ''}` }));
  const income = all.filter((c) => !SIGNED.has(c.tool));
  const r = v.ratios;
  const ratioCells: StatCell[] = r
    ? [
        { label: 'Loans to deposits', value: fmtPct(r.loan_to_deposit), sub: 'loan book over customer and bank deposits' },
        { label: 'Liquid assets', value: fmtPct(r.liquid_assets_to_assets), sub: 'cash and unencumbered liquid securities, of assets' },
        { label: 'Cost of funds', value: fmtPct(r.cost_of_funds, 2), sub: 'average rate on interest-bearing liabilities' },
        { label: 'Asset yield', value: fmtPct(r.asset_yield, 2), sub: 'average rate on earning assets' },
        { label: 'Net spread', value: fmtPct(r.net_spread, 2), sub: 'asset yield less cost of funds' },
        { label: 'Equity to assets', value: fmtPct(r.equity_to_assets), sub: 'shareholders’ funds over total assets' },
      ]
    : [];
  const limits = limitRows(v);
  const trend = v.lcr_trend.map((p, i) => [i, p.value] as const).filter((p): p is readonly [number, number] => p[1] !== null).map(([x, y]) => [x, y] as [number, number]);
  const labels = v.lcr_trend.map((p) => fmtDate(p.as_of).slice(4));
  const nii: [string, number, number][] = v.nii_months.map((m) => [fmtDate(m.period).slice(7), m.actual, m.budget]);
  const gap: [string, number][] = v.gap_buckets.map((b) => [b.label, b.gap]);
  return (
    <>
      <Section title="Balance sheet" meta={`as of ${fmtDateLong(v.as_of)} · parameters v${v.params_version} · in ${v.reporting_ccy}`} />
      <StatStrip cells={signedCells} testId="alm-strip" hero />
      <StatStrip cells={income} testId="alm-strip-2" />
      {ratioCells.length > 0 && <StatStrip cells={ratioCells} testId="alm-ratios" />}
      <div className="panel-row ov-bs">
        <div className="panel">
          <div className="t-heading">Limits</div>
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
        </div>
        <div className="panel">
          <div className="t-heading">LCR, last 30 snapshots</div>
          <LineChart series={[{ name: 'LCR', points: trend }]} yFmt={(y) => fmtPct(y, 0)} xFmt={(x) => labels[Math.round(x)] ?? ''} />
        </div>
        <div className="panel">
          <div className="t-heading">NII by month, actual against budget</div>
          <BarsPair data={nii} names={['actual', 'budget']} fmt={fmtMoneyShort} />
        </div>
        <div className="panel">
          <div className="t-heading">Repricing gap by bucket</div>
          <Bars data={gap} color="var(--s3)" fmt={fmtMoneyShort} labels="extremes" />
        </div>
      </div>
    </>
  );
}
