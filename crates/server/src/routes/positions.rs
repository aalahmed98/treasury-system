use axum::extract::State;
use axum::{Extension, Json};
use domain::Position;
use rust_decimal::Decimal;

use crate::auth::AuthUser;
use crate::db;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(sqlx::FromRow)]
struct Row {
    desk_id: i16,
    desk_code: String,
    ccy: String,
    net_notional: Decimal,
    deal_count: i64,
}

pub async fn list(State(state): State<AppState>, Extension(auth): Extension<AuthUser>) -> Result<Json<Vec<Position>>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT desk_id, desk_code, ccy, net_notional, deal_count FROM positions_v ORDER BY desk_code, ccy",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| Position { desk_id: r.desk_id, desk_code: r.desk_code, ccy: r.ccy, net_notional: r.net_notional, deal_count: r.deal_count })
            .collect(),
    ))
}
