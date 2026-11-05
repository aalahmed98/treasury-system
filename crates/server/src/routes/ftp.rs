//! The day's FTP curves. ALM drops its workbook, sees what it holds and how it differs from the
//! curves in force, and publishes it; every desk reads the curves in force on a date. The bond
//! portfolio locks each purchase to the curve published on its trade date (see the `pmu` crate).

use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use domain::{
    AsOfQuery, Calendar, CurvePoint, FtpCurrent, FtpCurve, FtpHistory, FtpHistoryPoint, FtpHistoryQuery, FtpPreview, FtpPublication, FtpPublicationSummary,
    FtpPublishRequest,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use crate::auth::AuthUser;
use crate::db;
use crate::error::ApiError;
use crate::ftp_sheet;
use crate::routes::audit::{self, AuditCtx};
use crate::state::AppState;

/// The largest workbook accepted.
pub const MAX_WORKBOOK: usize = 2 * 1024 * 1024;

async fn can_publish(conn: &mut PgConnection) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar("SELECT can_see_alm()").fetch_one(&mut *conn).await?)
}

/// Points of the given publications, grouped into curves.
async fn curves_of(conn: &mut PgConnection, ids: &[i32]) -> Result<BTreeMap<i32, Vec<FtpCurve>>, ApiError> {
    let rows: Vec<(i32, String, String, i32, Decimal)> = sqlx::query_as(
        "SELECT publication_id, ccy::text, tenor, tenor_days, rate FROM ftp_points WHERE publication_id = ANY($1) ORDER BY publication_id, ccy, tenor_days",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut out: BTreeMap<i32, Vec<FtpCurve>> = BTreeMap::new();
    for (id, ccy, tenor, days, rate) in rows {
        let curves = out.entry(id).or_default();
        if curves.last().is_none_or(|c| c.ccy != ccy) {
            curves.push(FtpCurve { ccy: ccy.clone(), points: Vec::new() });
        }
        curves.last_mut().expect("pushed above").points.push(CurvePoint { tenor, tenor_days: days, rate: rate.to_f64().unwrap_or(0.0) });
    }
    Ok(out)
}

type PubRow = (i32, NaiveDate, DateTime<Utc>, Option<String>, Option<String>, Option<String>, i64);

macro_rules! pub_select {
    () => {
        "SELECT p.id, p.as_of, p.published_at, u.display_name, p.file_name, p.note, \
         (SELECT count(*) FROM ftp_publications o WHERE o.as_of = p.as_of AND NOT o.is_current AND o.id < p.id) \
         FROM ftp_publications p LEFT JOIN users u ON u.id = p.published_by"
    };
}

async fn publications(conn: &mut PgConnection, rows: Vec<PubRow>) -> Result<Vec<FtpPublication>, ApiError> {
    let ids: Vec<i32> = rows.iter().map(|r| r.0).collect();
    let mut curves = curves_of(conn, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|(id, as_of, published_at, published_by, file_name, note, superseded)| FtpPublication {
            id,
            as_of,
            published_at,
            published_by,
            file_name,
            note,
            superseded,
            curves: curves.remove(&id).unwrap_or_default(),
        })
        .collect())
}

/// The publication in force on `date` (the latest current one on or before it) and the one before.
pub async fn in_force(conn: &mut PgConnection, date: NaiveDate) -> Result<(Option<FtpPublication>, Option<FtpPublication>), ApiError> {
    let rows: Vec<PubRow> = sqlx::query_as(concat!(pub_select!(), " WHERE p.is_current AND p.as_of <= $1 ORDER BY p.as_of DESC LIMIT 2")).bind(date).fetch_all(&mut *conn).await?;
    let mut pubs = publications(conn, rows).await?.into_iter();
    Ok((pubs.next(), pubs.next()))
}

/// Every current publication up to `to`, by date: what the portfolio locks its purchases against.
pub async fn all_current(conn: &mut PgConnection, to: NaiveDate) -> Result<BTreeMap<NaiveDate, (i32, Vec<FtpCurve>)>, ApiError> {
    let pubs: Vec<(i32, NaiveDate)> = sqlx::query_as("SELECT id, as_of FROM ftp_publications WHERE is_current AND as_of <= $1 ORDER BY as_of").bind(to).fetch_all(&mut *conn).await?;
    let ids: Vec<i32> = pubs.iter().map(|p| p.0).collect();
    let mut curves = curves_of(conn, &ids).await?;
    Ok(pubs.into_iter().map(|(id, d)| (d, (id, curves.remove(&id).unwrap_or_default()))).collect())
}

