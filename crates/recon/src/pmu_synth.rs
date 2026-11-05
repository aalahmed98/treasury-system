//! Synthetic data for the bond portfolio desk and the FTP curves it is charged with.
//!
//! The FTP publications stand in for ALM's daily workbook until the bank's own arrive: for each
//! business day of the market history, one curve per currency, the market curve plus a term
//! premium. Days that already have a publication (synthetic or ALM's own) are left alone, so a
//! re-run only adds the new days and never overwrites what ALM published.

use std::collections::BTreeSet;

use chrono::{Duration, NaiveDate};
use domain::calendar::Calendar;
use sqlx::{PgPool, QueryBuilder};

use crate::alm_synth::{Market, TENORS};

/// The currencies the bank publishes FTP curves for today.
pub const FTP_CCYS: [&str; 4] = ["BHD", "USD", "AED", "SAR"];

/// The liquidity premium ALM adds over the market curve, in percent: 10 bp overnight rising with
/// the square root of the term to about 1.5% at ten years.
pub fn term_premium(tenor_days: i32) -> f64 {
    let years = tenor_days.max(1) as f64 / 365.0;
    (10.0 + 45.0 * years.sqrt()) / 100.0
}

/// The synthetic FTP rate for a currency, day and tenor, in percent.
pub fn ftp_rate(market: &Market, ccy: &str, on: NaiveDate, tenor_days: i32) -> f64 {
    ((market.rate(ccy, on, tenor_days) + term_premium(tenor_days)) * 10_000.0).round() / 10_000.0
}

