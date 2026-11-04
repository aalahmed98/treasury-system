//! Market data for every desk: the live curves (or the daily closes on a date) and spots.

use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{AsOfQuery, FxSpot, MarketResponse};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::auth::AuthUser;
use crate::db;
use crate::error::ApiError;
use crate::routes::alm::{group_curves, load_curves};
use crate::state::AppState;

pub async fn rates(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<AsOfQuery>) -> Result<Json<MarketResponse>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let today = db::business_now(&mut tx).await?.date_naive();
    let curves = match q.as_of {
        Some(d) => load_curves(&mut tx, d).await?,
        None => {
            let live: Vec<(String, String, String, i32, Decimal, DateTime<Utc>)> =
                sqlx::query_as("SELECT ccy::text, index_code, tenor, tenor_days, rate, as_of FROM market_rates_live ORDER BY ccy, tenor_days").fetch_all(&mut *tx).await?;
            if live.is_empty() {
                load_curves(&mut tx, today).await?
            } else {
                group_curves(live.into_iter().map(|(c, i, t, d, r, at)| (c, i, t, d, r, at.date_naive())).collect())
            }
        }
    };
    let spot_date = q.as_of.unwrap_or(today);
    let rows: Vec<(String, NaiveDate, Decimal)> =
        sqlx::query_as("SELECT DISTINCT ON (pair) pair, as_of, mid FROM fx_spots WHERE as_of <= $1 ORDER BY pair, as_of DESC").bind(spot_date).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(MarketResponse { curves, spots: rows.into_iter().map(|(pair, as_of, mid)| FxSpot { pair, as_of, mid: mid.to_f64().unwrap_or(0.0) }).collect() }))
}
