//! Liquidity ladder (expected cash per currency per value date, with nostro actuals for entitled
//! users), amendments and cancellation of internal bookings, and deal history.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::HeaderMap;
use axum::{Extension, Json};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use domain::{
    AmendDeal, CashFlow, Deal, FieldChange, FlowsQuery, HistoryEntry, LadderOpening, LadderQuery, LadderResponse, LadderRow, NostroMovement,
};
use rust_decimal::Decimal;
use serde_json::Value;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db::{self, DealRow};
use crate::deal_select;
use crate::error::ApiError;
use crate::routes::audit::{self, AuditCtx};
use crate::state::AppState;

const ONE_DEAL: &str = concat!(deal_select!(), " WHERE d.id = $1");
const PREFERRED_CCYS: &[&str] = &["BHD", "USD", "EUR", "GBP", "SAR", "AED"];

pub async fn ladder(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Query(q): Query<LadderQuery>,
) -> Result<Json<LadderResponse>, ApiError> {
    let today = Utc::now().date_naive();
    let from = q.from.unwrap_or(today - Duration::days(2));
    let days = q.days.unwrap_or(30).clamp(1, 120);
    let to = from + Duration::days(days - 1);

    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let liquidity_access: bool = sqlx::query_scalar("SELECT can_see_liquidity()").fetch_one(&mut *tx).await?;

    let mut currencies: Vec<String> = sqlx::query_scalar("SELECT DISTINCT ccy FROM cash_flows_v").fetch_all(&mut *tx).await?;
    if liquidity_access {
        let accounts: Vec<String> = sqlx::query_scalar("SELECT ccy::text FROM nostro_accounts").fetch_all(&mut *tx).await?;
        currencies.extend(accounts);
    }
    currencies.sort_by_key(|c| (PREFERRED_CCYS.iter().position(|p| p == c).unwrap_or(usize::MAX), c.clone()));
    currencies.dedup();
    let ccy = q
        .ccy
        .map(|c| c.trim().to_uppercase())
        .filter(|c| c.len() == 3)
        .or_else(|| currencies.first().cloned())
        .unwrap_or_else(|| "BHD".to_string());

    // Expected flows per date, split by confirmation and direction.
    #[derive(sqlx::FromRow)]
    struct Agg {
        flow_date: NaiveDate,
        confirmed_in: Decimal,
        confirmed_out: Decimal,
        provisional_in: Decimal,
        provisional_out: Decimal,
        deals: i64,
    }
    let expected: Vec<Agg> = sqlx::query_as(
        "SELECT flow_date, \
                coalesce(sum(amount) FILTER (WHERE confirmed AND amount > 0), 0) AS confirmed_in, \
                coalesce(sum(amount) FILTER (WHERE confirmed AND amount < 0), 0) AS confirmed_out, \
                coalesce(sum(amount) FILTER (WHERE NOT confirmed AND amount > 0), 0) AS provisional_in, \
                coalesce(sum(amount) FILTER (WHERE NOT confirmed AND amount < 0), 0) AS provisional_out, \
                count(DISTINCT deal_id) AS deals \
         FROM cash_flows_v WHERE ccy = $1 AND flow_date BETWEEN $2 AND $3 GROUP BY flow_date ORDER BY flow_date",
    )
    .bind(&ccy)
    .bind(from)
    .bind(to)
    .fetch_all(&mut *tx)
    .await?;

    // Opening balance: the latest statement on or before `from`; closing if it has one.
    let mut opening = None;
    let mut actuals: Vec<(NaiveDate, Decimal, Decimal)> = Vec::new();
    if liquidity_access {
        let st: Option<(Decimal, Option<Decimal>, NaiveDate, String, DateTime<Utc>, String, String)> = sqlx::query_as(
            "SELECT s.opening_balance, s.closing_balance, s.statement_date, s.kind, s.received_at, a.bank, a.account_no \
             FROM nostro_statements s JOIN nostro_accounts a ON a.id = s.account_id \
             WHERE a.ccy = $1 AND s.statement_date <= $2 ORDER BY s.statement_date DESC, s.received_at DESC LIMIT 1",
        )
        .bind(&ccy)
        .bind(from)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((open, close, date, kind, received_at, bank, account_no)) = st {
            // A closing balance (end of day) carries to the next day; an opening (intraday) is the day's start.
            let (balance, effective) = match close {
                Some(c) if date < from => (c, date),
                _ => (open, date),
            };
            let _ = effective;
            opening = Some(LadderOpening { balance, statement_date: date, kind, received_at, bank, account_no });
        }
        actuals = sqlx::query_as(
            "SELECT m.value_date, coalesce(sum(m.amount) FILTER (WHERE m.amount > 0), 0), coalesce(sum(m.amount) FILTER (WHERE m.amount < 0), 0) \
             FROM nostro_movements m JOIN nostro_accounts a ON a.id = m.account_id \
             WHERE a.ccy = $1 AND m.value_date BETWEEN $2 AND $3 GROUP BY m.value_date",
        )
        .bind(&ccy)
        .bind(from)
        .bind(to)
        .fetch_all(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    // Cumulative balances from the opening (or zero) across every calendar day in range.
    let start = opening.as_ref().map(|o| o.balance).unwrap_or(Decimal::ZERO);
    let mut cum_expected = start;
    let mut cum_actual = start;
    let mut rows = Vec::with_capacity(days as usize);
    let mut d = from;
    while d <= to {
        let e = expected.iter().find(|x| x.flow_date == d);
        let (ci, co, pi, po, n) = e
            .map(|x| (x.confirmed_in, x.confirmed_out, x.provisional_in, x.provisional_out, x.deals))
            .unwrap_or((Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, 0));
        let net = (ci + co + pi + po).round_dp(2);
        cum_expected += net;
        let act = actuals.iter().find(|(vd, _, _)| *vd == d).map(|(_, i, o)| (*i, *o));
        let (actual_in, actual_out, actual_net, difference) = match act {
            Some((i, o)) => {
                let an = (i + o).round_dp(2);
                cum_actual += an;
                (Some(i), Some(o), Some(an), Some(an - net))
            }
            None => (None, None, None, None),
        };
        rows.push(LadderRow {
            date: d,
            confirmed_in: ci.round_dp(2),
            confirmed_out: co.round_dp(2),
            provisional_in: pi.round_dp(2),
            provisional_out: po.round_dp(2),
            net_expected: net,
            cumulative_expected: cum_expected.round_dp(2),
            deals: n,
            actual_in,
            actual_out,
            actual_net,
            difference,
        });
        d += Duration::days(1);
    }
    let _ = cum_actual;

    Ok(Json(LadderResponse { ccy, currencies, from, to, liquidity_access, opening, rows, generated_at: Utc::now() }))
}

pub async fn flows(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Query(q): Query<FlowsQuery>,
) -> Result<Json<Vec<CashFlow>>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        deal_id: Uuid,
        deal_ref: String,
        desk_code: String,
        flow_date: NaiveDate,
        ccy: String,
        amount: Decimal,
        leg: String,
        confirmed: bool,
        status: String,
        match_status: String,
    }
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT deal_id, deal_ref, desk_code, flow_date, ccy, amount, leg, confirmed, status, match_status \
         FROM cash_flows_v WHERE ccy = $1 AND flow_date = $2 ORDER BY abs(amount) DESC, deal_ref",
    )
    .bind(q.ccy.trim().to_uppercase())
    .bind(q.date)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| {
                Ok::<_, anyhow::Error>(CashFlow {
                    deal_id: r.deal_id,
                    deal_ref: r.deal_ref,
                    desk_code: r.desk_code,
                    flow_date: r.flow_date,
                    ccy: r.ccy,
                    amount: r.amount.round_dp(2),
                    leg: r.leg,
                    confirmed: r.confirmed,
                    status: r.status.parse().map_err(anyhow::Error::msg)?,
                    match_status: r.match_status.parse().map_err(anyhow::Error::msg)?,
                })
            })
            .collect::<Result<_, _>>()?,
    ))
}