/// Writes a publication for every business day of the market history that has none.
pub async fn write_ftp(pool: &PgPool, cal: &Calendar, market: &Market) -> anyhow::Result<usize> {
    let Some((first, last)) = market.span() else { return Ok(0) };
    let have: BTreeSet<NaiveDate> =
        sqlx::query_scalar::<_, NaiveDate>("SELECT DISTINCT as_of FROM ftp_publications WHERE as_of BETWEEN $1 AND $2").bind(first).bind(last).fetch_all(pool).await?.into_iter().collect();
    let mut days = Vec::new();
    let mut d = first;
    while d <= last {
        if cal.is_business_day(d) && !have.contains(&d) {
            days.push(d);
        }
        d += Duration::days(1);
    }
    let mut added = 0;
    for chunk in days.chunks(200) {
        let mut tx = pool.begin().await?;
        let mut qb = QueryBuilder::new("INSERT INTO ftp_publications (as_of, published_at, note) ");
        qb.push_values(chunk, |mut b, d| {
            let at = d.and_hms_opt(5, 30, 0).map(|t| t.and_utc());
            b.push_bind(*d).push_bind(at).push_bind("Synthetic curves (market curve plus a term premium) until ALM's workbook is published");
        });
        qb.push(" RETURNING id, as_of");
        let ids: Vec<(i32, NaiveDate)> = qb.build_query_as().fetch_all(&mut *tx).await?;
        let mut points = Vec::with_capacity(ids.len() * FTP_CCYS.len() * TENORS.len());
        for (id, d) in &ids {
            for ccy in FTP_CCYS {
                for (tenor, td) in TENORS {
                    points.push((*id, ccy, tenor, td, ftp_rate(market, ccy, *d, td)));
                }
            }
        }
        for part in points.chunks(3000) {
            let mut qb = QueryBuilder::new("INSERT INTO ftp_points (publication_id, ccy, tenor, tenor_days, rate) ");
            qb.push_values(part, |mut b, p| {
                b.push_bind(p.0).push_bind(p.1).push_bind(p.2).push_bind(p.3).push_bind(p.4);
            });
            qb.build().execute(&mut *tx).await?;
        }
        tx.commit().await?;
        added += ids.len();
    }
    if added > 0
        && let Some(d) = days.last()
    {
        sqlx::query("SELECT pg_notify('alm_events', json_build_object('ftp', $1::date)::text)").bind(d).execute(pool).await?;
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_premium_rises_with_the_term() {
        assert!((term_premium(1) - 0.1235).abs() < 1e-3);
        assert!((term_premium(365) - 0.55).abs() < 1e-9);
        assert!(term_premium(3650) > 1.5 && term_premium(3650) < 1.6);
        let m = Market::generate(42, NaiveDate::from_ymd_opt(2026, 11, 5).unwrap(), 30);
        let d = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        assert!(ftp_rate(&m, "USD", d, 1825) > m.rate("USD", d, 1825));
    }
}

// ---------------------------------------------------------------------------------------------
// The bond portfolio
// ---------------------------------------------------------------------------------------------

use std::collections::BTreeMap;

use alm::curve::Curve;
use alm::schedule::add_months;
use chrono::{Datelike, Utc};
use domain::{Bond, BondDayCount, BondRateType, CurvePoint, FtpCurve};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use uuid::Uuid;

use crate::alm_synth::normal;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PmuSummary {
    pub bonds: usize,
    pub lots: usize,
    pub swaps: usize,
    pub repos: usize,
    pub quotes: usize,
    pub prices: usize,
}

/// The engine's view of the synthetic market: curves by day, today's curves, spots.
pub fn engine_market(market: &Market, as_of: NaiveDate) -> pmu::Market {
    let mut history: BTreeMap<String, BTreeMap<NaiveDate, Vec<(f64, f64)>>> = BTreeMap::new();
    for r in &market.rates {
        history.entry(r.ccy.clone()).or_default().entry(r.as_of).or_default().push((r.tenor_days as f64 / 365.0, r.rate));
    }
    let history: BTreeMap<String, BTreeMap<NaiveDate, Curve>> = history
        .into_iter()
        .map(|(ccy, days)| {
            let m = days
                .into_iter()
                .map(|(d, mut pts)| {
                    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
                    (d, Curve { ccy: ccy.clone(), points: pts })
                })
                .collect();
            (ccy, m)
        })
        .collect();
    let today = history.iter().filter_map(|(c, m)| m.range(..=as_of).next_back().map(|(_, cv)| (c.clone(), cv.clone()))).collect();
    let mut spots: pmu::market::Spots = BTreeMap::new();
    for s in &market.spots {
        if let Some(ccy) = s.pair.strip_suffix("/BHD") {
            spots.entry(s.as_of).or_default().insert(ccy.to_string(), s.mid);
        }
    }
    pmu::Market { as_of, spots, history, today, prices: BTreeMap::new(), ftp: pmu::Ftp::default() }
}

async fn load_ftp(pool: &PgPool) -> anyhow::Result<pmu::Ftp> {
    let rows: Vec<(i32, NaiveDate, String, String, i32, rust_decimal::Decimal)> = sqlx::query_as(
        "SELECT p.id, p.as_of, x.ccy::text, x.tenor, x.tenor_days, x.rate FROM ftp_publications p JOIN ftp_points x ON x.publication_id = p.id \
         WHERE p.is_current ORDER BY p.as_of, x.ccy, x.tenor_days",
    )
    .fetch_all(pool)
    .await?;
    let mut by: BTreeMap<(NaiveDate, i32), BTreeMap<String, Vec<CurvePoint>>> = BTreeMap::new();
    for (id, d, ccy, tenor, days, rate) in rows {
        use rust_decimal::prelude::ToPrimitive;
        by.entry((d, id)).or_default().entry(ccy).or_default().push(CurvePoint { tenor, tenor_days: days, rate: rate.to_f64().unwrap_or(0.0) });
    }
    Ok(pmu::Ftp::from_publications(by.into_iter().map(|((d, id), m)| (d, id, m.into_iter().map(|(ccy, points)| FtpCurve { ccy, points }).collect()))))
}

/// (issuer label, issuer type, currency, how many, spread over the curve in percent, day count,
/// coupons a year, HQLA level)
#[allow(clippy::type_complexity)]
const UNIVERSE: &[(&str, &str, &str, usize, f64, BondDayCount, u32, Option<&str>)] = &[
    ("Bahrain sovereign", "SOVEREIGN", "BHD", 9, 0.55, BondDayCount::ActAct, 2, Some("L1")),
    ("Bahrain sovereign", "SOVEREIGN", "USD", 4, 1.60, BondDayCount::Thirty360, 2, Some("L2B")),
    ("Saudi sovereign", "GCC_SOVEREIGN", "USD", 4, 0.70, BondDayCount::Thirty360, 2, Some("L2A")),
    ("Abu Dhabi sovereign", "GCC_SOVEREIGN", "USD", 3, 0.45, BondDayCount::Thirty360, 2, Some("L2A")),
    ("Qatar sovereign", "GCC_SOVEREIGN", "USD", 3, 0.50, BondDayCount::Thirty360, 2, Some("L2A")),
    ("Saudi sukuk", "SUKUK", "SAR", 4, 0.40, BondDayCount::Act360, 2, Some("L2A")),
    ("UAE sukuk", "SUKUK", "AED", 3, 0.45, BondDayCount::Act360, 2, Some("L2A")),
    ("Supranational", "SUPRANATIONAL", "USD", 2, 0.15, BondDayCount::Thirty360, 2, Some("L1")),
    ("GCC bank", "BANK", "USD", 4, 1.20, BondDayCount::Thirty360, 2, None),
    ("GCC corporate", "CORPORATE", "USD", 3, 1.55, BondDayCount::Thirty360, 2, None),
];

const REPO_CPTYS: [(&str, f64); 6] = [("Citibank N.A.", 28.0), ("JPMorgan Chase", 30.0), ("Standard Chartered", 35.0), ("HSBC Bahrain", 38.0), ("Emirates NBD", 45.0), ("Gulf International Bank", 52.0)];
const QUOTE_TENORS: [(&str, i32, f64); 3] = [("1W", 7, 0.0), ("1M", 30, 4.0), ("3M", 91, 9.0)];

fn country(ccy: &str) -> &'static str {
    match ccy {
        "BHD" => "BH",
        "SAR" => "SA",
        "AED" => "AE",
        _ => "XS",
    }
}