async fn calendar(conn: &mut PgConnection) -> Result<Calendar, ApiError> {
    let holidays: Vec<NaiveDate> = sqlx::query_scalar("SELECT day FROM calendar_holidays").fetch_all(&mut *conn).await?;
    Ok(Calendar::new(holidays))
}

pub async fn current(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<AsOfQuery>) -> Result<Json<FtpCurrent>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let date = match q.as_of {
        Some(d) => d,
        None => db::business_now(&mut tx).await?.date_naive(),
    };
    let (current, previous) = in_force(&mut tx, date).await?;
    let cal = calendar(&mut tx).await?;
    let age_days = current.as_ref().map(|c| cal.business_days_between(c.as_of, date).max(0));
    type RecentRow = (i32, NaiveDate, DateTime<Utc>, Option<String>, Option<String>, Option<String>, Vec<String>, bool);
    let recent_rows: Vec<RecentRow> = sqlx::query_as(
        "SELECT p.id, p.as_of, p.published_at, u.display_name, p.file_name, p.note, \
                coalesce((SELECT array_agg(DISTINCT x.ccy::text ORDER BY x.ccy::text) FROM ftp_points x WHERE x.publication_id = p.id), '{}'), p.is_current \
         FROM ftp_publications p LEFT JOIN users u ON u.id = p.published_by \
         WHERE p.as_of <= $1 ORDER BY p.as_of DESC, p.id DESC LIMIT 15",
    )
    .bind(date)
    .fetch_all(&mut *tx)
    .await?;
    let can_publish = can_publish(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(FtpCurrent {
        date,
        stale: age_days.is_none_or(|a| a > 0),
        age_days,
        current,
        previous,
        can_publish,
        recent: recent_rows
            .into_iter()
            .map(|(id, as_of, published_at, published_by, file_name, note, currencies, is_current)| FtpPublicationSummary {
                id,
                as_of,
                published_at,
                published_by,
                file_name,
                note,
                currencies,
                is_current,
            })
            .collect(),
    }))
}

pub async fn history(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<FtpHistoryQuery>) -> Result<Json<FtpHistory>, ApiError> {
    let ccy = q.ccy.trim().to_uppercase();
    let tenor = q.tenor.trim().to_uppercase();
    let days = q.days.unwrap_or(90).clamp(5, 800);
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let today = db::business_now(&mut tx).await?.date_naive();
    let rows: Vec<(NaiveDate, Decimal)> = sqlx::query_as(
        "SELECT p.as_of, x.rate FROM ftp_publications p JOIN ftp_points x ON x.publication_id = p.id \
         WHERE p.is_current AND x.ccy = $1 AND x.tenor = $2 AND p.as_of BETWEEN $3 AND $4 ORDER BY p.as_of",
    )
    .bind(&ccy)
    .bind(&tenor)
    .bind(today - Duration::days(days))
    .bind(today)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(FtpHistory { ccy, tenor, points: rows.into_iter().map(|(as_of, r)| FtpHistoryPoint { as_of, rate: r.to_f64().unwrap_or(0.0) }).collect() }))
}