pub async fn movements(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Query(q): Query<FlowsQuery>,
) -> Result<Json<Vec<NostroMovement>>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i64,
        ccy: String,
        value_date: NaiveDate,
        booked_at: DateTime<Utc>,
        amount: Decimal,
        reference: String,
        counterparty: Option<String>,
        deal_ref_hint: Option<String>,
        source: String,
    }
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT m.id, a.ccy::text AS ccy, m.value_date, m.booked_at, m.amount, m.reference, m.counterparty, m.deal_ref_hint, m.source \
         FROM nostro_movements m JOIN nostro_accounts a ON a.id = m.account_id \
         WHERE a.ccy = $1 AND m.value_date = $2 ORDER BY m.booked_at",
    )
    .bind(q.ccy.trim().to_uppercase())
    .bind(q.date)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| NostroMovement {
                id: r.id,
                ccy: r.ccy,
                value_date: r.value_date,
                booked_at: r.booked_at,
                amount: r.amount,
                reference: r.reference,
                counterparty: r.counterparty,
                deal_ref_hint: r.deal_ref_hint,
                source: r.source,
            })
            .collect(),
    ))
}

// ---------------------------------------------------------------------------------------------
// Amend / cancel an internal booking
// ---------------------------------------------------------------------------------------------

pub async fn amend(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(req): Json<AmendDeal>,
) -> Result<Json<Deal>, ApiError> {
    apply_amendment(&state, peer, &headers, &auth, id, req, "deal_amend").await
}