fn round_to(x: f64, step: f64) -> f64 {
    (x / step).round() * step
}

struct Gen {
    bonds: Vec<(Bond, f64)>,
    lots: Vec<domain::Lot>,
    swaps: Vec<domain::Swap>,
    repos: Vec<domain::Repo>,
}

/// The whole portfolio, deterministic in the seed and the business date's market.
fn generate(market: &Market, pm: &pmu::Market, ftp: &pmu::Ftp, first: NaiveDate, today: NaiveDate, seed: u64) -> Gen {
    let mut rng = StdRng::seed_from_u64(seed ^ 0x504d_5500_0000);
    let mut bonds: Vec<(Bond, f64)> = Vec::new();
    let mut n = 0;
    for (issuer, kind, ccy, count, spread, dc, freq, hqla) in UNIVERSE {
        for k in 0..*count {
            n += 1;
            let years = 1.0 + rng.random_range(0.0..8.5);
            let mut maturity = add_months(today, (years * 12.0) as i64);
            maturity = NaiveDate::from_ymd_opt(maturity.year(), maturity.month(), 15).unwrap_or(maturity);
            let tenor = [3, 5, 5, 7, 10][rng.random_range(0..5)];
            let issue = add_months(maturity, -12 * tenor as i64);
            let float = *kind == "BANK" && k == 0;
            let coupon = if float { 0.0 } else { round_to(market.rate(ccy, issue, tenor * 365) + spread, 0.125).max(0.5) };
            let isin = format!("{}SYN{:06}{}", country(ccy), n, n % 10);
            let b = Bond {
                isin,
                name: if float { format!("{issuer} SOFR FRN {}", maturity.year()) } else { format!("{issuer} {coupon:.3}% {}", maturity.year()) },
                issuer: issuer.to_string(),
                issuer_type: kind.to_string(),
                ccy: ccy.to_string(),
                coupon,
                coupon_freq: if float { 4 } else { *freq },
                day_count: if float { BondDayCount::Act360 } else { *dc },
                issue_date: issue,
                maturity,
                rate_type: if float { BondRateType::Float } else { BondRateType::Fixed },
                float_index: float.then(|| "SOFR".to_string()),
                float_spread: if float { round_to(1.0 + rng.random_range(0.0..0.4), 0.05) } else { 0.0 },
                hqla_level: hqla.map(str::to_string),
            };
            bonds.push((b, *spread + 0.15 * normal(&mut rng)));
        }
    }
    // Two dollar bonds that matured during the year, so the history has redemptions in it.
    for (i, months_ago) in [(0, 7), (1, 3)] {
        n += 1;
        let maturity = add_months(today, -months_ago);
        let issue = add_months(maturity, -36);
        let coupon = round_to(market.rate("USD", issue, 1095) + 0.6, 0.125);
        bonds.push((
            Bond {
                isin: format!("XSSYN{n:06}{}", n % 10),
                name: format!("GCC bank {coupon:.3}% {}", maturity.year()),
                issuer: "GCC bank".into(),
                issuer_type: "BANK".into(),
                ccy: "USD".into(),
                coupon,
                coupon_freq: 2,
                day_count: BondDayCount::Thirty360,
                issue_date: issue,
                maturity,
                rate_type: BondRateType::Fixed,
                float_index: None,
                float_spread: 0.0,
                hqla_level: None,
            },
            0.6 + 0.1 * i as f64,
        ));
    }

    let mut lots = Vec::new();
    let mut kondor = 4_210_000;
    for (b, spread) in &bonds {
        let sched = pmu::bond::schedule(b);
        let lots_n = rng.random_range(2..5);
        let last = (b.maturity - Duration::days(30)).min(today - Duration::days(3));
        if last <= first {
            continue;
        }
        for _ in 0..lots_n {
            let span = (last - first).num_days();
            let trade = first + Duration::days(rng.random_range(0..span.max(1)));
            let settle = trade + Duration::days(2);
            if settle >= b.maturity {
                continue;
            }
            let rem = (b.maturity - settle).num_days();
            let clean = match b.rate_type {
                BondRateType::Fixed => {
                    let y = market.rate(&b.ccy, trade, rem as i32) + spread + 0.08 * normal(&mut rng);
                    round_to(pmu::bond::clean_from_yield(b, &sched, settle, y), 0.001)
                }
                BondRateType::Float => round_to(99.9 + 0.15 * normal(&mut rng), 0.001),
            };
            let face = match b.ccy.as_str() {
                "BHD" => round_to(rng.random_range(2.0..9.0), 0.5) * 1e6,
                "SAR" | "AED" => round_to(rng.random_range(15.0..60.0), 5.0) * 1e6,
                _ => round_to(rng.random_range(5.0..26.0), 1.0) * 1e6,
            };
            let book = if b.ccy == "BHD" { "AFS BHD" } else if rng.random_range(0.0..1.0) < 0.3 { "HTM" } else { "AFS USD" };
            kondor += rng.random_range(3..400);
            let ftp_lock = ftp.rate(&b.ccy, trade, rem);
            let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("pmu-lot:{}:{}:{}", b.isin, trade, face).as_bytes());
            let accrued = pmu::bond::accrued(b, &sched, settle, b.coupon) * face / 100.0;
            lots.push(domain::Lot {
                id,
                book: book.into(),
                isin: b.isin.clone(),
                trade_date: trade,
                settle_date: settle,
                face,
                clean_price: clean,
                accrued_paid: (accrued * 100.0).round() / 100.0,
                kondor_id: (rng.random_range(0.0..1.0) > 0.1).then(|| format!("KND{kondor:07}")),
                eir: pmu::eval::lot_eir(b, settle, clean, pm),
                ftp_rate: ftp_lock.map(|x| x.0),
                ftp_publication_id: ftp_lock.map(|x| x.1),
                status: if b.maturity <= today { "MATURED".into() } else { "OPEN".into() },
            });
        }
    }

    // Payer swaps on most fixed dollar lots in the AFS book with two years or more to run.
    let mut swaps = Vec::new();
    for l in &lots {
        let Some((b, _)) = bonds.iter().find(|(b, _)| b.isin == l.isin) else { continue };
        if b.ccy != "USD" || b.rate_type != BondRateType::Fixed || l.book != "AFS USD" || (b.maturity - l.settle_date).num_days() < 730 || rng.random_range(0.0..1.0) > 0.7 {
            continue;
        }
        let notional = round_to(l.face * rng.random_range(0.5..1.0), 1e6).max(1e6).min(l.face);
        let maturity = if rng.random_range(0.0..1.0) < 0.2 { add_months(b.maturity, -12).max(l.settle_date + Duration::days(400)) } else { b.maturity };
        let tenor = (maturity - l.settle_date).num_days() as i32;
        let fixed = round_to(market.rate("USD", l.trade_date, tenor) - 0.05, 0.0001);
        let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("pmu-irs:{}", l.id).as_bytes());
        let mut unwinds = Vec::new();
        if rng.random_range(0.0..1.0) < 0.18 && today - l.settle_date > Duration::days(90) {
            let span = (today - l.settle_date).num_days() - 60;
            let date = l.settle_date + Duration::days(60 + rng.random_range(0..span.max(1)));
            let un = round_to(notional * rng.random_range(0.25..0.5), 1e6).max(1e6).min(notional - 1e6);
            if un > 0.0 {
                let rem_years = (maturity - date).num_days() as f64 / 365.0;
                let settlement = un * (market.rate("USD", date, (maturity - date).num_days() as i32) - fixed) / 100.0 * rem_years * 0.9;
                unwinds.push(domain::SwapUnwind { date, notional: un, settlement: (settlement * 100.0).round() / 100.0 });
            }
        }
        swaps.push(domain::Swap {
            id,
            book: l.book.clone(),
            kondor_id: Some(format!("IRS{:07}", 1_200_000 + swaps.len() * 37)),
            ccy: "USD".into(),
            notional,
            start: l.settle_date,
            maturity,
            pay_fixed: true,
            fixed_rate: fixed,
            fixed_freq: 2,
            fixed_day_count: BondDayCount::Thirty360,
            float_index: "SOFR".into(),
            float_tenor_months: 3,
            float_spread: 0.0,
            status: "ACTIVE".into(),
            hedges: vec![(l.id, notional)],
            unwinds,
        });
    }

    // Repos against dollar sovereign and bank lots, each lot pledged at most once.
    let mut repos = Vec::new();
    let mut pledged: std::collections::BTreeSet<Uuid> = std::collections::BTreeSet::new();
    let candidates: Vec<&domain::Lot> = lots
        .iter()
        .filter(|l| l.status == "OPEN" && bonds.iter().any(|(b, _)| b.isin == l.isin && b.ccy == "USD" && b.rate_type == BondRateType::Fixed))
        .collect();
    for i in 0..22 {
        let (cpty, spread) = REPO_CPTYS[i % REPO_CPTYS.len()];
        let tenor = [7, 14, 30, 30, 60, 90, 180][rng.random_range(0..7)];
        let start = today - Duration::days(rng.random_range(1..(if i < 16 { tenor.min(150) } else { 150 }).max(2)));
        let end = start + Duration::days(tenor);
        let mut coll = Vec::new();
        let mut value = 0.0;
        for _ in 0..rng.random_range(1..3) {
            let free: Vec<&&domain::Lot> = candidates.iter().filter(|l| !pledged.contains(&l.id) && l.settle_date < start).collect();
            if free.is_empty() {
                break;
            }
            let l = free[rng.random_range(0..free.len())];
            pledged.insert(l.id);
            let face = round_to(l.face * rng.random_range(0.4..1.0), 1e6).max(1e6).min(l.face);
            coll.push((l.id, face));
            value += face * l.clean_price / 100.0;
        }
        if coll.is_empty() {
            continue;
        }
        let haircut = round_to(rng.random_range(2.0..8.0), 0.5);
        let rate = round_to(market.rate("USD", start, tenor as i32) + (spread + 6.0 * normal(&mut rng)) / 100.0, 0.0001);
        repos.push(domain::Repo {
            id: Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("pmu-repo:{i}:{start}").as_bytes()),
            counterparty: cpty.into(),
            kondor_id: Some(format!("REP{:07}", 3_300_000 + i * 53)),
            ccy: "USD".into(),
            start,
            end,
            cash: round_to(value * (1.0 - haircut / 100.0), 1000.0),
            rate,
            haircut_pct: haircut,
            collateral: coll,
        });
    }
    Gen { bonds, lots, swaps, repos }
}

