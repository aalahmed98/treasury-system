//! Row-level security and the definer function for the ALM tables: bank-wide data for the ALM
//! desk and the treasury head only, written by the reconciler's generator, parameters saved only
//! through the function and never touched by the reconciler.

use std::str::FromStr;

use chrono::Utc;
use server::{db, seed};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

async fn pool_for(owner: &PgPool, env: &str) -> PgPool {
    let url = std::env::var(env).unwrap_or_else(|_| panic!("{env} must be set"));
    let creds = PgConnectOptions::from_str(&url).expect("url parses");
    let test_db = owner.connect_options().get_database().expect("test db name").to_string();
    PgPoolOptions::new().max_connections(4).connect_with(creds.database(&test_db)).await.expect("role connects")
}

fn sqlstate(e: &sqlx::Error) -> String {
    e.as_database_error().and_then(|d| d.code().map(|c| c.to_string())).unwrap_or_else(|| format!("not a database error: {e}"))
}

/// One complete snapshot for today with a handful of cohorts, written as the owner.
async fn fixture(owner: &PgPool) -> (PgPool, PgPool) {
    let app = pool_for(owner, "APP_DATABASE_URL").await;
    let recon = pool_for(owner, "RECON_DATABASE_URL").await;
    seed::run(owner, &app).await.unwrap();
    let today = Utc::now().date_naive();
    sqlx::query("INSERT INTO bs_snapshots (as_of, business_date, generated_at, seed, complete) VALUES ($1, $1, now(), 1, true)")
        .bind(today)
        .execute(owner)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO bs_positions (as_of, position_id, side, book, segment, product, ccy, balance, rate, rate_type, hqla_level, lcr_class, maturity) VALUES \
         ($1, 'CASH', 'ASSET', 'CASH', 'CENTRAL_BANK', 'CASH', 'BHD', 1000, 0, 'NONE', 'L1', NULL, NULL), \
         ($1, 'RETAIL', 'LIABILITY', 'DEPOSITS', 'RETAIL', 'CURRENT_ACCOUNT', 'BHD', 5000, 0, 'ADMIN', NULL, 'RET_STABLE', NULL), \
         ($1, 'FI', 'LIABILITY', 'DEPOSITS', 'FI', 'TERM_DEPOSIT', 'BHD', 1000, 4, 'FIXED', NULL, 'FI_WHOLESALE', $1 + 10), \
         ($1, 'PLACE', 'ASSET', 'INTERBANK', 'FI', 'PLACEMENT', 'BHD', 800, 0, 'FIXED', NULL, 'INFLOW_FI', $1 + 5)",
    )
    .bind(today)
    .execute(owner)
    .await
    .unwrap();
    sqlx::query("INSERT INTO market_rates (as_of, ccy, index_code, tenor, tenor_days, rate) VALUES ($1, 'BHD', 'BHIBOR', '1M', 30, 5.5)")
        .bind(today)
        .execute(owner)
        .await
        .unwrap();
    (app, recon)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn alm_rows_are_for_the_alm_desk_and_the_head_only(owner: PgPool) {
    let (app, _recon) = fixture(&owner).await;
    for (user, expect_positions, expect_params) in [("fx1", 0i64, 0i64), ("mm1", 0, 0), ("alm1", 4, 1), ("head1", 4, 1)] {
        let mut tx = db::begin_as(&app, seed::user_id(user)).await.unwrap();
        let positions: i64 = sqlx::query_scalar("SELECT count(*) FROM bs_positions").fetch_one(&mut *tx).await.unwrap();
        let snapshots: i64 = sqlx::query_scalar("SELECT count(*) FROM bs_snapshots").fetch_one(&mut *tx).await.unwrap();
        let params: i64 = sqlx::query_scalar("SELECT count(*) FROM alm_parameters").fetch_one(&mut *tx).await.unwrap();
        let rates: i64 = sqlx::query_scalar("SELECT count(*) FROM market_rates").fetch_one(&mut *tx).await.unwrap();
        assert_eq!(positions, expect_positions, "{user} positions");
        assert_eq!(snapshots, (expect_positions > 0) as i64, "{user} snapshots");
        assert_eq!(params, expect_params, "{user} parameters");
        assert_eq!(rates, 1, "{user} sees market data");
        tx.commit().await.unwrap();
    }
    // No identity at all: nothing.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM bs_positions").fetch_one(&app).await.unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn the_api_role_cannot_write_alm_tables(owner: PgPool) {
    let (app, _recon) = fixture(&owner).await;
    let today = Utc::now().date_naive();
    let denied = [
        "INSERT INTO bs_snapshots (as_of, business_date, generated_at, seed) VALUES ('2030-01-01', '2030-01-01', now(), 1)",
        "INSERT INTO bs_positions (as_of, position_id, side, book, segment, product, ccy, balance, rate, rate_type) VALUES ($1, 'X', 'ASSET', 'B', 'S', 'P', 'BHD', 1, 0, 'NONE')",
        "UPDATE bs_positions SET balance = 0",
        "DELETE FROM bs_snapshots",
        "INSERT INTO alm_parameters (params, is_active) VALUES ('{}', false)",
        "UPDATE alm_parameters SET is_active = false",
        "DELETE FROM alm_parameters",
        "INSERT INTO market_rates (as_of, ccy, index_code, tenor, tenor_days, rate) VALUES ($1, 'USD', 'SOFR', '1M', 30, 4)",
        "INSERT INTO alm_period (period, book, side, ccy, kind, nii, avg_balance) VALUES ($1, 'B', 'ASSET', 'BHD', 'ACTUAL', 1, 1)",
    ];
    for sql in denied {
        let mut tx = db::begin_as(&app, seed::user_id("head1")).await.unwrap();
        let q = if sql.contains("$1") { sqlx::query(sql).bind(today) } else { sqlx::query(sql) };
        let err = q.execute(&mut *tx).await.expect_err(sql);
        assert_eq!(sqlstate(&err), "42501", "{sql}: {err}");
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn the_reconciler_writes_the_book_but_never_the_parameters(owner: PgPool) {
    let (_app, recon) = fixture(&owner).await;
    sqlx::query("INSERT INTO bs_snapshots (as_of, business_date, generated_at, seed, complete) VALUES ('2030-01-01', '2030-01-01', now(), 1, false)")
        .execute(&recon)
        .await
        .expect("the reconciler writes snapshots");
    sqlx::query("DELETE FROM bs_snapshots WHERE as_of = '2030-01-01'").execute(&recon).await.expect("and removes its own");
    for sql in ["SELECT count(*) FROM alm_parameters", "INSERT INTO alm_parameters (params) VALUES ('{}')", "SELECT alm_parameters_save('{}', 'x')"] {
        let err = sqlx::query(sql).execute(&recon).await.expect_err(sql);
        assert_eq!(sqlstate(&err), "42501", "{sql}: {err}");
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn saving_parameters_needs_the_alm_desk_and_a_valid_document(owner: PgPool) {
    let (app, _recon) = fixture(&owner).await;
    let valid = serde_json::to_value(domain::AlmParams::default()).unwrap();

    let mut tx = db::begin_as(&app, seed::user_id("fx1")).await.unwrap();
    let err = sqlx::query_scalar::<_, i32>("SELECT alm_parameters_save($1, $2)").bind(&valid).bind("tighter").fetch_one(&mut *tx).await.expect_err("fx1");
    assert_eq!(sqlstate(&err), "42501");
    drop(tx);

    for (doc, note, what) in [
        (serde_json::json!({"lcr": {}}), "x", "missing sections"),
        (serde_json::json!({"lcr": {"outflow_rates": {"RET_STABLE": 150}}, "nsfr": {}, "gap": {}, "stress": {}, "nii": {}, "funding": {}}), "x", "rate above 100"),
        (valid.clone(), "   ", "empty note"),
    ] {
        let mut tx = db::begin_as(&app, seed::user_id("alm1")).await.unwrap();
        let err = sqlx::query_scalar::<_, i32>("SELECT alm_parameters_save($1, $2)").bind(&doc).bind(note).fetch_one(&mut *tx).await.expect_err(what);
        assert_eq!(sqlstate(&err), "P0001", "{what}: {err}");
    }

    let mut tx = db::begin_as(&app, seed::user_id("alm1")).await.unwrap();
    let v: i32 = sqlx::query_scalar("SELECT alm_parameters_save($1, $2)").bind(&valid).bind("internal minimum raised").fetch_one(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(v, 2);
    let (active, by, note): (i32, Option<uuid::Uuid>, Option<String>) =
        sqlx::query_as("SELECT version, created_by, note FROM alm_parameters WHERE is_active").fetch_one(&owner).await.unwrap();
    assert_eq!(active, 2);
    assert_eq!(by, Some(seed::user_id("alm1")));
    assert_eq!(note.as_deref(), Some("internal minimum raised"));
    let actives: i64 = sqlx::query_scalar("SELECT count(*) FROM alm_parameters WHERE is_active").fetch_one(&owner).await.unwrap();
    assert_eq!(actives, 1);
}