pub async fn cancel(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<Deal>, ApiError> {
    let req = AmendDeal { status: Some(domain::DealStatus::Cancelled), ..Default::default() };
    apply_amendment(&state, peer, &headers, &auth, id, req, "deal_cancel").await
}

async fn apply_amendment(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
    auth: &AuthUser,
    id: Uuid,
    req: AmendDeal,
    action: &str,
) -> Result<Json<Deal>, ApiError> {
    let bad = |m: &str| ApiError::BadRequest(m.to_string());
    if let Some(c) = &req.counterparty {
        let c = c.trim();
        if c.is_empty() || c.chars().count() > 200 || c.chars().any(char::is_control) {
            return Err(bad("counterparty must be 1 to 200 printable characters"));
        }
    }
    if req.notional.is_some_and(|n| n <= Decimal::ZERO) {
        return Err(bad("notional must be positive"));
    }
    if let Some(s) = req.status {
        if s != domain::DealStatus::Cancelled {
            return Err(bad("status can only be changed to CANCELLED here"));
        }
    }
    if req.counterparty.is_none() && req.rate.is_none() && req.notional.is_none() && req.value_date.is_none() && req.maturity_date.is_none() && req.status.is_none() {
        return Err(bad("nothing to change"));
    }

    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let before: Option<DealRow> = sqlx::query_as(ONE_DEAL).bind(id).fetch_optional(&mut *tx).await?;
    let before = before.ok_or(ApiError::NotFound)?;
    if before.origin != "INTERNAL" {
        return Err(ApiError::Forbidden("only internal bookings can be amended here; external deals belong to their source system".into()));
    }
    if let (Some(v), None) = (req.value_date, req.maturity_date) {
        if before.maturity_date.is_some_and(|m| m < v) {
            return Err(bad("value date after maturity date"));
        }
    }
    let updated = sqlx::query(
        "UPDATE deals SET counterparty = coalesce($2, counterparty), rate = coalesce($3, rate), notional = coalesce($4, notional), \
                value_date = coalesce($5, value_date), maturity_date = coalesce($6, maturity_date), status = coalesce($7, status) \
         WHERE id = $1",
    )
    .bind(id)
    .bind(req.counterparty.as_deref().map(str::trim))
    .bind(req.rate)
    .bind(req.notional)
    .bind(req.value_date)
    .bind(req.maturity_date)
    .bind(req.status.map(|s| s.as_str()))
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated == 0 {
        // Visible, but the amendment policy refused it: no booking right on this desk.
        return Err(ApiError::Forbidden("not entitled to amend bookings on this desk".into()));
    }
    let after: DealRow = sqlx::query_as(ONE_DEAL).bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;

    let ctx = AuditCtx::new(Some(peer), headers);
    audit::write(
        &state.pool,
        Some(auth.user.id),
        &auth.user.username,
        action,
        &ctx,
        Some(serde_json::json!({ "deal_id": id, "deal_ref": after.deal_ref, "version": after.version })),
    )
    .await;
    Ok(Json(Deal::try_from(after)?))
}

// ---------------------------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------------------------

const TRACKED: &[&str] = &[
    "product", "side", "ccy1", "ccy2", "notional", "rate", "trade_date", "value_date", "maturity_date", "counterparty", "status", "match_status", "confirmed",
];

pub async fn history(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<HistoryEntry>>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let rows: Vec<(i32, String, DateTime<Utc>, Option<Uuid>, Option<String>, String, Value)> = sqlx::query_as(
        "SELECT version, op, changed_at, changed_by, changed_by_name, changed_by_role, snapshot FROM deal_history_for($1)",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    if rows.is_empty() {
        return Err(ApiError::NotFound);
    }
    let mut out = Vec::with_capacity(rows.len());
    let mut prev: Option<Value> = None;
    for (version, op, changed_at, _by, by_name, role, snapshot) in rows {
        let changes = match &prev {
            None => Vec::new(),
            Some(p) => TRACKED
                .iter()
                .filter_map(|f| {
                    let a = render(p.get(*f));
                    let b = render(snapshot.get(*f));
                    (a != b).then(|| FieldChange { field: (*f).to_string(), from: a, to: b })
                })
                .collect(),
        };
        out.push(HistoryEntry { version, op, changed_at, changed_by: by_name, changed_by_role: role, changes });
        prev = Some(snapshot);
    }
    Ok(Json(out))
}

fn render(v: Option<&Value>) -> Option<String> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.trim().to_string()),
        Some(other) => Some(other.to_string()),
    }
}