/// Daily clean prices and yields for `days` up to `today`: the curve plus the bond's own spread
/// wandering a little.
fn prices(market: &Market, bonds: &[(Bond, f64)], from: NaiveDate, today: NaiveDate, seed: u64) -> Vec<(NaiveDate, String, f64, f64)> {
    let mut out = Vec::new();
    for (b, spread) in bonds {
        if b.maturity <= from {
            continue;
        }
        let sched = pmu::bond::schedule(b);
        let mut rng = StdRng::seed_from_u64(seed ^ crate::alm_synth::hash(&b.isin) ^ 0x7072_6963);
        let mut wander = 0.0;
        let mut d = from;
        while d <= today && d < b.maturity {
            wander = 0.92 * wander + 0.02 * normal(&mut rng);
            let (clean, ytm) = match b.rate_type {
                BondRateType::Fixed => {
                    let y = market.rate(&b.ccy, d, (b.maturity - d).num_days() as i32) + spread + wander;
                    (pmu::bond::clean_from_yield(b, &sched, d, y), y)
                }
                BondRateType::Float => (100.0 + 0.4 * wander, market.rate(&b.ccy, d, 91) + b.float_spread),
            };
            out.push((d, b.isin.clone(), (clean * 1e6).round() / 1e6, (ytm * 1e6).round() / 1e6));
            d += Duration::days(1);
        }
    }
    out
}

