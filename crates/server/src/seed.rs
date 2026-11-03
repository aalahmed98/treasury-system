//! Mock data for phase 1. Users, credentials and entitlements are written by the owner role
//! (the app role has no such rights); deals are then booked through the app role under each
//! trader's own identity, so the booking policy is exercised by the seed itself.

use chrono::{Datelike, Duration, NaiveDate, Utc, Weekday};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::auth::password;
use crate::db;

pub const DEV_PASSWORD: &str = "password";

struct SeedUser {
    username: &'static str,
    display_name: &'static str,
    home: &'static str,
    access: &'static [(&'static str, bool)],
}

const USERS: &[SeedUser] = &[
    SeedUser { username: "fx1", display_name: "FX Trader One", home: "FX", access: &[("FX", true)] },
    SeedUser { username: "fx2", display_name: "FX Trader Two", home: "FX", access: &[("FX", true)] },
    SeedUser { username: "mm1", display_name: "MM Trader One", home: "MM", access: &[("MM", true)] },
    SeedUser {
        username: "alm1",
        display_name: "ALM Analyst",
        home: "ALM",
        access: &[("FX", false), ("MM", false), ("ALM", false)],
    },
    SeedUser {
        username: "head1",
        display_name: "Head of Treasury",
        home: "HEAD",
        access: &[("FX", false), ("MM", false), ("ALM", false), ("HEAD", false)],
    },
];

const COUNTERPARTIES: &[&str] = &[
    "Gulf International Bank",
    "Arab Banking Corporation",
    "BBK",
    "Ahli United Bank",
    "Al Salam Bank",
    "HSBC Bahrain",
    "Standard Chartered",
    "Citibank N.A.",
    "JPMorgan Chase",
    "Emirates NBD",
    "Al Rajhi Bank",
    "Qatar National Bank",
];

// (ccy1, ccy2, mid)
const FX_PAIRS: &[(&str, &str, f64)] = &[
    ("USD", "BHD", 0.37600),
    ("EUR", "USD", 1.08500),
    ("GBP", "USD", 1.27000),
    ("USD", "SAR", 3.75000),
    ("EUR", "BHD", 0.40800),
    ("USD", "AED", 3.67250),
];
const NOTIONALS: &[i64] = &[100_000, 250_000, 500_000, 1_000_000, 2_000_000, 5_000_000, 10_000_000, 25_000_000];
const FWD_TENORS: &[i64] = &[7, 14, 30, 60, 90, 180];
const MM_CCYS: &[&str] = &["BHD", "USD", "SAR"];
const MM_TENORS: &[i64] = &[1, 7, 14, 30, 90, 180];

pub fn user_id(username: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("treasury-user:{username}").as_bytes())
}