/// Reads a dropped workbook and says what publishing it would do. Nothing is stored.
pub async fn preview(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, body: Bytes) -> Result<Json<FtpPreview>, ApiError> {
    if body.is_empty() {
        return Err(ApiError::BadRequest("the file is empty".into()));
    }
    if body.len() > MAX_WORKBOOK {
        return Err(ApiError::BadRequest("the workbook is larger than 2 MB".into()));
    }
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    if !can_publish(&mut tx).await? {
        return Err(ApiError::Forbidden("only ALM may publish the FTP curves".into()));
    }
    let today = db::business_now(&mut tx).await?.date_naive();
    let parsed = tokio::task::spawn_blocking(move || {
        let sha = hex::encode(Sha256::digest(&body));
        (ftp_sheet::parse(&body), sha)
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    let (p, sha) = parsed;
    let as_of = p.sheet_date.unwrap_or(today);
    // Compare with what is in force the day before, so republishing a day compares with its eve.
    let (before, _) = in_force(&mut tx, as_of - Duration::days(1)).await?;
    tx.commit().await?;
    let changes = before.as_ref().map(|b| ftp_sheet::changes(&b.curves, &p.curves)).unwrap_or_default();
    let mut warnings = p.warnings;
    if let Some(d) = p.sheet_date
        && (d > today + Duration::days(3) || d < today - Duration::days(31))
    {
        warnings.push(domain::SheetIssue { sheet: String::new(), cell: String::new(), message: format!("the workbook is dated {d}, far from today ({today})") });
    }
    Ok(Json(FtpPreview {
        sheet_date: p.sheet_date,
        as_of,
        layout: p.layout,
        curves: p.curves,
        fractions: p.fractions,
        errors: p.errors,
        warnings,
        changes,
        compared_with: before.as_ref().map(|b| b.as_of),
        previous: before.map(|b| b.curves).unwrap_or_default(),
        file_sha256: sha,
    }))
}

fn validate(req: &FtpPublishRequest, today: NaiveDate) -> Result<(), ApiError> {
    let bad = |m: String| Err(ApiError::BadRequest(m));
    if req.curves.is_empty() {
        return bad("there is no curve to publish".into());
    }
    if req.as_of > today + Duration::days(3) || req.as_of < today - Duration::days(31) {
        return bad(format!("curves can be published for {} to {}", today - Duration::days(31), today + Duration::days(3)));
    }
    let mut seen = std::collections::BTreeSet::new();
    for c in &req.curves {
        if !ftp_sheet::is_currency(&c.ccy) {
            return bad(format!("'{}' is not a currency code", c.ccy));
        }
        if !seen.insert(c.ccy.as_str()) {
            return bad(format!("{} appears twice", c.ccy));
        }
        if c.points.is_empty() {
            return bad(format!("{} has no points", c.ccy));
        }
        let mut days = std::collections::BTreeSet::new();
        for p in &c.points {
            if p.tenor_days <= 0 || p.tenor_days > 36_500 || p.tenor.is_empty() || p.tenor.len() > 12 {
                return bad(format!("{} has a tenor that cannot be read", c.ccy));
            }
            if !days.insert(p.tenor_days) {
                return bad(format!("{} has {} twice", c.ccy, p.tenor));
            }
            if !p.rate.is_finite() || !(-5.0..=50.0).contains(&p.rate) {
                return bad(format!("{} {} is outside -5% to 50%", c.ccy, p.tenor));
            }
        }
    }
    if let Some(s) = &req.file_sha256
        && !(s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()))
    {
        return bad("the file fingerprint is malformed".into());
    }
    Ok(())
}

pub async fn publish(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<FtpPublishRequest>,
) -> Result<(StatusCode, Json<FtpPublication>), ApiError> {
    let note = req.note.trim();
    if note.is_empty() || note.chars().count() > 500 || note.chars().any(char::is_control) {
        return Err(ApiError::BadRequest("a note of up to 500 printable characters is required".into()));
    }
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    if !can_publish(&mut tx).await? {
        return Err(ApiError::Forbidden("only ALM may publish the FTP curves".into()));
    }
    let today = db::business_now(&mut tx).await?.date_naive();
    validate(&req, today)?;
    let points: Vec<serde_json::Value> = req
        .curves
        .iter()
        .flat_map(|c| c.points.iter().map(move |p| serde_json::json!({ "ccy": c.ccy, "tenor": p.tenor, "tenor_days": p.tenor_days, "rate": (p.rate * 1e6).round() / 1e6 })))
        .collect();
    let file_name = req.file_name.as_deref().map(|f| f.chars().filter(|c| !c.is_control()).take(255).collect::<String>());
    let id: i32 = sqlx::query_scalar("SELECT ftp_publish($1, $2, $3, $4, $5)")
        .bind(req.as_of)
        .bind(serde_json::Value::Array(points))
        .bind(file_name.as_deref())
        .bind(req.file_sha256.as_deref())
        .bind(note)
        .fetch_one(&mut *tx)
        .await?;
    let rows: Vec<PubRow> = sqlx::query_as(concat!(pub_select!(), " WHERE p.id = $1")).bind(id).fetch_all(&mut *tx).await?;
    let publication = publications(&mut tx, rows).await?.into_iter().next().ok_or_else(|| ApiError::Internal(anyhow::anyhow!("publication {id} vanished")))?;
    tx.commit().await?;
    let ctx = AuditCtx::new(Some(peer), &headers);
    let ccys: Vec<&str> = req.curves.iter().map(|c| c.ccy.as_str()).collect();
    audit::write(
        &state.pool,
        Some(auth.user.id),
        &auth.user.username,
        "ftp_publish",
        &ctx,
        Some(serde_json::json!({ "id": id, "as_of": req.as_of, "currencies": ccys, "file": file_name, "sha256": req.file_sha256, "note": note })),
    )
    .await;
    Ok((StatusCode::CREATED, Json(publication)))
}

/// The workbook to start the next publication from: the curves in force, in the wide layout.
pub async fn template(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<AsOfQuery>) -> Result<Response, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let date = match q.as_of {
        Some(d) => d,
        None => db::business_now(&mut tx).await?.date_naive(),
    };
    let (current, _) = in_force(&mut tx, date).await?;
    tx.commit().await?;
    let curves = current.map(|c| c.curves).unwrap_or_default();
    let bytes = ftp_sheet::template(date, &curves).map_err(|e| ApiError::Internal(anyhow::anyhow!("writing the template: {e}")))?;
    let mut res = bytes.into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"ftp-curves-{date}.xlsx\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(res)
}

