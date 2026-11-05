// The PMU desk's tickets, in a sheet over the right of the page: buy or sell a bond picked by ISIN
// (adding it when the system does not know it), a repo or reverse repo, and closing or rolling a
// repo. Every figure comes from the server's preview, which is what the booking is checked against.
import { useDeferredValue, useEffect, useMemo, useState, type ReactNode } from 'react';
import { useSearchParams } from 'react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ApiError, endpoints } from '../../api/client';
import { usePmuFunding, usePmuOverview } from '../../api/queries';
import type { BondSearchHit, BondTerms, LotPick, RepoRequest, RepoView, TicketRequest } from '../../api/types';
import { fmtDateShort, fmtMoneyShort } from '../../lib/fmt';
import { Badge, Chip } from '../../components/ui';
import { useToast } from '../../components/feedback';

const n = (s: string) => {
  const v = Number(s.replace(/,/g, '').trim());
  return Number.isFinite(v) ? v : NaN;
};
/** "10m" or "2.5m" or "500k" as a face amount. */
const amount = (s: string) => {
  const t = s.trim().toLowerCase().replace(/,/g, '');
  const m = /^([0-9.]+)\s*(m|k|bn)?$/.exec(t);
  if (!m) return NaN;
  const v = Number(m[1]);
  return m[2] === 'bn' ? v * 1e9 : m[2] === 'm' ? v * 1e6 : m[2] === 'k' ? v * 1e3 : v;
};
const pct = (v: number | null | undefined, dp = 3) => (v === null || v === undefined || !Number.isFinite(v) ? '—' : `${v.toFixed(dp)}%`);
const money = (v: number, ccy: string) => `${fmtMoneyShort(v)} ${ccy}`;
const today = () => new Date().toISOString().slice(0, 10);
const addDays = (d: string, k: number) => new Date(Date.parse(`${d}T00:00:00Z`) + k * 86_400_000).toISOString().slice(0, 10);

function errorText(e: unknown): string {
  return e instanceof ApiError ? e.message : 'the request failed';
}

function Issues({ blocking, warnings }: { blocking: string[]; warnings: string[] }) {
  if (!blocking.length && !warnings.length) return null;
  return (
    <>
      {blocking.length > 0 && (
        <ul className="issues bad" data-testid="ticket-blocking">
          {blocking.map((b) => (
            <li key={b}>{b}</li>
          ))}
        </ul>
      )}
      {warnings.length > 0 && (
        <ul className="issues warn" data-testid="ticket-warnings">
          {warnings.map((w) => (
            <li key={w}>{w}</li>
          ))}
        </ul>
      )}
    </>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <span className="k">{label}</span>
      <div>{children}</div>
    </>
  );
}

/** The sheet the PMU strip opens: `?book=buy|sell|repo` (and `isin=`), or `?repo=<id>` for a repo's actions. */
export function BookSheet() {
  const [params, setParams] = useSearchParams();
  const mode = params.get('book');
  const repoId = params.get('repo');
  const close = () => {
    const next = new URLSearchParams(params);
    next.delete('book');
    next.delete('isin');
    next.delete('repo');
    setParams(next, { replace: true });
  };
  const setMode = (m: string) => {
    const next = new URLSearchParams(params);
    next.set('book', m);
    next.delete('repo');
    setParams(next, { replace: true });
  };
  if (!mode && !repoId) return null;
  return (
    <aside className="detail-pane sheet" data-testid="pmu-ticket" aria-label="PMU ticket">
      <div className="row" style={{ gap: 6, marginBottom: 8 }}>
        {repoId ? (
          <span className="t-heading">Repo</span>
        ) : (
          <>
            <Chip small selected={mode === 'buy'} onClick={() => setMode('buy')}>
              Buy bond
            </Chip>
            <Chip small selected={mode === 'sell'} onClick={() => setMode('sell')}>
              Sell bond
            </Chip>
            <Chip small selected={mode === 'repo'} onClick={() => setMode('repo')}>
              Repo
            </Chip>
          </>
        )}
        <span className="grow" />
        <button type="button" className="btn btn-ghost btn-sm" onClick={close} aria-label="close the ticket">
          ✕
        </button>
      </div>
      {repoId ? <RepoActions id={repoId} onDone={close} /> : mode === 'repo' ? <RepoTicket onDone={close} /> : <BondTicket key={mode} side={mode === 'sell' ? 'SELL' : 'BUY'} initialIsin={params.get('isin') ?? ''} onDone={close} />}
    </aside>
  );
}

// ---------------------------------------------------------------------------------------------
// Bonds
// ---------------------------------------------------------------------------------------------