/// `owner` must be the schema owner; `app` the application role.
pub async fn run(owner: &PgPool, app: &PgPool) -> anyhow::Result<()> {
    let desks: HashMap<String, i16> = sqlx::query_as::<_, (i16, String)>("SELECT id, code FROM desks")
        .fetch_all(owner)
        .await?
        .into_iter()
        .map(|(id, code)| (code, id))
        .collect();
    let desk = |code: &str| desks[code];

    let hash = password::hash(DEV_PASSWORD)?;
    for u in USERS {
        let id = user_id(u.username);
        sqlx::query(
            "INSERT INTO users (id, username, display_name, home_desk_id) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (username) DO UPDATE SET display_name = EXCLUDED.display_name",
        )
        .bind(id)
        .bind(u.username)
        .bind(u.display_name)
        .bind(desk(u.home))
        .execute(owner)
        .await?;
        sqlx::query(
            "INSERT INTO user_credentials (user_id, password_hash) VALUES ($1, $2) \
             ON CONFLICT (user_id) DO UPDATE SET password_hash = EXCLUDED.password_hash, updated_at = now()",
        )
        .bind(id)
        .bind(&hash)
        .execute(owner)
        .await?;
        for (code, can_book) in u.access {
            sqlx::query(
                "INSERT INTO user_desk_access (user_id, desk_id, can_book) VALUES ($1, $2, $3) \
                 ON CONFLICT (user_id, desk_id) DO UPDATE SET can_book = EXCLUDED.can_book",
            )
            .bind(id)
            .bind(desk(code))
            .bind(can_book)
            .execute(owner)
            .await?;
        }
    }
    tracing::info!(count = USERS.len(), "users seeded (development password: see README)");

    // head1 sees every desk, so its count is the total.
    let mut tx = db::begin_as(app, user_id("head1")).await?;
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM deals").fetch_one(&mut *tx).await?;
    tx.commit().await?;
    if existing > 0 {
        tracing::info!(existing, "deals already present; skipping deal seed (docker compose down -v to reset)");
        return Ok(());
    }

    let mut rng = StdRng::seed_from_u64(42);
    let today = Utc::now().date_naive();

    let fx_traders = ["fx1", "fx2"];
    let mut fx = Vec::with_capacity(300);
    for i in 0..300 {
        fx.push((fx_traders[i % 2], gen_fx(&mut rng, today)));
    }
    let mut mm = Vec::with_capacity(200);
    for _ in 0..200 {
        mm.push(("mm1", gen_mm(&mut rng, today)));
    }

    for (trader, group) in [("fx1", "FX"), ("fx2", "FX"), ("mm1", "MM")] {
        let rows: Vec<&GenDeal> = fx.iter().chain(mm.iter()).filter(|(t, _)| *t == trader).map(|(_, d)| d).collect();
        let mut tx = db::begin_as(app, user_id(trader)).await?;
        for d in &rows {
            sqlx::query(
                "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, \
                                    trade_date, value_date, maturity_date, counterparty, trader_id, status, \
                                    created_at, updated_at) \
                 VALUES ($1, $2 || '-' || lpad(nextval('deal_ref_seq')::text, 6, '0'), $3, $4, $5, $6, $7, $8, $9, \
                         $10, $11, $12, $13, $14, $15, $16, $16)",
            )
            .bind(Uuid::now_v7())
            .bind(group)
            .bind(desk(group))
            .bind(d.product)
            .bind(d.side)
            .bind(d.ccy1)
            .bind(d.ccy2)
            .bind(d.notional)
            .bind(d.rate)
            .bind(d.trade_date)
            .bind(d.value_date)
            .bind(d.maturity_date)
            .bind(d.counterparty)
            .bind(user_id(trader))
            .bind(d.status)
            .bind(d.trade_date.and_hms_opt(rng.random_range(8..16), rng.random_range(0..60), 0).unwrap().and_utc())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        tracing::info!(trader, count = rows.len(), "deals seeded");
    }
    Ok(())
}

struct GenDeal {
    product: &'static str,
    side: &'static str,
    ccy1: &'static str,
    ccy2: Option<&'static str>,
    notional: Decimal,
    rate: Decimal,
    trade_date: NaiveDate,
    value_date: NaiveDate,
    maturity_date: Option<NaiveDate>,
    counterparty: &'static str,
    status: &'static str,
}

/// Bahrain business days are Sunday to Thursday.
fn business_day(mut d: NaiveDate) -> NaiveDate {
    while matches!(d.weekday(), Weekday::Fri | Weekday::Sat) {
        d += Duration::days(1);
    }
    d
}

fn trade_date(rng: &mut StdRng, today: NaiveDate) -> NaiveDate {
    let mut d = today - Duration::days(rng.random_range(0..30));
    while matches!(d.weekday(), Weekday::Fri | Weekday::Sat) {
        d -= Duration::days(1);
    }
    d
}

fn status_for(rng: &mut StdRng, age_days: i64) -> &'static str {
    let roll: u8 = rng.random_range(0..100);
    match age_days {
        a if a > 10 => if roll < 3 { "CANCELLED" } else { "SETTLED" },
        a if a > 2 => if roll < 80 { "CONFIRMED" } else { "BOOKED" },
        _ => if roll < 60 { "BOOKED" } else { "EXECUTED" },
    }
}

fn pick<'a, T>(rng: &mut StdRng, xs: &'a [T]) -> &'a T {
    &xs[rng.random_range(0..xs.len())]
}

fn dec(v: f64, scale: u32) -> Decimal {
    Decimal::from_f64(v).unwrap_or_default().round_dp(scale)
}

fn gen_fx(rng: &mut StdRng, today: NaiveDate) -> GenDeal {
    let (ccy1, ccy2, mid) = *pick(rng, FX_PAIRS);
    let td = trade_date(rng, today);
    let spot = business_day(td + Duration::days(2));
    let is_fwd = rng.random_range(0..100) < 35;
    let (product, value_date) = if is_fwd {
        ("FX_FWD", business_day(spot + Duration::days(*pick(rng, FWD_TENORS))))
    } else {
        ("FX_SPOT", spot)
    };
    let jitter = 1.0 + rng.random_range(-0.003..0.003);
    let fwd_points = if is_fwd { 1.0 + (value_date - spot).num_days() as f64 * 0.00002 } else { 1.0 };
    GenDeal {
        product,
        side: if rng.random_range(0..2) == 0 { "BUY" } else { "SELL" },
        ccy1,
        ccy2: Some(ccy2),
        notional: Decimal::from(*pick(rng, NOTIONALS) * rng.random_range(1..4)),
        rate: dec(mid * jitter * fwd_points, 5),
        trade_date: td,
        value_date,
        maturity_date: None,
        counterparty: pick(rng, COUNTERPARTIES),
        status: status_for(rng, (today - td).num_days()),
    }
}

fn gen_mm(rng: &mut StdRng, today: NaiveDate) -> GenDeal {
    let td = trade_date(rng, today);
    let tenor = *pick(rng, MM_TENORS);
    let value_date = if tenor == 1 { td } else { business_day(td + Duration::days(2)) };
    let is_depo = rng.random_range(0..2) == 0;
    GenDeal {
        product: if is_depo { "MM_DEPO" } else { "MM_LOAN" },
        side: if is_depo { "LEND" } else { "BORROW" },
        ccy1: pick(rng, MM_CCYS),
        ccy2: None,
        notional: Decimal::from(*pick(rng, NOTIONALS) * rng.random_range(1..4)),
        rate: dec(rng.random_range(4.0..6.5), 4),
        trade_date: td,
        value_date,
        maturity_date: Some(business_day(value_date + Duration::days(tenor))),
        counterparty: pick(rng, COUNTERPARTIES),
        status: status_for(rng, (today - td).num_days()),
    }
}