type QuoteRow = (NaiveDate, &'static str, &'static str, i32, f64, chrono::DateTime<Utc>);

fn quotes(market: &Market, cal: &Calendar, from: NaiveDate, today: NaiveDate, seed: u64) -> Vec<QuoteRow> {
    let mut out = Vec::new();
    for (ci, (cpty, base)) in REPO_CPTYS.iter().enumerate() {
        let mut rng = StdRng::seed_from_u64(seed ^ crate::alm_synth::hash(cpty) ^ 0x7175_6f74);
        // The last two counterparties went quiet a few days ago, so the page has stale quotes.
        let stop = if ci >= 4 { today - Duration::days(3 + ci as i64) } else { today };
        let mut d = from;
        while d <= stop {
            if cal.is_business_day(d) && rng.random_range(0.0..1.0) < 0.8 {
                for (tenor, days, bump) in QUOTE_TENORS {
                    let s = base + bump + 3.0 * normal(&mut rng) + (market.rate("USD", d, 365) - 4.3) * 2.0;
                    let at = d.and_hms_opt(7, 30 + rng.random_range(0..90) as u32 % 30, 0).map(|t| t.and_utc()).unwrap_or_else(Utc::now);
                    out.push((d, *cpty, tenor, days, (s * 100.0).round() / 100.0, at));
                }
            }
            d += Duration::days(1);
        }
    }
    out
}