/// Publishes a workbook from the command line, as `username`: the same reading, checks and audit
/// as a drop in the browser. Used when ALM's file arrives in a folder rather than through the page.
pub async fn import_file(owner: &sqlx::PgPool, app: &sqlx::PgPool, username: &str, path: &std::path::Path, as_of: Option<NaiveDate>, note: &str) -> anyhow::Result<(i32, NaiveDate, Vec<String>)> {
    let user: uuid::Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(username)
        .fetch_optional(owner)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no user named {username}"))?;
    let bytes = std::fs::read(path).map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
    anyhow::ensure!(bytes.len() <= MAX_WORKBOOK, "the workbook is larger than 2 MB");
    let parsed = ftp_sheet::parse(&bytes);
    if !parsed.errors.is_empty() {
        let lines: Vec<String> = parsed.errors.iter().map(|e| format!("  {} {}: {}", e.sheet, e.cell, e.message)).collect();
        anyhow::bail!("the workbook cannot be published:\n{}", lines.join("\n"));
    }
    let mut tx = db::begin_as(app, user).await?;
    anyhow::ensure!(can_publish(&mut tx).await.map_err(|e| anyhow::anyhow!("{e:?}"))?, "{username} may not publish the FTP curves");
    let today = db::business_now(&mut tx).await?.date_naive();
    let req = FtpPublishRequest {
        as_of: as_of.or(parsed.sheet_date).unwrap_or(today),
        curves: parsed.curves,
        file_name: path.file_name().map(|f| f.to_string_lossy().into_owned()),
        file_sha256: Some(hex::encode(Sha256::digest(&bytes))),
        note: note.trim().to_string(),
    };
    anyhow::ensure!(!req.note.is_empty(), "a note is required (--note)");
    validate(&req, today).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let points: Vec<serde_json::Value> = req
        .curves
        .iter()
        .flat_map(|c| c.points.iter().map(move |p| serde_json::json!({ "ccy": c.ccy, "tenor": p.tenor, "tenor_days": p.tenor_days, "rate": (p.rate * 1e6).round() / 1e6 })))
        .collect();
    let id: i32 = sqlx::query_scalar("SELECT ftp_publish($1, $2, $3, $4, $5)")
        .bind(req.as_of)
        .bind(serde_json::Value::Array(points))
        .bind(req.file_name.as_deref())
        .bind(req.file_sha256.as_deref())
        .bind(&req.note)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    let ccys: Vec<String> = req.curves.iter().map(|c| c.ccy.clone()).collect();
    audit::write(
        app,
        Some(user),
        username,
        "ftp_publish",
        &AuditCtx { client_ip: None, request_id: None },
        Some(serde_json::json!({ "id": id, "as_of": req.as_of, "currencies": ccys, "file": req.file_name, "sha256": req.file_sha256, "note": req.note, "via": "command line" })),
    )
    .await;
    Ok((id, req.as_of, ccys))
}
