//! Synthetic nostro statements: what the correspondent banks would have reported for the cash
//! the golden record expected. Real MT942 / camt.052 feeds replace this later; the tables and the
//! ladder stay the same.

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use sqlx::PgPool;

use crate::calendar::Calendar;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NostroSummary {
    pub statements: usize,
    pub movements: usize,
    pub missing: usize,
    pub unexpected: usize,
    pub adjusted: usize,
}

/// Opening balances at the start of the window, in each account's own currency.
fn seed_balance(ccy: &str) -> Decimal {
    Decimal::from(match ccy {
        "BHD" => 1_500_000_000,
        "USD" => 600_000_000,
        "EUR" => 250_000_000,
        "GBP" => 120_000_000,
        "SAR" => 500_000_000,
        "AED" => 200_000_000,
        _ => 50_000_000,
    })
}

/// Generate statements and movements for every business day from `from` to `to` inclusive.
/// Movements follow the expected flows of deals that have at least been booked, with a small
/// share missing, a small share off by a few units (fees, rounding), and a few unexpected ones.
pub async fn generate(pool: &PgPool, cal: &Calendar, from: NaiveDate, to: NaiveDate, seed: u64) -> anyhow::Result<NostroSummary> {
    let mut rng = StdRng::seed_from_u64(seed ^ 0x4e05_7120);
    let accounts: Vec<(i16, String)> = sqlx::query_as("SELECT id, ccy::text FROM nostro_accounts ORDER BY id").fetch_all(pool).await?;
    let mut summary = NostroSummary::default();

    for (account_id, ccy) in accounts {
        let ccy = ccy.trim().to_string();
        let mut balance = seed_balance(&ccy);
        let mut day = from;
        let mut seq = 0u32;
        while day <= to {
            if !cal.is_business_day(day) {
                day += Duration::days(1);
                continue;
            }
            let opening = balance;
            let flows: Vec<(String, Decimal, String)> = sqlx::query_as(
                "SELECT deal_ref, sum(amount), max(desk_code) FROM cash_flows_v WHERE ccy = $1 AND flow_date = $2 \
                 AND status IN ('BOOKED', 'CONFIRMED', 'SETTLED') GROUP BY deal_ref ORDER BY deal_ref",
            )
            .bind(&ccy)
            .bind(day)
            .fetch_all(pool)
            .await?;

            let mut day_total = Decimal::ZERO;
            for (deal_ref, expected, _desk) in flows {
                let roll: f64 = rng.random_range(0.0..1.0);
                if roll < 0.02 {
                    summary.missing += 1;
                    continue; // the correspondent never booked it (or booked it on another day)
                }
                let mut amount = expected.round_dp(2);
                if roll < 0.05 {
                    // fee or rounding difference
                    let fee = Decimal::from_f64(rng.random_range(-45.0..-5.0)).unwrap_or_default().round_dp(2);
                    amount += fee;
                    summary.adjusted += 1;
                }
                seq += 1;
                let hint = if rng.random_range(0.0..1.0) < 0.8 { Some(deal_ref.clone()) } else { None };
                let booked_at = at(day, rng.random_range(6..17), rng.random_range(0..60));
                insert_movement(pool, account_id, day, booked_at, amount, &format!("{ccy}-{}-{seq:04}", day.format("%Y%m%d")), hint.as_deref(), "MT942").await?;
                day_total += amount;
                summary.movements += 1;
            }
            // Customer and fee flows treasury never booked.
            let extras = rng.random_range(0..3);
            for _ in 0..extras {
                seq += 1;
                let magnitude = Decimal::from(rng.random_range(5_000..400_000) as i64);
                let amount = if rng.random_range(0..2) == 0 { magnitude } else { -magnitude };
                let booked_at = at(day, rng.random_range(6..17), rng.random_range(0..60));
                let kinds = ["CUSTOMER TRANSFER", "CORRESPONDENT FEES", "INTEREST", "CARD SETTLEMENT"];
                let label = kinds[rng.random_range(0..kinds.len())];
                insert_movement(pool, account_id, day, booked_at, amount, &format!("{ccy}-{}-{seq:04}", day.format("%Y%m%d")), None, label).await?;
                day_total += amount;
                summary.movements += 1;
                summary.unexpected += 1;
            }

            balance = opening + day_total;
            // Intraday statement in the morning, end-of-day statement after close.
            sqlx::query(
                "INSERT INTO nostro_statements (account_id, statement_date, kind, opening_balance, closing_balance, received_at) VALUES ($1, $2, 'MT942', $3, NULL, $4) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(account_id)
            .bind(day)
            .bind(opening)
            .bind(at(day, 6, 5))
            .execute(pool)
            .await?;
            sqlx::query(
                "INSERT INTO nostro_statements (account_id, statement_date, kind, opening_balance, closing_balance, received_at) VALUES ($1, $2, 'MT940', $3, $4, $5) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(account_id)
            .bind(day)
            .bind(opening)
            .bind(balance)
            .bind(at(day, 18, 30))
            .execute(pool)
            .await?;
            summary.statements += 2;
            day += Duration::days(1);
        }
    }
    Ok(summary)
}

fn at(day: NaiveDate, h: u32, m: u32) -> DateTime<Utc> {
    day.and_time(NaiveTime::from_hms_opt(h, m, 0).expect("valid time")).and_utc()
}

#[allow(clippy::too_many_arguments)]
async fn insert_movement(
    pool: &PgPool,
    account_id: i16,
    value_date: NaiveDate,
    booked_at: DateTime<Utc>,
    amount: Decimal,
    reference: &str,
    deal_ref_hint: Option<&str>,
    source: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO nostro_movements (account_id, value_date, booked_at, amount, reference, counterparty, deal_ref_hint, source) \
         VALUES ($1, $2, $3, $4, $5, NULL, $6, $7) ON CONFLICT (account_id, reference) DO NOTHING",
    )
    .bind(account_id)
    .bind(value_date)
    .bind(booked_at)
    .bind(amount)
    .bind(reference)
    .bind(deal_ref_hint)
    .bind(source)
    .execute(pool)
    .await?;
    Ok(())
}