function BondTicket({ side, initialIsin, onDone }: { side: 'BUY' | 'SELL'; initialIsin: string; onDone: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const ov = usePmuOverview();
  const [query, setQuery] = useState(initialIsin);
  const [bond, setBond] = useState<BondSearchHit | null>(null);
  const [adding, setAdding] = useState(false);
  const [book, setBook] = useState('');
  const [face, setFace] = useState('');
  const [mode, setMode] = useState<'price' | 'yield'>('price');
  const [price, setPrice] = useState('');
  const [yieldPct, setYield] = useState('');
  const [trade, setTrade] = useState(today());
  const [settle, setSettle] = useState('');
  const [kondor, setKondor] = useState('');
  const [confirm, setConfirm] = useState(false);
  const [manual, setManual] = useState(false);
  const [picks, setPicks] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);

  const q = useDeferredValue(query.trim());
  const search = useQuery({ queryKey: ['pmu', 'bonds', q], queryFn: () => endpoints.pmuBonds(q), enabled: !bond && q.length >= 2, staleTime: 30_000 });
  // Opened on an ISIN (from a bond's detail): pick it as soon as the search finds it.
  useEffect(() => {
    if (!bond && initialIsin && search.data) {
      const hit = search.data.find((h) => h.bond.isin === initialIsin.toUpperCase());
      if (hit) choose(hit);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search.data]);

  const choose = (h: BondSearchHit) => {
    setBond(h);
    setAdding(false);
    setQuery(h.bond.isin);
    const held = h.holdings[0]?.[0];
    const byCcy = ov.data?.by_book.find((b) => (h.bond.ccy === 'BHD' ? b.book.includes('BHD') : b.book.includes('USD')))?.book;
    setBook(side === 'SELL' ? '' : (held ?? byCcy ?? ov.data?.by_book[0]?.book ?? ''));
    if (h.market_price !== null) setPrice(h.market_price.toFixed(3));
    if (h.market_ytm !== null) setYield(h.market_ytm.toFixed(3));
  };

  const faceV = amount(face);
  const pickList: LotPick[] | null = manual
    ? Object.entries(picks)
        .map(([lot_id, f]) => ({ lot_id, face: amount(f) }))
        .filter((p) => Number.isFinite(p.face) && p.face > 0)
    : null;
  const req: TicketRequest | null =
    bond && Number.isFinite(faceV) && faceV > 0
      ? {
          side,
          isin: bond.bond.isin,
          book: book || null,
          face: faceV,
          clean_price: mode === 'price' && Number.isFinite(n(price)) && n(price) > 0 ? n(price) : null,
          ytm: mode === 'yield' && Number.isFinite(n(yieldPct)) ? n(yieldPct) : null,
          trade_date: trade || null,
          settle_date: settle || null,
          lots: pickList,
          kondor_id: kondor.trim() || null,
          confirm_off_market: confirm,
        }
      : null;
  const reqKey = useDeferredValue(JSON.stringify(req));
  const pv = useQuery({
    queryKey: ['pmu', 'ticket', reqKey],
    queryFn: () => endpoints.pmuTicketPreview(JSON.parse(reqKey) as TicketRequest),
    enabled: !!req && reqKey !== 'null',
    placeholderData: (p) => p,
    retry: false,
  });
  const p = pv.data;
  const quote = p?.quote;
  // Choosing lots starts from the oldest-first allocation.
  useEffect(() => {
    if (manual && p && Object.keys(picks).length === 0) setPicks(Object.fromEntries(p.legs.map((l) => [l.lot_id, fmtMoneyShort(l.face)])));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [manual]);

  const book_ = useMutation({
    mutationFn: () => (side === 'BUY' ? endpoints.pmuBuy(req!) : endpoints.pmuSell(req!)),
    onSuccess: (r) => {
      void qc.invalidateQueries({ queryKey: ['pmu'] });
      toast({ tone: 'good', title: r.message, detail: kondor.trim() ? 'Booked with its Kondor id.' : 'Booked as provisional until its Kondor id is attached.' });
      onDone();
    },
    onError: (e) => setError(errorText(e)),
  });
  const blocked = !req || !p || p.blocking.length > 0 || pv.isError || (quote?.off_market && !confirm) || (side === 'BUY' && !book);
  const ccy = bond?.bond.ccy ?? '';

  return (
    <div data-testid={`ticket-${side.toLowerCase()}`}>
      <label className="field">
        <span className="t-label">Bond: ISIN, name or issuer</span>
        <input
          className="input"
          placeholder="e.g. XSSYN0000166 or Saudi"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setBond(null);
            setAdding(false);
          }}
          aria-label="Bond search"
          autoFocus={!initialIsin}
        />
      </label>
      {!bond && !adding && q.length >= 2 && (
        <div className="hits" data-testid="bond-hits">
          {(search.data ?? [])
            .filter((h) => side === 'BUY' || h.holdings.length > 0)
            .slice(0, 8)
            .map((h) => (
              <button key={h.bond.isin} type="button" className="hit" onClick={() => choose(h)}>
                <span className="t-medium">{h.bond.name}</span>
                <span className="t-small">
                  {h.bond.isin} · {h.bond.ccy} · matures {fmtDateShort(h.bond.maturity)}
                  {h.holdings.length ? ` · held ${h.holdings.map(([b, held]) => `${fmtMoneyShort(held)} ${b}`).join(', ')}` : ''}
                </span>
              </button>
            ))}
          {search.data && search.data.length === 0 && <div className="t-small">No bond matches.</div>}
          {side === 'BUY' && search.data && !search.data.some((h) => h.bond.isin === q.toUpperCase()) && /^[A-Za-z]{2}[A-Za-z0-9]{9}[0-9]$/.test(q) && (
            <button type="button" className="btn btn-sm" onClick={() => setAdding(true)}>
              Add {q.toUpperCase()} as a new bond
            </button>
          )}
        </div>
      )}
      {adding && (
        <NewBondForm
          isin={q.toUpperCase()}
          onAdded={async (isin) => {
            const hits = await endpoints.pmuBonds(isin);
            const hit = hits.find((h) => h.bond.isin === isin);
            if (hit) choose(hit);
          }}
        />
      )}

      {bond && (
        <>
          <div className="bond-card">
            <div className="t-medium">{bond.bond.name}</div>
            <div className="t-small">
              {bond.bond.isin} · {bond.bond.issuer} · {bond.bond.rate_type === 'FLOAT' ? `${bond.bond.float_index} + ${bond.bond.float_spread}%` : `${bond.bond.coupon}%`}, {bond.bond.coupon_freq}× a year,{' '}
              {bond.bond.day_count} · matures {fmtDateShort(bond.bond.maturity)}
            </div>
            <div className="t-small">
              market {bond.market_price !== null ? bond.market_price.toFixed(3) : '—'} / {pct(bond.market_ytm)}
              {bond.holdings.length > 0 && ` · held ${bond.holdings.map(([b, h, pl, fr]) => `${b} ${fmtMoneyShort(h)} (${fmtMoneyShort(pl)} pledged, ${fmtMoneyShort(fr)} free)`).join('; ')}`}
            </div>
          </div>
          <div className="form-grid">
            <Field label={side === 'BUY' ? 'Into book' : 'From book'}>
              <select className="input input-sm" value={book} onChange={(e) => setBook(e.target.value)} aria-label="Book">
                {side === 'SELL' && <option value="">any book</option>}
                {(ov.data?.by_book ?? []).map((b) => (
                  <option key={b.book} value={b.book}>
                    {b.book}
                  </option>
                ))}
              </select>
            </Field>
            <Field label={`Face, ${ccy}`}>
              <input className="input input-sm" value={face} onChange={(e) => setFace(e.target.value)} placeholder="e.g. 10m" aria-label="Face" />
            </Field>
            <Field label="Price">
              <div className="row" style={{ gap: 4 }}>
                <Chip small selected={mode === 'price'} onClick={() => setMode('price')}>
                  Clean price
                </Chip>
                <Chip small selected={mode === 'yield'} onClick={() => setMode('yield')} disabled={bond.bond.rate_type === 'FLOAT'}>
                  Yield
                </Chip>
                {mode === 'price' ? (
                  <input className="input input-sm" style={{ width: 100 }} value={price} onChange={(e) => setPrice(e.target.value)} aria-label="Clean price" />
                ) : (
                  <input className="input input-sm" style={{ width: 100 }} value={yieldPct} onChange={(e) => setYield(e.target.value)} aria-label="Yield" />
                )}
              </div>
            </Field>
            <Field label="Trade date">
              <input className="input input-sm" type="date" value={trade} onChange={(e) => setTrade(e.target.value)} aria-label="Trade date" />
            </Field>
            <Field label="Settles">
              <input className="input input-sm" type="date" value={settle || quote?.settle_date || ''} onChange={(e) => setSettle(e.target.value)} aria-label="Settlement date" />
            </Field>
            <Field label="Kondor id">
              <input className="input input-sm" value={kondor} onChange={(e) => setKondor(e.target.value)} placeholder="optional; provisional without it" aria-label="Kondor id" />
            </Field>
          </div>
        </>
      )}

      {bond && quote && (
        <div className={pv.isFetching ? 'dimmed' : ''} data-testid="ticket-preview">
          <div className="kv quote">
            <span>Clean · dirty</span>
            <span>
              {quote.clean_price.toFixed(4)} · {quote.dirty_price.toFixed(4)}
            </span>
            <span>Yield</span>
            <span>{pct(quote.ytm)}</span>
            <span>Accrued</span>
            <span>{money(quote.accrued, ccy)}</span>
            <span>Consideration</span>
            <span className="t-medium">
              {money(quote.consideration, ccy)} · {fmtMoneyShort(quote.consideration_bhd)} BHD
            </span>
            {side === 'BUY' && (
              <>
                <span>Effective rate</span>
                <span>{pct(quote.eir)}</span>
                <span>FTP locks at</span>
                <span>{quote.ftp_rate !== null ? `${pct(quote.ftp_rate)} (curve of ${quote.ftp_published ? fmtDateShort(quote.ftp_published) : '—'})` : 'no curve'}</span>
                <span>Carry after FTP</span>
                <span className={quote.carry_bp !== null && quote.carry_bp < 0 ? 't-bad' : ''}>
                  {quote.carry_bp !== null ? `${quote.carry_bp.toFixed(0)} bp · ${fmtMoneyShort(quote.carry_bhd_year ?? 0)} BHD a year` : '—'}
                </span>
              </>
            )}
            <span>DV01 · duration</span>
            <span>
              {fmtMoneyShort(quote.dv01_bhd)} BHD per bp · {quote.mod_duration.toFixed(2)}
            </span>
            <span>Market</span>
            <span className={quote.off_market ? 't-warn' : ''}>
              {quote.market_price !== null ? `${quote.market_price.toFixed(3)} / ${pct(quote.market_ytm)} on ${quote.market_as_of ? fmtDateShort(quote.market_as_of) : '—'}` : 'no price on file'}
              {quote.off_market_bp !== null && ` · ${quote.off_market_bp > 0 ? '+' : ''}${quote.off_market_bp.toFixed(0)} bp`}
            </span>
          </div>

          {side === 'SELL' && p && (
            <div data-testid="sale-lots">
              <div className="row" style={{ gap: 6, marginTop: 10 }}>
                <span className="t-label">Lots sold</span>
                <span className="grow" />
                <Chip small selected={!manual} onClick={() => setManual(false)}>
                  Oldest first
                </Chip>
                <Chip small selected={manual} onClick={() => setManual(true)}>
                  Choose lots
                </Chip>
              </div>
              <table className="table compact">
                <thead>
                  <tr>
                    <th>Bought</th>
                    <th>Book</th>
                    <th className="num">Free</th>
                    <th className="num">Sell</th>
                    <th className="num">Realised</th>
                  </tr>
                </thead>
                <tbody>
                  {p.candidates.map((c) => {
                    const leg = p.legs.find((l) => l.lot_id === c.lot_id);
                    return (
                      <tr key={c.lot_id}>
                        <td>
                          {fmtDateShort(c.trade_date)} <span className="t-small">at {c.purchase_price.toFixed(2)}</span>
                        </td>
                        <td className="t-small">{c.book}</td>
                        <td className="num" title={`held ${fmtMoneyShort(c.held)}, pledged ${fmtMoneyShort(c.pledged)}`}>
                          {fmtMoneyShort(c.free)}
                        </td>
                        <td className="num">
                          {manual ? (
                            <input
                              className="input input-sm"
                              style={{ width: 80 }}
                              value={picks[c.lot_id] ?? ''}
                              onChange={(e) => setPicks({ ...picks, [c.lot_id]: e.target.value })}
                              aria-label={`sell from the lot of ${c.trade_date}`}
                            />
                          ) : leg ? (
                            fmtMoneyShort(leg.face)
                          ) : (
                            '—'
                          )}
                        </td>
                        <td className={`num ${leg && leg.realised_bhd < 0 ? 't-bad' : ''}`}>{leg ? fmtMoneyShort(leg.realised_bhd) : ''}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
              <div className="t-small">
                {fmtMoneyShort(p.free_total)} {ccy} free to sell · realised against amortised cost {fmtMoneyShort(p.realised_bhd)} BHD
              </div>
            </div>
          )}
          <Issues blocking={p?.blocking ?? []} warnings={p?.warnings ?? []} />
          {quote.off_market && (
            <label className="row" style={{ gap: 6, marginTop: 6 }}>
              <input type="checkbox" checked={confirm} onChange={(e) => setConfirm(e.target.checked)} aria-label="Confirm off-market price" />
              <span className="t-small">The price is away from the market; book it anyway.</span>
            </label>
          )}
        </div>
      )}
      {pv.isError && <div className="notice bad">{errorText(pv.error)}</div>}
      {error && (
        <div className="notice bad" role="alert">
          {error}
        </div>
      )}
      {bond && (
        <div className="row" style={{ gap: 8, marginTop: 12 }}>
          <button type="button" className="btn btn-primary" disabled={!!blocked || book_.isPending} onClick={() => book_.mutate()}>
            {book_.isPending ? 'Booking…' : side === 'BUY' ? 'Book the purchase' : 'Book the sale'}
          </button>
          <span className="t-small">It counts at once{kondor.trim() ? '' : ', provisional until its Kondor id is attached'}.</span>
        </div>
      )}
    </div>
  );
}

const ISSUER_TYPES = ['SOVEREIGN', 'GCC_SOVEREIGN', 'SUPRANATIONAL', 'BANK', 'CORPORATE', 'SUKUK'];

function NewBondForm({ isin, onAdded }: { isin: string; onAdded: (isin: string) => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const [b, setB] = useState<BondTerms>({
    isin,
    name: '',
    issuer: '',
    issuer_type: 'SOVEREIGN',
    ccy: 'USD',
    coupon: 5,
    coupon_freq: 2,
    day_count: '30/360',
    issue_date: today(),
    maturity: addDays(today(), 5 * 365),
    rate_type: 'FIXED',
    float_index: null,
    float_spread: 0,
    hqla_level: null,
  });
  const [error, setError] = useState<string | null>(null);
  const add = useMutation({
    mutationFn: () => endpoints.pmuAddBond({ ...b, isin }),
    onSuccess: (r) => {
      void qc.invalidateQueries({ queryKey: ['pmu'] });
      toast({ tone: 'good', title: r.message, detail: 'Every PMU user can now trade it.' });
      onAdded(isin);
    },
    onError: (e) => setError(errorText(e)),
  });
  const set = (k: keyof BondTerms, v: string | number | null) => setB({ ...b, [k]: v });
  return (
    <div className="save-box" data-testid="new-bond" style={{ marginTop: 8 }}>
      <div className="t-heading">New bond {isin}</div>
      <div className="form-grid" style={{ marginTop: 6 }}>
        <Field label="Name">
          <input className="input input-sm" value={b.name} onChange={(e) => set('name', e.target.value)} placeholder="e.g. Kuwait sovereign 4.75% 2031" aria-label="Bond name" />
        </Field>
        <Field label="Issuer">
          <input className="input input-sm" value={b.issuer} onChange={(e) => set('issuer', e.target.value)} aria-label="Issuer" />
        </Field>
        <Field label="Issuer type">
          <select className="input input-sm" value={b.issuer_type} onChange={(e) => set('issuer_type', e.target.value)} aria-label="Issuer type">
            {ISSUER_TYPES.map((t) => (
              <option key={t} value={t}>
                {t.toLowerCase().replace('_', ' ')}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Currency">
          <input className="input input-sm" style={{ width: 70 }} value={b.ccy} onChange={(e) => set('ccy', e.target.value.toUpperCase())} aria-label="Bond currency" />
        </Field>
        <Field label="Coupon">
          <div className="row" style={{ gap: 4 }}>
            <Chip small selected={b.rate_type === 'FIXED'} onClick={() => setB({ ...b, rate_type: 'FIXED', float_index: null })}>
              Fixed
            </Chip>
            <Chip small selected={b.rate_type === 'FLOAT'} onClick={() => setB({ ...b, rate_type: 'FLOAT', float_index: 'SOFR', coupon_freq: 4, day_count: 'ACT/360' })}>
              Floating
            </Chip>
            {b.rate_type === 'FIXED' ? (
              <input className="input input-sm" style={{ width: 70 }} value={b.coupon} onChange={(e) => set('coupon', Number(e.target.value))} aria-label="Coupon" />
            ) : (
              <input className="input input-sm" style={{ width: 70 }} value={b.float_spread} onChange={(e) => set('float_spread', Number(e.target.value))} aria-label="Spread over the index" />
            )}
            <span className="t-small">%</span>
          </div>
        </Field>
        <Field label="Paid">
          <select className="input input-sm" value={b.coupon_freq} onChange={(e) => set('coupon_freq', Number(e.target.value))} aria-label="Coupons a year">
            {[1, 2, 4, 12].map((f) => (
              <option key={f} value={f}>
                {f}× a year
              </option>
            ))}
          </select>{' '}
          <select className="input input-sm" value={b.day_count} onChange={(e) => set('day_count', e.target.value)} aria-label="Day count">
            {['30/360', 'ACT/ACT', 'ACT/360', 'ACT/365'].map((d) => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Issued, matures">
          <div className="row" style={{ gap: 4 }}>
            <input className="input input-sm" type="date" value={b.issue_date} onChange={(e) => set('issue_date', e.target.value)} aria-label="Issue date" />
            <input className="input input-sm" type="date" value={b.maturity} onChange={(e) => set('maturity', e.target.value)} aria-label="Maturity" />
          </div>
        </Field>
        <Field label="HQLA">
          <select className="input input-sm" value={b.hqla_level ?? ''} onChange={(e) => set('hqla_level', e.target.value || null)} aria-label="HQLA level">
            <option value="">not HQLA</option>
            {['L1', 'L2A', 'L2B'].map((h) => (
              <option key={h} value={h}>
                {h}
              </option>
            ))}
          </select>
        </Field>
      </div>
      {error && <div className="notice bad">{error}</div>}
      <button type="button" className="btn btn-sm btn-primary" style={{ marginTop: 8 }} disabled={!b.name.trim() || !b.issuer.trim() || add.isPending} onClick={() => add.mutate()}>
        {add.isPending ? 'Setting up…' : 'Set up the bond'}
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// Repos
// ---------------------------------------------------------------------------------------------

const TENORS: [string, number][] = [
  ['1W', 7],
  ['2W', 14],
  ['1M', 30],
  ['3M', 91],
  ['6M', 182],
];

function RepoTicket({ onDone }: { onDone: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const funding = usePmuFunding();
  const [direction, setDirection] = useState<'REPO' | 'REVERSE'>('REPO');
  const [cpty, setCpty] = useState('');
  const [ccy, setCcy] = useState('USD');
  const [start, setStart] = useState(today());
  const [end, setEnd] = useState(addDays(today(), 30));
  const [rate, setRate] = useState('');
  const [haircut, setHaircut] = useState('5');
  const [cash, setCash] = useState('');
  const [picks, setPicks] = useState<Record<string, string>>({});
  const [cIsin, setCIsin] = useState('');
  const [cFace, setCFace] = useState('');
  const [cNote, setCNote] = useState('');
  const [kondor, setKondor] = useState('');
  const [error, setError] = useState<string | null>(null);
  const counterparties = useMemo(() => [...new Set([...(funding.data?.quotes ?? []).map((q) => q.counterparty), ...(funding.data?.repos ?? []).map((r) => r.counterparty)])].sort(), [funding.data]);

  const collateral: LotPick[] = Object.entries(picks)
    .map(([lot_id, f]) => ({ lot_id, face: amount(f) }))
    .filter((p) => Number.isFinite(p.face) && p.face > 0);
  const rateV = n(rate);
  const req: RepoRequest = {
    direction,
    counterparty: cpty,
    ccy,
    start,
    end,
    rate: Number.isFinite(rateV) ? rateV : 0,
    haircut_pct: Number.isFinite(n(haircut)) ? n(haircut) : 0,
    cash: cash.trim() ? amount(cash) : null,
    collateral: direction === 'REPO' ? collateral : [],
    collateral_isin: direction === 'REVERSE' ? cIsin.trim().toUpperCase() || null : null,
    collateral_face: direction === 'REVERSE' && cFace.trim() ? amount(cFace) : null,
    collateral_note: direction === 'REVERSE' ? cNote.trim() || null : null,
    kondor_id: kondor.trim() || null,
  };
  const key = useDeferredValue(JSON.stringify(req));
  const pv = useQuery({ queryKey: ['pmu', 'repo-preview', key], queryFn: () => endpoints.pmuRepoPreview(JSON.parse(key) as RepoRequest), placeholderData: (p) => p, retry: false });
  const p = pv.data;
  // The rate starts at the counterparty's latest quote over SOFR until the dealer types one.
  const [rateTouched, setRateTouched] = useState(false);
  useEffect(() => {
    if (!rateTouched && p?.indicative_rate !== null && p?.indicative_rate !== undefined) setRate(p.indicative_rate.toFixed(3));
  }, [p?.indicative_rate, rateTouched]);

  const book = useMutation({
    mutationFn: () => endpoints.pmuRepo(req),
    onSuccess: (r) => {
      void qc.invalidateQueries({ queryKey: ['pmu'] });
      toast({ tone: 'good', title: r.message, detail: kondor.trim() ? 'Booked with its Kondor id.' : 'Booked as provisional until its Kondor id is attached.' });
      onDone();
    },
    onError: (e) => setError(errorText(e)),
  });
  const blocked = !p || p.blocking.length > 0 || !cpty.trim() || !Number.isFinite(rateV) || (direction === 'REVERSE' && (!cIsin.trim() || !cFace.trim() || !cash.trim()));
  const days = Math.round((Date.parse(end) - Date.parse(start)) / 86_400_000);
  return (
    <div data-testid="ticket-repo">
      <div className="row" style={{ gap: 4, marginBottom: 8 }}>
        <Chip small selected={direction === 'REPO'} onClick={() => setDirection('REPO')}>
          Repo: borrow cash
        </Chip>
        <Chip small selected={direction === 'REVERSE'} onClick={() => setDirection('REVERSE')}>
          Reverse repo: lend cash
        </Chip>
      </div>
      <div className="form-grid">
        <Field label="Counterparty">
          <input className="input input-sm" list="pmu-cptys" value={cpty} onChange={(e) => setCpty(e.target.value)} aria-label="Counterparty" />
          <datalist id="pmu-cptys">
            {counterparties.map((c) => (
              <option key={c} value={c} />
            ))}
          </datalist>
        </Field>
        <Field label="Currency">
          <select className="input input-sm" value={ccy} onChange={(e) => setCcy(e.target.value)} aria-label="Repo currency">
            {['USD', 'BHD', 'SAR', 'AED'].map((c) => (
              <option key={c}>{c}</option>
            ))}
          </select>
        </Field>
        <Field label="Starts">
          <input className="input input-sm" type="date" value={start} onChange={(e) => setStart(e.target.value)} aria-label="Repo start" />
        </Field>
        <Field label="Ends">
          <div className="row" style={{ gap: 3, flexWrap: 'wrap' }}>
            {TENORS.map(([t, d]) => (
              <Chip key={t} small selected={days === d} onClick={() => setEnd(addDays(start, d))}>
                {t}
              </Chip>
            ))}
            <input className="input input-sm" type="date" value={end} onChange={(e) => setEnd(e.target.value)} aria-label="Repo end" />
          </div>
        </Field>
        <Field label="Rate, %">
          <input
            className="input input-sm"
            style={{ width: 90 }}
            value={rate}
            onChange={(e) => {
              setRate(e.target.value);
              setRateTouched(true);
            }}
            aria-label="Repo rate"
          />{' '}
          <span className="t-small">
            {p?.quote_spread_bp !== null && p?.quote_spread_bp !== undefined
              ? `${cpty || 'quote'} ${p.quote_tenor} SOFR + ${p.quote_spread_bp.toFixed(0)} bp (${p.quote_as_of ? fmtDateShort(p.quote_as_of) : ''})`
              : p?.sofr
                ? `SOFR ${pct(p.sofr)}`
                : ''}
          </span>
        </Field>
        {direction === 'REPO' && (
          <Field label="Haircut, %">
            <input className="input input-sm" style={{ width: 70 }} value={haircut} onChange={(e) => setHaircut(e.target.value)} aria-label="Haircut" />
          </Field>
        )}
      </div>

      {direction === 'REPO' ? (
        <div data-testid="repo-collateral">
          <div className="t-label" style={{ marginTop: 10 }}>
            Bonds to pledge (face free over the term)
          </div>
          <div className="pledge-list">
            <table className="table compact">
              <tbody>
                {(p?.available ?? []).slice(0, 40).map((c) => (
                  <tr key={c.lot_id}>
                    <td>
                      <input
                        type="checkbox"
                        checked={c.lot_id in picks}
                        onChange={(e) => {
                          const next = { ...picks };
                          if (e.target.checked) next[c.lot_id] = fmtMoneyShort(c.free);
                          else delete next[c.lot_id];
                          setPicks(next);
                        }}
                        aria-label={`pledge ${c.name} from ${c.book}`}
                      />
                    </td>
                    <td className="ellipsis" style={{ maxWidth: 150 }} title={`${c.name} ${c.isin}`}>
                      {c.name}
                    </td>
                    <td className="t-small">{c.book}</td>
                    <td className="num t-small">{fmtMoneyShort(c.free)} free</td>
                    <td className="num">
                      {c.lot_id in picks && (
                        <input className="input input-sm" style={{ width: 76 }} value={picks[c.lot_id]} onChange={(e) => setPicks({ ...picks, [c.lot_id]: e.target.value })} aria-label={`face pledged of ${c.name}`} />
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      ) : (
        <div className="form-grid" style={{ marginTop: 10 }}>
          <Field label="Bond received">
            <input className="input input-sm" value={cIsin} onChange={(e) => setCIsin(e.target.value)} placeholder="their ISIN" aria-label="Collateral ISIN" />
          </Field>
          <Field label="Its face">
            <input className="input input-sm" value={cFace} onChange={(e) => setCFace(e.target.value)} placeholder="e.g. 10.5m" aria-label="Collateral face" />
          </Field>
          <Field label="Note">
            <input className="input input-sm" value={cNote} onChange={(e) => setCNote(e.target.value)} aria-label="Collateral note" />
          </Field>
        </div>
      )}
      <div className="form-grid" style={{ marginTop: 8 }}>
        <Field label={direction === 'REPO' ? 'Cash raised' : 'Cash lent'}>
          <input className="input input-sm" value={cash} onChange={(e) => setCash(e.target.value)} placeholder={direction === 'REPO' && p ? `${fmtMoneyShort(p.cash)} after the haircut` : 'e.g. 10m'} aria-label="Repo cash" />
        </Field>
        <Field label="Kondor id">
          <input className="input input-sm" value={kondor} onChange={(e) => setKondor(e.target.value)} placeholder="optional" aria-label="Repo Kondor id" />
        </Field>
      </div>
      {p && (
        <div className={`kv quote ${pv.isFetching ? 'dimmed' : ''}`} data-testid="repo-preview">
          {direction === 'REPO' && (
            <>
              <span>Collateral value</span>
              <span>{money(p.collateral_value, ccy)}</span>
            </>
          )}
          <span>Cash</span>
          <span className="t-medium">
            {money(p.cash, ccy)} · {fmtMoneyShort(p.cash_bhd)} BHD
          </span>
          <span>{direction === 'REPO' ? 'Interest paid' : 'Interest received'}</span>
          <span>
            {money(p.interest, ccy)} over {p.days} days
          </span>
        </div>
      )}
      <Issues blocking={p?.blocking ?? []} warnings={p?.warnings ?? []} />
      {error && (
        <div className="notice bad" role="alert">
          {error}
        </div>
      )}
      <div className="row" style={{ gap: 8, marginTop: 12 }}>
        <button type="button" className="btn btn-primary" disabled={blocked || book.isPending} onClick={() => book.mutate()}>
          {book.isPending ? 'Booking…' : direction === 'REPO' ? 'Book the repo' : 'Book the reverse repo'}
        </button>
      </div>
    </div>
  );
}

function RepoActions({ id, onDone }: { id: string; onDone: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const funding = usePmuFunding();
  const r: RepoView | undefined = funding.data?.repos.find((x) => x.id === id);
  const [closeOn, setCloseOn] = useState(today());
  const [rollOn, setRollOn] = useState('');
  const [rollEnd, setRollEnd] = useState('');
  const [rollRate, setRollRate] = useState('');
  const [kondor, setKondor] = useState('');
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (r && !rollOn) {
      // A repo closes the day after it starts at the earliest.
      if (closeOn <= r.start) setCloseOn(addDays(r.start, 1));
      setRollOn(r.end);
      setRollEnd(addDays(r.end, Math.max(7, Math.round((Date.parse(r.end) - Date.parse(r.start)) / 86_400_000))));
      setRollRate(r.rate.toFixed(3));
    }
  }, [r, rollOn, closeOn]);
  const done = (msg: string) => {
    void qc.invalidateQueries({ queryKey: ['pmu'] });
    toast({ tone: 'good', title: msg });
    onDone();
  };
  const close = useMutation({ mutationFn: () => endpoints.pmuRepoClose(id, closeOn), onSuccess: (x) => done(x.message), onError: (e) => setError(errorText(e)) });
  const roll = useMutation({ mutationFn: () => endpoints.pmuRepoRoll(id, { on: rollOn, end: rollEnd, rate: Number(rollRate) }), onSuccess: (x) => done(x.message), onError: (e) => setError(errorText(e)) });
  const attach = useMutation({ mutationFn: () => endpoints.pmuKondor('repo', id, kondor.trim()), onSuccess: (x) => done(x.message), onError: (e) => setError(errorText(e)) });
  if (!r) return <div className="t-muted">{funding.isLoading ? 'loading…' : 'That repo is no longer open.'}</div>;
  return (
    <div data-testid="repo-actions">
      <div className="bond-card">
        <div className="t-medium">
          {r.direction === 'REVERSE' ? 'Reverse repo' : 'Repo'} with {r.counterparty} {r.provisional && <Badge tone="warn">provisional</Badge>}
        </div>
        <div className="t-small">
          {fmtMoneyShort(r.cash_bhd)} BHD at {pct(r.rate)} · {fmtDateShort(r.start)} to {fmtDateShort(r.end)} ({r.days_left} days left) · {r.isins.join(', ')}
        </div>
      </div>
      {r.closed_early ? (
        <div className="t-small" style={{ marginTop: 10 }}>
          Closed early: it ends on {fmtDateShort(r.end)}.
        </div>
      ) : (
        <>
          <div className="t-label" style={{ marginTop: 10 }}>
            Close early
          </div>
          <div className="row" style={{ gap: 6 }}>
            <input className="input input-sm" type="date" value={closeOn} min={addDays(r.start, 1)} max={addDays(r.end, -1)} onChange={(e) => setCloseOn(e.target.value)} aria-label="Close on" />
            <button type="button" className="btn btn-sm" disabled={close.isPending} onClick={() => close.mutate()}>
              Close the repo
            </button>
          </div>
        </>
      )}
      <div className="t-label" style={{ marginTop: 12 }}>
        Roll into a new repo
      </div>
      <div className="form-grid">
        <Field label="On">
          <input className="input input-sm" type="date" value={rollOn} onChange={(e) => setRollOn(e.target.value)} aria-label="Roll on" />
        </Field>
        <Field label="New end">
          <input className="input input-sm" type="date" value={rollEnd} onChange={(e) => setRollEnd(e.target.value)} aria-label="New end" />
        </Field>
        <Field label="New rate, %">
          <input className="input input-sm" style={{ width: 90 }} value={rollRate} onChange={(e) => setRollRate(e.target.value)} aria-label="New rate" />
        </Field>
      </div>
      <button type="button" className="btn btn-sm" style={{ marginTop: 6 }} disabled={roll.isPending || !rollOn || !rollEnd} onClick={() => roll.mutate()}>
        Roll it
      </button>
      {r.provisional && (
        <>
          <div className="t-label" style={{ marginTop: 12 }}>
            Kondor id
          </div>
          <div className="row" style={{ gap: 6 }}>
            <input className="input input-sm" value={kondor} onChange={(e) => setKondor(e.target.value)} aria-label="Kondor id for the repo" />
            <button type="button" className="btn btn-sm" disabled={!kondor.trim() || attach.isPending} onClick={() => attach.mutate()}>
              Attach
            </button>
          </div>
        </>
      )}
      {error && (
        <div className="notice bad" role="alert" style={{ marginTop: 8 }}>
          {error}
        </div>
      )}
    </div>
  );
}