/// Writes the bond portfolio once, then on each run extends prices and repo quotes to the
/// business date.
pub async fn write_portfolio(pool: &PgPool, cal: &Calendar, market: &Market, today: NaiveDate, seed: u64) -> anyhow::Result<PmuSummary> {
    let mut sum = PmuSummary::default();
    let Some((first, _)) = market.span() else { return Ok(sum) };
    let have: i64 = sqlx::query_scalar("SELECT count(*) FROM bonds").fetch_one(pool).await?;
    let ftp = load_ftp(pool).await?;
    let pm = engine_market(market, today);
    let start = first + Duration::days(5);
    let bonds: Vec<(Bond, f64)> = if have == 0 {
        let g = generate(market, &pm, &ftp, start, today, seed);
        let mut tx = pool.begin().await?;
        let mut qb = QueryBuilder::new("INSERT INTO bonds (isin, name, issuer, issuer_type, ccy, coupon, coupon_freq, day_count, issue_date, maturity, rate_type, float_index, float_spread, hqla_level) ");
        qb.push_values(&g.bonds, |mut q, (b, _)| {
            q.push_bind(&b.isin)
                .push_bind(&b.name)
                .push_bind(&b.issuer)
                .push_bind(&b.issuer_type)
                .push_bind(&b.ccy)
                .push_bind(b.coupon)
                .push_bind(b.coupon_freq as i16)
                .push_bind(b.day_count.as_str())
                .push_bind(b.issue_date)
                .push_bind(b.maturity)
                .push_bind(b.rate_type.as_str())
                .push_bind(&b.float_index)
                .push_bind(b.float_spread)
                .push_bind(&b.hqla_level);
        });
        qb.build().execute(&mut *tx).await?;
        for chunk in g.lots.chunks(500) {
            let mut qb = QueryBuilder::new("INSERT INTO bond_lots (id, book, isin, trade_date, settle_date, face, clean_price, accrued_paid, kondor_id, eir, ftp_rate, ftp_publication_id, status) ");
            qb.push_values(chunk, |mut q, l| {
                q.push_bind(l.id)
                    .push_bind(&l.book)
                    .push_bind(&l.isin)
                    .push_bind(l.trade_date)
                    .push_bind(l.settle_date)
                    .push_bind(l.face)
                    .push_bind(l.clean_price)
                    .push_bind(l.accrued_paid)
                    .push_bind(&l.kondor_id)
                    .push_bind((l.eir * 1e8).round() / 1e8)
                    .push_bind(l.ftp_rate)
                    .push_bind(l.ftp_publication_id)
                    .push_bind(&l.status);
            });
            qb.build().execute(&mut *tx).await?;
        }
        for s in &g.swaps {
            sqlx::query(
                "INSERT INTO irs (id, book, kondor_id, ccy, notional, start_date, maturity, pay_fixed, fixed_rate, fixed_freq, fixed_day_count, float_index, float_tenor_months, float_spread, status) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
            )
            .bind(s.id)
            .bind(&s.book)
            .bind(&s.kondor_id)
            .bind(&s.ccy)
            .bind(s.notional)
            .bind(s.start)
            .bind(s.maturity)
            .bind(s.pay_fixed)
            .bind(s.fixed_rate)
            .bind(s.fixed_freq as i16)
            .bind(s.fixed_day_count.as_str())
            .bind(&s.float_index)
            .bind(s.float_tenor_months as i16)
            .bind(s.float_spread)
            .bind(if s.maturity <= today { "MATURED" } else { "ACTIVE" })
            .execute(&mut *tx)
            .await?;
            for (lot, n) in &s.hedges {
                sqlx::query("INSERT INTO irs_hedges (irs_id, lot_id, notional) VALUES ($1, $2, $3)").bind(s.id).bind(lot).bind(n).execute(&mut *tx).await?;
            }
            for u in &s.unwinds {
                sqlx::query("INSERT INTO irs_unwinds (irs_id, date, notional, settlement) VALUES ($1, $2, $3, $4)").bind(s.id).bind(u.date).bind(u.notional).bind(u.settlement).execute(&mut *tx).await?;
            }
        }
        for r in &g.repos {
            sqlx::query("INSERT INTO repos (id, counterparty, kondor_id, ccy, start_date, end_date, cash, rate, haircut_pct) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
                .bind(r.id)
                .bind(&r.counterparty)
                .bind(&r.kondor_id)
                .bind(&r.ccy)
                .bind(r.start)
                .bind(r.end)
                .bind(r.cash)
                .bind(r.rate)
                .bind(r.haircut_pct)
                .execute(&mut *tx)
                .await?;
            for (lot, f) in &r.collateral {
                sqlx::query("INSERT INTO repo_collateral (repo_id, lot_id, face) VALUES ($1, $2, $3)").bind(r.id).bind(lot).bind(f).execute(&mut *tx).await?;
            }
        }
        tx.commit().await?;
        sum.bonds = g.bonds.len();
        sum.lots = g.lots.len();
        sum.swaps = g.swaps.len();
        sum.repos = g.repos.len();
        g.bonds
    } else {
        // Later runs: the stored bonds with the spread their last price implies.
        #[allow(clippy::type_complexity)]
        let rows: Vec<(String, String, String, String, String, rust_decimal::Decimal, i16, String, NaiveDate, NaiveDate, String, Option<String>, rust_decimal::Decimal, Option<rust_decimal::Decimal>, Option<NaiveDate>)> =
            sqlx::query_as(
                "SELECT b.isin, b.name, b.issuer, b.issuer_type, b.ccy::text, b.coupon, b.coupon_freq, b.day_count, b.issue_date, b.maturity, b.rate_type, b.float_index, b.float_spread, \
                        p.ytm, p.as_of \
                 FROM bonds b LEFT JOIN LATERAL (SELECT ytm, as_of FROM bond_prices x WHERE x.isin = b.isin ORDER BY as_of DESC LIMIT 1) p ON true",
            )
            .fetch_all(pool)
            .await?;
        use rust_decimal::prelude::ToPrimitive;
        rows
            .into_iter()
            .map(|(isin, name, issuer, kind, ccy, coupon, freq, dc, issue, maturity, rt, fi, fs, ytm, at)| {
                let b = Bond {
                    isin,
                    name,
                    issuer,
                    issuer_type: kind,
                    ccy,
                    coupon: coupon.to_f64().unwrap_or(0.0),
                    coupon_freq: freq as u32,
                    day_count: dc.parse().unwrap_or(BondDayCount::Thirty360),
                    issue_date: issue,
                    maturity,
                    rate_type: rt.parse().unwrap_or(BondRateType::Fixed),
                    float_index: fi,
                    float_spread: fs.to_f64().unwrap_or(0.0),
                    hqla_level: None,
                };
                let spread = match (ytm, at) {
                    (Some(y), Some(d)) => y.to_f64().unwrap_or(0.0) - market.rate(&b.ccy, d, (b.maturity - d).num_days().max(1) as i32),
                    _ => 0.6,
                };
                (b, spread)
            })
            .collect()
    };

    // Prices and quotes from the day after the last held up to the business date.
    let last_price: Option<NaiveDate> = sqlx::query_scalar("SELECT max(as_of) FROM bond_prices").fetch_one(pool).await?;
    let from_p = last_price.map(|d| d + Duration::days(1)).unwrap_or(today - Duration::days(60)).max(start);
    let px = prices(market, &bonds, from_p, today, seed);
    for chunk in px.chunks(3000) {
        let mut qb = QueryBuilder::new("INSERT INTO bond_prices (as_of, isin, clean_price, ytm) ");
        qb.push_values(chunk, |mut q, (d, isin, c, y)| {
            q.push_bind(*d).push_bind(isin).push_bind(*c).push_bind(*y);
        });
        qb.push(" ON CONFLICT DO NOTHING");
        sum.prices += qb.build().execute(pool).await?.rows_affected() as usize;
    }
    let last_quote: Option<NaiveDate> = sqlx::query_scalar("SELECT max(as_of) FROM repo_quotes").fetch_one(pool).await?;
    let from_q = last_quote.map(|d| d + Duration::days(1)).unwrap_or(today - Duration::days(30)).max(start);
    let qs = quotes(market, cal, from_q, today, seed);
    for chunk in qs.chunks(3000) {
        let mut qb = QueryBuilder::new("INSERT INTO repo_quotes (as_of, counterparty, tenor, tenor_days, spread_bp, quoted_at) ");
        qb.push_values(chunk, |mut q, (d, c, t, td, s, at)| {
            q.push_bind(*d).push_bind(*c).push_bind(*t).push_bind(*td).push_bind(*s).push_bind(*at);
        });
        qb.push(" ON CONFLICT DO NOTHING");
        sum.quotes += qb.build().execute(pool).await?.rows_affected() as usize;
    }
    if sum.lots > 0 || sum.prices > 0 || sum.quotes > 0 {
        sqlx::query("SELECT pg_notify('alm_events', json_build_object('pmu', true)::text)").execute(pool).await?;
    }
    Ok(sum)
}
