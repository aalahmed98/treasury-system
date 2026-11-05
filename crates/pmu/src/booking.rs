//! What the PMU tickets show before anything is booked: a bond's price and yield for a face and
//! settlement date, the effective rate a purchase earns, the FTP rate it would lock and the carry
//! that leaves, its DV01 and distance from the market; for a sale, the lots it draws on (oldest
//! first unless chosen) with the gain realised against amortised cost; for a repo, the cash the
//! collateral raises and the counterparty's latest quote.

use chrono::{Duration, NaiveDate};
use domain::{Bond, BondQuote, BondRateType, LotPick, PledgeCandidate, Repo, RepoPreview, SaleLeg, SaleView};
use uuid::Uuid;

use crate::bond::{accrued, clean_from_yield, dirty_from_yield, mod_duration, period_of, schedule, yield_from_clean};
use crate::eval::{Portfolio, lot_eir};
use crate::ledger::{mm_basis, walk_lot};
use crate::market::Market;

/// A trade further than this from the market (yield bp for a fixed bond, price bp for a floater)
/// needs the dealer's confirmation.
pub const OFF_MARKET_BP: f64 = 15.0;

pub enum Price {
    Clean(f64),
    Yield(f64),
}

/// The bond priced for `face` settling on `settle`, at a clean price or a yield.
pub fn quote_bond(m: &Market, b: &Bond, face: f64, trade: NaiveDate, settle: NaiveDate, price: Price) -> BondQuote {
    let sched = schedule(b);
    let float = b.rate_type == BondRateType::Float;
    let coupon_now = match (float, period_of(b.issue_date, &sched, settle)) {
        (true, Some((prev, next))) => m.fixing(&b.ccy, prev, (next - prev).num_days()) + b.float_spread,
        (true, None) => b.float_spread,
        (false, _) => b.coupon,
    };
    let (clean, ytm) = match (price, float) {
        (Price::Clean(c), false) => (c, yield_from_clean(b, &sched, settle, c)),
        (Price::Yield(y), false) => (clean_from_yield(b, &sched, settle, y), y),
        (Price::Clean(c), true) => (c, coupon_now),
        (Price::Yield(_), true) => (100.0, coupon_now),
    };
    let acc100 = accrued(b, &sched, settle, coupon_now);
    let spot = m.spot(&b.ccy, trade);
    let consideration = (clean + acc100) * face / 100.0;
    let eir = lot_eir(b, settle, clean, m);
    let term = (b.maturity - settle).num_days();
    let ftp = m.ftp.rate(&b.ccy, trade, term);
    let ftp_published = ftp.and_then(|(_, id)| m.ftp.by_date.iter().find(|(_, (pid, _))| *pid == id).map(|(d, _)| *d));
    let (dv01, md) = if float {
        let next = period_of(b.issue_date, &sched, settle).map(|(_, n)| n).unwrap_or(b.maturity);
        let tau = (next - settle).num_days().max(0) as f64 / 365.0;
        (-face * tau / 10_000.0 * spot, tau)
    } else {
        ((dirty_from_yield(b, &sched, settle, ytm + 0.01) - dirty_from_yield(b, &sched, settle, ytm)) * face / 100.0 * spot, mod_duration(b, &sched, settle, ytm))
    };
    let market = m.price(&b.isin, trade).map(|(c, y)| (c, y, m.prices.get(&b.isin).and_then(|p| p.range(..=trade).next_back().map(|(d, _)| *d))));
    let off = market.map(|(mc, my, _)| if float { (clean - mc) * 100.0 } else { (ytm - my) * 100.0 });
    BondQuote {
        isin: b.isin.clone(),
        ccy: b.ccy.clone(),
        trade_date: trade,
        settle_date: settle,
        face,
        clean_price: clean,
        dirty_price: clean + acc100,
        ytm,
        accrued: acc100 * face / 100.0,
        consideration,
        consideration_bhd: consideration * spot,
        spot,
        eir,
        ftp_rate: ftp.map(|x| x.0),
        ftp_published,
        carry_bp: ftp.map(|(r, _)| (eir - r) * 100.0),
        carry_bhd_year: ftp.map(|(r, _)| clean * face / 100.0 * spot * (eir - r) / 100.0),
        dv01_bhd: dv01,
        mod_duration: md,
        market_price: market.map(|x| x.0),
        market_ytm: market.map(|x| x.1),
        market_as_of: market.and_then(|x| x.2),
        off_market_bp: off,
        off_market: off.is_some_and(|o| o.abs() > OFF_MARKET_BP),
    }
}

/// Face of a lot pledged to repos open at any point of `[from, to)`, other than `except`.
pub fn pledged_between(p: &Portfolio, lot: Uuid, from: NaiveDate, to: NaiveDate, except: Option<Uuid>) -> f64 {
    p.repos
        .iter()
        .filter(|r| !r.reverse() && Some(r.id) != except && r.start < to && r.end_on() > from)
        .flat_map(|r| r.collateral.iter().filter(|(l, _)| *l == lot).map(|(_, f)| *f))
        .sum()
}

