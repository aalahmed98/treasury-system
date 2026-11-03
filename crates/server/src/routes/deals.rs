use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use domain::{Deal, DealsQuery, DealsResponse, NewDeal};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db::{self, DealRow, rows_to_deals};
use crate::deal_select;
use crate::error::ApiError;
use crate::state::AppState;

const LIST_SQL: &str = concat!(
    deal_select!(),
    " WHERE ($1::timestamptz IS NULL OR d.updated_at > $1) \
      ORDER BY d.updated_at DESC, d.id DESC LIMIT $2"
);
const ONE_SQL: &str = concat!(deal_select!(), " WHERE d.id = $1");
const MAX_COUNTERPARTY_LEN: usize = 200;

pub async fn list(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Query(q): Query<DealsQuery>,
) -> Result<Json<DealsResponse>, ApiError> {
    let limit = q.limit.unwrap_or(5000).clamp(1, 20_000);
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let as_of: DateTime<Utc> = sqlx::query_scalar("SELECT now()").fetch_one(&mut *tx).await?;
    let rows: Vec<DealRow> = sqlx::query_as(LIST_SQL)
        .bind(q.since)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(DealsResponse { deals: rows_to_deals(rows)?, as_of }))
}

pub async fn get_one(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<Deal>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let row: Option<DealRow> = sqlx::query_as(ONE_SQL).bind(id).fetch_optional(&mut *tx).await?;
    tx.commit().await?;
    // RLS hides rows the caller may not see, so "not entitled" and "does not exist" look identical.
    let row = row.ok_or(ApiError::NotFound)?;
    Ok(Json(Deal::try_from(row)?))
}

pub async fn create(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(new): Json<NewDeal>,
) -> Result<(StatusCode, Json<Deal>), ApiError> {
    let new = validate(new)?;
    // A desk the user cannot even read is reported as forbidden before touching the database.
    let desk = auth.desk(new.desk_id).ok_or_else(|| ApiError::Forbidden("not entitled to that desk".into()))?;
    let desk_code = desk.code.clone();

    let id = Uuid::now_v7();
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    // The INSERT policy decides (can_book on the desk, trader must be the caller);
    // a violation surfaces as SQLSTATE 42501 -> 403.
    sqlx::query(
        "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, \
                            trade_date, value_date, maturity_date, counterparty, trader_id, status) \
         VALUES ($1, $2 || '-' || lpad(nextval('deal_ref_seq')::text, 6, '0'), $3, $4, $5, $6, $7, $8, $9, \
                 $10, $11, $12, $13, $14, 'BOOKED')",
    )
    .bind(id)
    .bind(&desk_code)
    .bind(new.desk_id)
    .bind(new.product.as_str())
    .bind(new.side.as_str())
    .bind(&new.ccy1)
    .bind(&new.ccy2)
    .bind(new.notional)
    .bind(new.rate)
    .bind(new.trade_date)
    .bind(new.value_date)
    .bind(new.maturity_date)
    .bind(&new.counterparty)
    .bind(auth.user.id)
    .execute(&mut *tx)
    .await?;
    let row: DealRow = sqlx::query_as(ONE_SQL).bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(Deal::try_from(row)?)))
}

fn validate(mut d: NewDeal) -> Result<NewDeal, ApiError> {
    let bad = |m: &str| Err(ApiError::BadRequest(m.to_string()));
    if !is_ccy(&d.ccy1) {
        return bad("ccy1 must be a 3-letter ISO code");
    }
    match (d.product.is_fx(), &d.ccy2) {
        (true, Some(c)) if !is_ccy(c) => return bad("ccy2 must be a 3-letter ISO code"),
        (true, None) => return bad("FX deals need ccy2"),
        (false, Some(_)) => return bad("money market deals take a single currency"),
        _ => {}
    }
    if d.notional <= rust_decimal::Decimal::ZERO {
        return bad("notional must be positive");
    }
    if d.value_date < d.trade_date {
        return bad("value date before trade date");
    }
    if d.maturity_date.is_some_and(|m| m < d.value_date) {
        return bad("maturity date before value date");
    }
    d.counterparty = d.counterparty.trim().to_string();
    if d.counterparty.is_empty() || d.counterparty.chars().count() > MAX_COUNTERPARTY_LEN {
        return bad("counterparty is required (at most 200 characters)");
    }
    if d.counterparty.chars().any(char::is_control) {
        return bad("counterparty contains control characters");
    }
    Ok(d)
}

fn is_ccy(s: &str) -> bool {
    s.len() == 3 && s.bytes().all(|b| b.is_ascii_uppercase())
}
