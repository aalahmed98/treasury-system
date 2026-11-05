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