/// Every lot of the bond (in `book` when given) that could be sold on `settle`, oldest first, with
/// its book value on that date.
pub fn sale_candidates(m: &Market, p: &Portfolio, isin: &str, book: Option<&str>, settle: NaiveDate) -> Vec<SaleLeg> {
    let Some(b) = p.bonds.get(isin) else { return Vec::new() };
    let mut lots: Vec<_> = p.lots.iter().filter(|l| l.isin == isin && book.is_none_or(|bk| l.book == bk) && l.settle_date < settle && l.face_on(settle) > 0.005).collect();
    lots.sort_by_key(|l| (l.trade_date, l.id));
    lots.into_iter()
        .map(|l| {
            let held = l.face_on(settle);
            // Promised to a sale settling later already.
            let later: f64 = l.sales.iter().filter(|s| s.settle_date > settle).map(|s| s.face).sum();
            let pledged = pledged_between(p, l.id, settle, settle + Duration::days(1), None);
            let st = walk_lot(m, l, b, settle, settle, |_| {});
            SaleLeg {
                lot_id: l.id,
                book: l.book.clone(),
                trade_date: l.trade_date,
                purchase_price: l.clean_price,
                held,
                pledged,
                free: (held - later - pledged).max(0.0),
                face: 0.0,
                book_value: if st.face > 0.0 { st.book_value / st.face } else { 0.0 },
                accrued: if st.face > 0.0 { st.accrued / st.face } else { 0.0 },
                realised: 0.0,
                realised_bhd: 0.0,
                kondor_id: l.kondor_id.clone(),
            }
        })
        .collect()
}

/// A candidate leg (book value and accrued held per unit of face) with `face` taken from it.
fn take(mut leg: SaleLeg, face: f64, clean: f64, spot: f64) -> SaleLeg {
    let per_unit_bv = leg.book_value;
    let per_unit_acc = leg.accrued;
    leg.face = face;
    leg.book_value = per_unit_bv * face;
    leg.accrued = per_unit_acc * face;
    leg.realised = clean * face / 100.0 - leg.book_value;
    leg.realised_bhd = leg.realised * spot;
    leg
}

/// The lots a sale of `face` draws on: those chosen, or the oldest first up to their free face.
/// Returns the legs and what could not be allocated.
#[allow(clippy::too_many_arguments)]
pub fn allocate(m: &Market, p: &Portfolio, isin: &str, book: Option<&str>, settle: NaiveDate, clean: f64, face: f64, chosen: Option<&[LotPick]>) -> (Vec<SaleLeg>, Vec<SaleLeg>, f64) {
    let candidates = sale_candidates(m, p, isin, book, settle);
    let spot = p.bonds.get(isin).map(|b| m.spot(&b.ccy, settle)).unwrap_or(1.0);
    let mut legs = Vec::new();
    let mut left = face;
    match chosen {
        Some(picks) => {
            for pick in picks {
                if let Some(c) = candidates.iter().find(|c| c.lot_id == pick.lot_id) {
                    legs.push(take(c.clone(), pick.face, clean, spot));
                    left -= pick.face;
                }
            }
        }
        None => {
            for c in &candidates {
                if left <= 0.005 {
                    break;
                }
                let part = c.free.min(left);
                if part > 0.005 {
                    legs.push(take(c.clone(), part, clean, spot));
                    left -= part;
                }
            }
        }
    }
    (legs, candidates, left.max(0.0))
}

/// Booked sales of a bond with the gain each realised, newest first.
pub fn sale_views(m: &Market, p: &Portfolio, isin: &str) -> Vec<SaleView> {
    let Some(b) = p.bonds.get(isin) else { return Vec::new() };
    let mut out = Vec::new();
    for l in p.lots.iter().filter(|l| l.isin == isin && !l.sales.is_empty()) {
        let first = l.sales.iter().map(|s| s.settle_date).min().unwrap_or(l.settle_date);
        let last = l.sales.iter().map(|s| s.settle_date).max().unwrap_or(l.settle_date);
        let mut by_day: std::collections::BTreeMap<NaiveDate, f64> = std::collections::BTreeMap::new();
        walk_lot(m, l, b, first, last, |d| {
            if d.realised != 0.0 {
                by_day.insert(d.date, d.realised);
            }
        });
        for s in &l.sales {
            let same_day: f64 = l.sales.iter().filter(|x| x.settle_date == s.settle_date).map(|x| x.face).sum();
            let realised = by_day.get(&s.settle_date).copied().unwrap_or(0.0) * s.face / same_day.max(1e-9);
            out.push(SaleView {
                sale_ref: s.sale_ref.clone(),
                lot_id: l.id,
                book: l.book.clone(),
                trade_date: s.trade_date,
                settle_date: s.settle_date,
                face: s.face,
                clean_price: s.clean_price,
                realised_bhd: realised * m.spot(&b.ccy, s.settle_date),
                kondor_id: s.kondor_id.clone(),
            });
        }
    }
    out.sort_by(|a, b| b.settle_date.cmp(&a.settle_date).then_with(|| a.sale_ref.cmp(&b.sale_ref)));
    out
}

