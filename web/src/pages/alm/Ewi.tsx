// Early-warning indicators and the contingency funding plan's stage they add up to.
import type { EwiIndicator, EwiResult, EwiStatus } from '../../api/types';
import { Badge, Dot, type Tone } from '../../components/ui';
import { LineChart } from '../../components/charts';
import { useAlm, useTool } from '../../alm/state';
import { AssumptionsHeader, Group, Num, ResultsHeader, ToolLayout } from '../../alm/tool';
import { fmtDate, fmtDateLong } from '../../lib/fmt';

export const ewiTone = (s: EwiStatus): Tone => (s === 'RED' ? 'bad' : s === 'AMBER' ? 'warn' : 'good');
const STAGE_TONE: Record<string, Tone> = { NORMAL: 'good', ALERT: 'warn', CRISIS: 'bad' };
const LABELS: Record<string, string> = {
  deposits_5d: 'Deposits, 5-day change (%)',
  retail_5d: 'Retail deposits, 5-day change (%)',
  wholesale_30d: 'Wholesale due in 30 days (%)',
  top10: 'Ten largest depositors (%)',
  lcr_5d: 'LCR, 5-day change (pp)',
  bhibor_5d: '3M BHIBOR, 5-day change (bp)',
  peg_spread_5d: 'BHIBOR over SOFR, 5-day change (bp)',
  survival: 'Survival under stress (days)',
};

function Assumptions() {
  const { working, edit, bank } = useAlm();
  if (!working || !bank) return null;
  const t = working.ewi.thresholds;
  return (
    <>
      <AssumptionsHeader />
      <Group title="Thresholds" help="Amber and red levels for each indicator. Two reds put the plan at crisis; one red or two ambers at alert.">
        <div className="factor-grid" style={{ gridTemplateColumns: 'minmax(0, 1fr) auto auto' }}>
          <span />
          <span className="h">Amber</span>
          <span className="h">Red</span>
          {Object.keys(LABELS).map((k) => (
            <div key={k} style={{ display: 'contents' }}>
              <span className="k" title={LABELS[k]}>
                {LABELS[k]}
              </span>
              <Num value={t[k]?.amber ?? 0} base={bank.ewi.thresholds[k]?.amber} min={-1000} max={1000} step={0.5} decimals={1} width={68} onChange={(v) => edit((p) => void (p.ewi.thresholds[k] = { ...(p.ewi.thresholds[k] ?? { amber: 0, red: 0 }), amber: v }))} />
              <Num value={t[k]?.red ?? 0} base={bank.ewi.thresholds[k]?.red} min={-1000} max={1000} step={0.5} decimals={1} width={68} onChange={(v) => edit((p) => void (p.ewi.thresholds[k] = { ...(p.ewi.thresholds[k] ?? { amber: 0, red: 0 }), red: v }))} />
            </div>
          ))}
        </div>
      </Group>
    </>
  );
}

function fmtValue(i: EwiIndicator): string {
  if (i.value === null) return '—';
  if (i.key === 'survival') return i.value > 90 ? 'beyond 90 days' : `${i.value.toFixed(0)} days`;
  const sign = i.unit !== '%' || i.key.endsWith('_5d') ? (i.value > 0 ? '+' : '') : '';
  return `${sign}${i.value.toFixed(i.unit === 'bp' ? 0 : 1)} ${i.unit}`;
}

function Indicator({ i }: { i: EwiIndicator }) {
  const points = i.series.map((p, idx) => [idx, p.value] as const).filter((p): p is readonly [number, number] => p[1] !== null).map(([x, y]) => [x, y] as [number, number]);
  const word = i.rising_is_worse ? '≥' : '≤';
  return (
    <div className="ewi-row" data-testid={`ewi-${i.key}`}>
      <div>
        <div className="row" style={{ gap: 8 }}>
          <Dot tone={ewiTone(i.status)} />
          <span className="t-medium">{i.label}</span>
          <Badge tone={ewiTone(i.status)}>{i.status.toLowerCase()}</Badge>
        </div>
        <div className="t-small prose">{i.what}</div>
      </div>
      <div className="ewi-value">
        <div className="t-heading">{fmtValue(i)}</div>
        <div className="t-small">
          amber {word} {i.amber} · red {word} {i.red}
        </div>
      </div>
      <div className="ewi-spark">{points.length > 1 ? <LineChart series={[{ name: i.label, points }]} height={56} yFmt={(y) => y.toFixed(1)} xFmt={(x) => fmtDate(i.series[Math.round(x)]?.as_of ?? '').slice(4)} /> : <span className="t-small">today only</span>}</div>
    </div>
  );
}

export function Ewi() {
  const q = useTool<EwiResult>('ewi');
  const r = q.data;
  const ambers = r?.indicators.filter((i) => i.status === 'AMBER').length ?? 0;
  const reds = r?.indicators.filter((i) => i.status === 'RED').length ?? 0;
  return (
    <ToolLayout assumptions={<Assumptions />} ready={!!r} busy={q.isFetching && !!r}>
      <ResultsHeader title="Early warning and contingency funding" loading={q.isFetching} error={q.error?.message} onRetry={() => void q.refetch()} />
      {r && (
        <>
          <div style={{ marginBottom: 10 }} data-testid="headline">
            <div className="row" style={{ alignItems: 'center', gap: 10 }}>
              <span className="t-hero">{r.stage.charAt(0) + r.stage.slice(1).toLowerCase()}</span>
              <Badge tone={STAGE_TONE[r.stage]} dot>
                contingency plan stage
              </Badge>
            </div>
            <div className="t-small">
              {reds} red and {ambers} amber of {r.indicators.length} indicators · {fmtDateLong(r.as_of)} · {r.params_version !== null ? `bank parameters v${r.params_version}` : 'unsaved what-if'}
            </div>
          </div>
          <div className="panel" style={{ marginBottom: 12 }}>
            <div className="t-heading">What the plan asks at this stage</div>
            <ul className="actions">
              {r.actions.map((a, n) => (
                <li key={n}>{a}</li>
              ))}
            </ul>
          </div>
          <div className="panel ewi-list">
            {r.indicators.map((i) => (
              <Indicator key={i.key} i={i} />
            ))}
          </div>
        </>
      )}
    </ToolLayout>
  );
}
