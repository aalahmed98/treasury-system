//! Database access helpers. The single most important function is `begin_as`: every query that
//! touches entitled data runs inside a transaction whose `app.user_id` setting names the caller.

use chrono::{DateTime, NaiveDate, Utc};
use domain::Deal;
use rust_decimal::Decimal;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub async fn connect(url: &str) -> sqlx::Result<PgPool> {
    PgPoolOptions::new().max_connections(16).connect(url).await
}

/// Open a transaction with the caller's identity set for the duration of that transaction only.
/// `set_config(..., true)` is transaction-local, so a connection returning to the pool after
/// commit or rollback carries no identity.
pub async fn begin_as(pool: &PgPool, user_id: Uuid) -> sqlx::Result<Transaction<'static, Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

/// The deal projection every read path shares.
#[macro_export]
macro_rules! deal_select {
    () => {
        "SELECT d.id, d.deal_ref, d.desk_id, k.code AS desk_code, d.product, d.side, \
                d.ccy1::text AS ccy1, d.ccy2::text AS ccy2, d.notional, d.rate, \
                d.trade_date, d.value_date, d.maturity_date, d.counterparty, \
                d.trader_id, u.display_name AS trader_name, d.status, d.version, \
                d.created_at, d.updated_at \
         FROM deals d \
         JOIN desks k ON k.id = d.desk_id \
         JOIN users u ON u.id = d.trader_id"
    };
}

#[derive(Debug, sqlx::FromRow)]
pub struct DealRow {
    pub id: Uuid,
    pub deal_ref: String,
    pub desk_id: i16,
    pub desk_code: String,
    pub product: String,
    pub side: String,
    pub ccy1: String,
    pub ccy2: Option<String>,
    pub notional: Decimal,
    pub rate: Decimal,
    pub trade_date: NaiveDate,
    pub value_date: NaiveDate,
    pub maturity_date: Option<NaiveDate>,
    pub counterparty: String,
    pub trader_id: Uuid,
    pub trader_name: String,
    pub status: String,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TryFrom<DealRow> for Deal {
    type Error = anyhow::Error;

    fn try_from(r: DealRow) -> Result<Self, Self::Error> {
        Ok(Deal {
            id: r.id,
            deal_ref: r.deal_ref,
            desk_id: r.desk_id,
            desk_code: r.desk_code,
            product: r.product.parse().map_err(anyhow::Error::msg)?,
            side: r.side.parse().map_err(anyhow::Error::msg)?,
            ccy1: r.ccy1,
            ccy2: r.ccy2,
            notional: r.notional,
            rate: r.rate,
            trade_date: r.trade_date,
            value_date: r.value_date,
            maturity_date: r.maturity_date,
            counterparty: r.counterparty,
            trader_id: r.trader_id,
            trader_name: r.trader_name,
            status: r.status.parse().map_err(anyhow::Error::msg)?,
            version: r.version,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    }
}

pub fn rows_to_deals(rows: Vec<DealRow>) -> anyhow::Result<Vec<Deal>> {
    rows.into_iter().map(Deal::try_from).collect()
}