/// Lots with face free to pledge over `[start, end)`, largest free face first.
pub fn pledge_candidates(m: &Market, p: &Portfolio, ccy: Option<&str>, start: NaiveDate, end: NaiveDate) -> Vec<PledgeCandidate> {
    let mut out: Vec<PledgeCandidate> = p
        .lots
        .iter()
        .filter_map(|l| {
            let b = p.bonds.get(&l.isin)?;
            if l.settle_date > start || b.maturity <= end || ccy.is_some_and(|c| c != b.ccy) {
                return None;
            }
            let held = l.face_on(start).min(l.face_on(end));
            let free = held - pledged_between(p, l.id, start, end, None);
            (free > 0.5).then(|| PledgeCandidate {
                lot_id: l.id,
                isin: b.isin.clone(),
                name: b.name.clone(),
                book: l.book.clone(),
                ccy: b.ccy.clone(),
                free,
                price: m.price(&b.isin, start).map(|x| x.0).unwrap_or(l.clean_price),
            })
        })
        .collect();
    out.sort_by(|a, b| b.free.total_cmp(&a.free));
    out
}

/// What a repo raises and costs, and what the counterparty last quoted.
#[allow(clippy::too_many_arguments)]
pub fn repo_preview(m: &Market, p: &Portfolio, direction: &str, counterparty: &str, ccy: &str, start: NaiveDate, end: NaiveDate, rate: f64, haircut: f64, cash: Option<f64>, collateral: &[LotPick], except: Option<Uuid>) -> RepoPreview {
    let mut warnings = Vec::new();
    let mut blocking = Vec::new();
    if end <= start {
        blocking.push("the repo must end after it starts".into());
    }
    let mut value = 0.0;
    if direction == "REPO" {
        if collateral.is_empty() {
            blocking.push("pick the bonds to pledge".into());
        }
        for c in collateral {
            let Some(l) = p.lots.iter().find(|l| l.id == c.lot_id) else {
                blocking.push("a pledged lot no longer exists".into());
                continue;
            };
            let b = &p.bonds[&l.isin];
            let free = l.face_on(start).min(l.face_on(end)) - pledged_between(p, l.id, start, end, except);
            if c.face > free + 0.005 {
                blocking.push(format!("{} has only {:.0} free to pledge over the term", b.name, free.max(0.0)));
            }
            if b.maturity <= end {
                blocking.push(format!("{} matures before the repo ends", b.name));
            }
            if b.ccy != ccy {
                warnings.push(format!("{} is in {}, the cash in {ccy}", b.name, b.ccy));
            }
            let price = m.price(&b.isin, start).map(|x| x.0).unwrap_or(l.clean_price);
            let acc = accrued(b, &schedule(b), start, b.coupon);
            value += (price + acc) * c.face / 100.0 * m.spot(&b.ccy, start) / m.spot(ccy, start).max(1e-12);
        }
    }
    let computed = value * (1.0 - haircut / 100.0);
    let cash_v = cash.unwrap_or(computed);
    if direction == "REPO" && cash.is_some_and(|c| c > computed * 1.0001 && computed > 0.0) {
        warnings.push(format!("the cash is more than the collateral raises after the haircut ({computed:.0})"));
    }
    let days = (end - start).num_days().max(0);
    let interest = cash_v * rate / 100.0 * days as f64 / mm_basis(ccy);
    // The latest quote from the counterparty at the nearest tenor.
    let q = p
        .quotes
        .iter()
        .filter(|q| q.counterparty.eq_ignore_ascii_case(counterparty.trim()) && q.as_of <= start.max(m.as_of))
        .min_by_key(|q| ((q.tenor_days as i64 - days).abs(), -(q.as_of - NaiveDate::MIN).num_days()));
    let sofr = q.map(|q| m.fixing("USD", q.as_of, q.tenor_days as i64)).or_else(|| (days > 0).then(|| m.fixing("USD", m.as_of.min(start), days)));
    if q.is_some_and(|q| (m.as_of - q.as_of).num_days() > 2) {
        warnings.push("the counterparty's latest quote is more than two days old".into());
    }
    let spot = m.spot(ccy, start);
    RepoPreview {
        collateral_value: value,
        cash: cash_v,
        cash_bhd: cash_v * spot,
        interest,
        interest_bhd: interest * spot,
        days,
        quote_spread_bp: q.map(|q| q.spread_bp),
        quote_tenor: q.map(|q| q.tenor.clone()),
        quote_as_of: q.map(|q| q.as_of),
        sofr,
        indicative_rate: match (q, sofr) {
            (Some(q), Some(s)) => Some(s + q.spread_bp / 100.0),
            _ => None,
        },
        available: if direction == "REPO" { pledge_candidates(m, p, None, start, end) } else { Vec::new() },
        warnings,
        blocking,
    }
}

/// A repo as the engine sees it, for checking a roll against the rest of the book.
pub fn find_repo(p: &Portfolio, id: Uuid) -> Option<&Repo> {
    p.repos.iter().find(|r| r.id == id)
}
