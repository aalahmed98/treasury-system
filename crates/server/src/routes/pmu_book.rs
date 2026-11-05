//! The PMU desk's tickets: find a bond by ISIN (or add one), price a purchase or a sale before
//! booking it, book it, and book, close or roll repos and reverse repos. The server recomputes
//! every preview at booking and books exactly what it shows; the definer functions in 0009 decide
//! who may book and whether the face is free.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use chrono::{Duration, NaiveDate};
use domain::{
    BondRateType, BondSearchHit, BondSearchQuery, Booked, Calendar, KondorAttach, NewBond, RepoClose, RepoPreview, RepoRequest, RepoRoll, TicketPreview,
    TicketRequest,
};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db;
use crate::error::ApiError;
use crate::ftp_sheet::is_currency;
use crate::routes::audit::{self, AuditCtx};
use crate::routes::pmu::context;
use crate::state::{AppState, PmuData};

use pmu::booking::short as short_;

fn bad(m: impl Into<String>) -> ApiError {
    ApiError::BadRequest(m.into())
}

async fn require_booker(conn: &mut PgConnection) -> Result<(), ApiError> {
    let ok: bool = sqlx::query_scalar("SELECT pmu_can_book()").fetch_one(&mut *conn).await?;
    if ok { Ok(()) } else { Err(ApiError::Forbidden("not entitled to book on the PMU desk".into())) }
}

async fn calendar(conn: &mut PgConnection) -> Result<Calendar, ApiError> {
    let holidays: Vec<NaiveDate> = sqlx::query_scalar("SELECT day FROM calendar_holidays").fetch_all(&mut *conn).await?;
    Ok(Calendar::new(holidays))
}

/// ISO 6166: two letters, nine letters or digits, a check digit (Luhn over the letters as numbers).
pub fn isin_valid(isin: &str) -> bool {
    let s = isin.trim().to_uppercase();
    if s.len() != 12 || !s[..2].chars().all(|c| c.is_ascii_uppercase()) || !s.chars().all(|c| c.is_ascii_alphanumeric()) || !s[11..].chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let digits: String = s.chars().map(|c| if c.is_ascii_digit() { c.to_string() } else { (c as u32 - 'A' as u32 + 10).to_string() }).collect();
    let sum: u32 = digits
        .chars()
        .rev()
        .enumerate()
        .map(|(i, c)| {
            let d = c.to_digit(10).unwrap_or(0);
            if i % 2 == 1 {
                let x = d * 2;
                x / 10 + x % 10
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

// ---------------------------------------------------------------------------------------------
// Finding and adding bonds
// ---------------------------------------------------------------------------------------------

pub async fn search(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<BondSearchQuery>) -> Result<Json<Vec<BondSearchHit>>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let (on, _, data) = context(&state, &mut tx).await?;
    tx.commit().await?;
    let needle = q.q.unwrap_or_default().trim().to_lowercase();
    let (p, m) = (&data.portfolio, &data.market);
    let mut hits: Vec<BondSearchHit> = p
        .bonds
        .values()
        .filter(|b| needle.is_empty() || b.isin.to_lowercase().contains(&needle) || b.name.to_lowercase().contains(&needle) || b.issuer.to_lowercase().contains(&needle))
        .filter(|b| b.maturity > on)
        .map(|b| {
            let mut holdings: Vec<(String, f64, f64, f64)> = Vec::new();
            for bk in &p.books {
                let lots: Vec<_> = p.lots.iter().filter(|l| l.isin == b.isin && l.book == bk.code && pmu::eval::held_on(l, b, on)).collect();
                if lots.is_empty() {
                    continue;
                }
                let held: f64 = lots.iter().map(|l| l.face_on(on)).sum();
                let pledged: f64 = lots.iter().map(|l| pmu::eval::pledged_face(p, l, on)).sum();
                holdings.push((bk.code.clone(), held, pledged, (held - pledged).max(0.0)));
            }
            let px = m.price(&b.isin, on);
            BondSearchHit { bond: b.clone(), market_price: px.map(|x| x.0), market_ytm: px.map(|x| x.1), holdings }
        })
        .collect();
    // Held bonds first, then by name.
    hits.sort_by(|a, b| b.holdings.is_empty().cmp(&a.holdings.is_empty()).reverse().then_with(|| a.bond.name.cmp(&b.bond.name)));
    hits.truncate(30);
    Ok(Json(hits))
}

pub async fn add_bond(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Json(mut b): Json<NewBond>,
) -> Result<(StatusCode, Json<Booked>), ApiError> {
    b.isin = b.isin.trim().to_uppercase();
    b.ccy = b.ccy.trim().to_uppercase();
    if !isin_valid(&b.isin) {
        return Err(bad(format!("{} is not a valid ISIN (two letters, nine characters and a check digit)", b.isin)));
    }
    if !is_currency(&b.ccy) {
        return Err(bad(format!("'{}' is not a currency code", b.ccy)));
    }
    if b.name.trim().is_empty() || b.issuer.trim().is_empty() || b.name.len() > 120 || b.issuer.len() > 120 {
        return Err(bad("the bond needs a name and an issuer"));
    }
    if !["SOVEREIGN", "GCC_SOVEREIGN", "SUPRANATIONAL", "BANK", "CORPORATE", "SUKUK"].contains(&b.issuer_type.as_str()) {
        return Err(bad("unknown issuer type"));
    }
    if ![1, 2, 4, 12].contains(&b.coupon_freq) {
        return Err(bad("coupons are paid 1, 2, 4 or 12 times a year"));
    }
    if b.maturity <= b.issue_date {
        return Err(bad("the bond must mature after it is issued"));
    }
    match b.rate_type {
        BondRateType::Fixed if !(0.0..=30.0).contains(&b.coupon) => return Err(bad("the coupon must be 0% to 30%")),
        BondRateType::Float if b.float_index.as_deref().is_none_or(|i| i.trim().is_empty()) => return Err(bad("a floating bond names its index")),
        _ => {}
    }
    if b.hqla_level.as_deref().is_some_and(|h| !h.is_empty() && !["L1", "L2A", "L2B"].contains(&h)) {
        return Err(bad("HQLA level is L1, L2A or L2B"));
    }
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    require_booker(&mut tx).await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM bonds WHERE isin = $1)").bind(&b.isin).fetch_one(&mut *tx).await?;
    if exists {
        return Err(bad(format!("{} is already set up", b.isin)));
    }
    let body = serde_json::json!({
        "isin": b.isin, "name": b.name.trim(), "issuer": b.issuer.trim(), "issuer_type": b.issuer_type, "ccy": b.ccy,
        "coupon": if b.rate_type == BondRateType::Float { 0.0 } else { b.coupon }, "coupon_freq": b.coupon_freq, "day_count": b.day_count.as_str(),
        "issue_date": b.issue_date, "maturity": b.maturity, "rate_type": b.rate_type.as_str(), "float_index": b.float_index, "float_spread": b.float_spread,
        "hqla_level": b.hqla_level,
    });
    let isin: String = sqlx::query_scalar("SELECT pmu_bond_add($1)").bind(&body).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    audit::write(&state.pool, Some(auth.user.id), &auth.user.username, "pmu_bond_add", &AuditCtx::new(Some(peer), &headers), Some(body)).await;
    Ok((StatusCode::CREATED, Json(Booked { id: isin.clone(), message: format!("{isin} set up") })))
}

// ---------------------------------------------------------------------------------------------
// Purchases and sales
// ---------------------------------------------------------------------------------------------

/// The preview the ticket shows, and the one a booking is checked against.
fn preview_of(data: &PmuData, cal: &Calendar, on: NaiveDate, req: &TicketRequest) -> Result<TicketPreview, ApiError> {
    let (p, m) = (&data.portfolio, &data.market);
    let b = p.bonds.get(&req.isin.trim().to_uppercase()).ok_or_else(|| bad(format!("no bond with ISIN {}; add it first", req.isin.trim())))?;
    let sell = match req.side.as_str() {
        "BUY" => false,
        "SELL" => true,
        _ => return Err(bad("side is BUY or SELL")),
    };
    if !(req.face.is_finite() && req.face > 0.0) {
        return Err(bad("the face must be more than zero"));
    }
    let trade = req.trade_date.unwrap_or(on);
    let settle = req.settle_date.unwrap_or_else(|| cal.add_business_days(trade, 2));
    if settle < trade {
        return Err(bad("settlement is on or after the trade date"));
    }
    if trade > on + Duration::days(5) || trade < on - Duration::days(31) {
        return Err(bad(format!("trades can be booked for {} to {}", on - Duration::days(31), on + Duration::days(5))));
    }
    let price = match (req.clean_price, req.ytm) {
        (Some(c), _) if c.is_finite() && c > 0.0 && c < 1000.0 => pmu::booking::Price::Clean(c),
        (None, Some(y)) if y.is_finite() && (-5.0..60.0).contains(&y) => pmu::booking::Price::Yield(y),
        (None, None) => pmu::booking::Price::Clean(m.price(&b.isin, on).map(|x| x.0).unwrap_or(100.0)),
        _ => return Err(bad("give a clean price or a yield")),
    };
    let quote = pmu::booking::quote_bond(m, b, req.face, trade, settle, price);
    let mut warnings = Vec::new();
    let mut blocking = Vec::new();
    if settle >= b.maturity {
        blocking.push(format!("the bond matures on {}, on or before settlement", b.maturity));
    }
    if quote.off_market {
        warnings.push(format!(
            "{:+.0} bp from the market ({} on {})",
            quote.off_market_bp.unwrap_or(0.0),
            match (b.rate_type, quote.market_ytm, quote.market_price) {
                (BondRateType::Fixed, Some(y), _) => format!("yield {y:.3}%"),
                (_, _, Some(c)) => format!("price {c:.3}"),
                _ => "the last price".into(),
            },
            quote.market_as_of.map(|d| d.to_string()).unwrap_or_default()
        ));
    }
    let (mut legs, mut candidates, mut realised) = (Vec::new(), Vec::new(), 0.0);
    let mut free_total = 0.0;
    if sell {
        let picks = req.lots.as_deref();
        let (l, c, short) = pmu::booking::allocate(m, p, &b.isin, req.book.as_deref(), settle, quote.clean_price, req.face, picks);
        free_total = c.iter().map(|x| x.free).sum();
        if short > 0.005 {
            blocking.push(format!("only {} of face is free to sell on {settle}; {} short", short_(free_total), short_(short)));
        }
        for leg in &l {
            if let Some(cand) = c.iter().find(|x| x.lot_id == leg.lot_id)
                && leg.face > cand.free + 0.005
            {
                blocking.push(format!("the lot bought on {} has only {} free", cand.trade_date, short_(cand.free)));
            }
        }
        if let Some(picks) = picks {
            let total: f64 = picks.iter().map(|x| x.face).sum();
            if (total - req.face).abs() > 0.005 {
                blocking.push(format!("the lots chosen add up to {}, not {}", short_(total), short_(req.face)));
            }
        }
        realised = l.iter().map(|x| x.realised_bhd).sum();
        legs = l;
        candidates = c;
    } else {
        let book = req.book.as_deref().ok_or_else(|| bad("choose the book to buy into"))?;
        match p.books.iter().find(|x| x.code == book) {
            None => blocking.push(format!("no book {book}")),
            Some(bk) if bk.ccy != b.ccy && bk.code != "HTM" => warnings.push(format!("a {} bond in the {} book", b.ccy, bk.code)),
            _ => {}
        }
        if quote.ftp_rate.is_none() {
            warnings.push(format!("no FTP curve for {}: the purchase will carry no funding charge", b.ccy));
        }
    }
    Ok(TicketPreview { quote, legs, candidates, free_total, realised_bhd: realised, warnings, blocking })
}

pub async fn preview(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<TicketRequest>) -> Result<Json<TicketPreview>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let (on, _, data) = context(&state, &mut tx).await?;
    let cal = calendar(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(preview_of(&data, &cal, on, &req)?))
}

async fn checked(state: &AppState, auth: &AuthUser, req: &TicketRequest) -> Result<(sqlx::Transaction<'static, sqlx::Postgres>, TicketPreview), ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    require_booker(&mut tx).await?;
    let (on, _, data) = context(state, &mut tx).await?;
    let cal = calendar(&mut tx).await?;
    let pv = preview_of(&data, &cal, on, req)?;
    if let Some(b) = pv.blocking.first() {
        return Err(bad(b.clone()));
    }
    if pv.quote.off_market && !req.confirm_off_market {
        return Err(bad("the price is off the market; confirm it to book"));
    }
    if req.kondor_id.as_deref().is_some_and(|k| k.trim().len() > 40) {
        return Err(bad("a Kondor id is at most 40 characters"));
    }
    Ok((tx, pv))
}

pub async fn buy(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<TicketRequest>,
) -> Result<(StatusCode, Json<Booked>), ApiError> {
    if req.side != "BUY" {
        return Err(bad("this books purchases"));
    }
    let (mut tx, pv) = checked(&state, &auth, &req).await?;
    let q = &pv.quote;
    let pub_id = {
        let (_, _, data) = context(&state, &mut tx).await?;
        data.market.ftp.rate(&q.ccy, q.trade_date, (data.portfolio.bonds[&q.isin].maturity - q.settle_date).num_days()).map(|x| x.1)
    };
    let id = Uuid::now_v7();
    let book = req.book.clone().unwrap_or_default();
    sqlx::query("SELECT pmu_buy($1, $2, $3, $4, $5, $6::numeric, $7::numeric, $8::numeric, $9, $10::numeric, $11::numeric, $12)")
        .bind(id)
        .bind(&book)
        .bind(&q.isin)
        .bind(q.trade_date)
        .bind(q.settle_date)
        .bind(format!("{:.2}", q.face))
        .bind(format!("{:.6}", q.clean_price))
        .bind(format!("{:.2}", q.accrued))
        .bind(req.kondor_id.as_deref())
        .bind(format!("{:.8}", q.eir))
        .bind(q.ftp_rate.map(|r| format!("{r:.6}")))
        .bind(pub_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let detail = serde_json::json!({ "lot": id, "isin": q.isin, "book": book, "face": q.face, "clean": q.clean_price, "settle": q.settle_date, "eir": q.eir, "ftp": q.ftp_rate, "kondor": req.kondor_id });
    audit::write(&state.pool, Some(auth.user.id), &auth.user.username, "pmu_buy", &AuditCtx::new(Some(peer), &headers), Some(detail)).await;
    Ok((StatusCode::CREATED, Json(Booked { id: id.to_string(), message: format!("Bought {:.0} {} of {} at {:.3}", q.face, q.ccy, q.isin, q.clean_price) })))
}

pub async fn sell(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<TicketRequest>,
) -> Result<(StatusCode, Json<Booked>), ApiError> {
    if req.side != "SELL" {
        return Err(bad("this books sales"));
    }
    let (mut tx, pv) = checked(&state, &auth, &req).await?;
    let q = &pv.quote;
    let sale_ref: String = sqlx::query_scalar("SELECT 'PMU-S-' || lpad(nextval('pmu_ref_seq')::text, 6, '0')").fetch_one(&mut *tx).await?;
    let alloc: Vec<serde_json::Value> = pv.legs.iter().map(|l| serde_json::json!({ "lot_id": l.lot_id, "face": (l.face * 100.0).round() / 100.0, "accrued": (l.accrued * 100.0).round() / 100.0 })).collect();
    sqlx::query("SELECT pmu_sell($1, $2, $3, $4::numeric, $5, $6)")
        .bind(&sale_ref)
        .bind(q.trade_date)
        .bind(q.settle_date)
        .bind(format!("{:.6}", q.clean_price))
        .bind(req.kondor_id.as_deref())
        .bind(serde_json::Value::Array(alloc.clone()))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let detail = serde_json::json!({ "sale": sale_ref, "isin": q.isin, "face": q.face, "clean": q.clean_price, "settle": q.settle_date, "lots": alloc, "realised_bhd": pv.realised_bhd });
    audit::write(&state.pool, Some(auth.user.id), &auth.user.username, "pmu_sell", &AuditCtx::new(Some(peer), &headers), Some(detail)).await;
    Ok((StatusCode::CREATED, Json(Booked { id: sale_ref.clone(), message: format!("Sold {:.0} {} of {} at {:.3} ({sale_ref})", q.face, q.ccy, q.isin, q.clean_price) })))
}

// ---------------------------------------------------------------------------------------------
// Repos
// ---------------------------------------------------------------------------------------------

fn validate_repo(r: &RepoRequest) -> Result<(), ApiError> {
    if !["REPO", "REVERSE"].contains(&r.direction.as_str()) {
        return Err(bad("direction is REPO or REVERSE"));
    }
    if r.counterparty.trim().is_empty() || r.counterparty.len() > 120 {
        return Err(bad("name the counterparty"));
    }
    if !is_currency(&r.ccy) {
        return Err(bad(format!("'{}' is not a currency code", r.ccy)));
    }
    if !(r.rate.is_finite() && (-5.0..=50.0).contains(&r.rate)) {
        return Err(bad("the rate must be -5% to 50%"));
    }
    if !(0.0..=100.0).contains(&r.haircut_pct) {
        return Err(bad("the haircut is 0% to 100%"));
    }
    if r.cash.is_some_and(|c| !(c.is_finite() && c > 0.0)) {
        return Err(bad("the cash must be more than zero"));
    }
    if r.direction == "REVERSE" && (r.collateral_isin.as_deref().is_none_or(|i| i.trim().is_empty()) || r.collateral_face.is_none_or(|f| f <= 0.0) || r.cash.is_none()) {
        return Err(bad("a reverse repo names the bond received, its face and the cash lent"));
    }
    Ok(())
}

fn repo_preview_of(data: &PmuData, on: NaiveDate, r: &RepoRequest, except: Option<Uuid>) -> RepoPreview {
    let mut pv = pmu::booking::repo_preview(&data.market, &data.portfolio, &r.direction, &r.counterparty, &r.ccy, r.start, r.end, r.rate, r.haircut_pct, r.cash, &r.collateral, except);
    if r.start < on - Duration::days(31) || r.start > on + Duration::days(31) {
        pv.blocking.push(format!("a repo can start from {} to {}", on - Duration::days(31), on + Duration::days(31)));
    }
    pv
}

pub async fn repo_preview(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(r): Json<RepoRequest>) -> Result<Json<RepoPreview>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let (on, _, data) = context(&state, &mut tx).await?;
    tx.commit().await?;
    Ok(Json(repo_preview_of(&data, on, &r, None)))
}

pub async fn repo_book(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Json(r): Json<RepoRequest>,
) -> Result<(StatusCode, Json<Booked>), ApiError> {
    validate_repo(&r)?;
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    require_booker(&mut tx).await?;
    let (on, _, data) = context(&state, &mut tx).await?;
    let pv = repo_preview_of(&data, on, &r, None);
    if let Some(b) = pv.blocking.first() {
        return Err(bad(b.clone()));
    }
    let id = Uuid::now_v7();
    let coll: Vec<serde_json::Value> = r.collateral.iter().map(|c| serde_json::json!({ "lot_id": c.lot_id, "face": c.face })).collect();
    sqlx::query("SELECT pmu_repo_book($1, $2, $3, $4, $5, $6, $7::numeric, $8::numeric, $9::numeric, $10, $11, $12, $13::numeric, $14, NULL)")
        .bind(id)
        .bind(&r.direction)
        .bind(r.counterparty.trim())
        .bind(&r.ccy)
        .bind(r.start)
        .bind(r.end)
        .bind(format!("{:.2}", pv.cash))
        .bind(format!("{:.6}", r.rate))
        .bind(format!("{:.3}", r.haircut_pct))
        .bind(r.kondor_id.as_deref())
        .bind(if r.direction == "REPO" { Some(serde_json::Value::Array(coll.clone())) } else { None })
        .bind(r.collateral_isin.as_deref().map(|s| s.trim().to_uppercase()))
        .bind(r.collateral_face.map(|f| format!("{f:.2}")))
        .bind(r.collateral_note.as_deref())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let detail = serde_json::json!({ "repo": id, "direction": r.direction, "counterparty": r.counterparty, "cash": pv.cash, "rate": r.rate, "start": r.start, "end": r.end, "collateral": coll, "reverse_collateral": r.collateral_isin });
    audit::write(&state.pool, Some(auth.user.id), &auth.user.username, "pmu_repo_book", &AuditCtx::new(Some(peer), &headers), Some(detail)).await;
    let what = if r.direction == "REPO" { "Repo" } else { "Reverse repo" };
    Ok((StatusCode::CREATED, Json(Booked { id: id.to_string(), message: format!("{what} with {} for {:.0} {} to {}", r.counterparty.trim(), pv.cash, r.ccy, r.end) })))
}

pub async fn repo_close(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(c): Json<RepoClose>,
) -> Result<Json<Booked>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    require_booker(&mut tx).await?;
    sqlx::query("SELECT pmu_repo_close($1, $2)").bind(id).bind(c.on).execute(&mut *tx).await?;
    tx.commit().await?;
    audit::write(&state.pool, Some(auth.user.id), &auth.user.username, "pmu_repo_close", &AuditCtx::new(Some(peer), &headers), Some(serde_json::json!({ "repo": id, "on": c.on }))).await;
    Ok(Json(Booked { id: id.to_string(), message: format!("Repo closed on {}", c.on) }))
}

pub async fn repo_roll(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(r): Json<RepoRoll>,
) -> Result<(StatusCode, Json<Booked>), ApiError> {
    if !(r.rate.is_finite() && (-5.0..=50.0).contains(&r.rate)) {
        return Err(bad("the rate must be -5% to 50%"));
    }
    if r.end <= r.on {
        return Err(bad("the new repo must end after it starts"));
    }
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    require_booker(&mut tx).await?;
    let (_, _, data) = context(&state, &mut tx).await?;
    let old = pmu::booking::find_repo(&data.portfolio, id).ok_or(ApiError::NotFound)?;
    // The collateral carries over; it must stay free until the new end date.
    if !old.reverse() {
        let picks: Vec<domain::LotPick> = old.collateral.iter().map(|(l, f)| domain::LotPick { lot_id: *l, face: *f }).collect();
        let pv = pmu::booking::repo_preview(&data.market, &data.portfolio, &old.direction, &old.counterparty, &old.ccy, r.on, r.end, r.rate, old.haircut_pct, r.cash.or(Some(old.cash)), &picks, Some(id));
        if let Some(b) = pv.blocking.first() {
            return Err(bad(b.clone()));
        }
    }
    let new = Uuid::now_v7();
    sqlx::query("SELECT pmu_repo_roll($1, $2, $3, $4, $5::numeric, $6::numeric, $7)")
        .bind(id)
        .bind(new)
        .bind(r.on)
        .bind(r.end)
        .bind(format!("{:.6}", r.rate))
        .bind(format!("{:.2}", r.cash.unwrap_or(old.cash)))
        .bind(r.kondor_id.as_deref())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    audit::write(&state.pool, Some(auth.user.id), &auth.user.username, "pmu_repo_roll", &AuditCtx::new(Some(peer), &headers), Some(serde_json::json!({ "from": id, "to": new, "on": r.on, "end": r.end, "rate": r.rate }))).await;
    Ok((StatusCode::CREATED, Json(Booked { id: new.to_string(), message: format!("Rolled to {} at {:.3}%", r.end, r.rate) })))
}

async fn kondor(state: &AppState, auth: &AuthUser, kind: &str, id: &str, k: &KondorAttach, peer: SocketAddr, headers: &HeaderMap) -> Result<Json<Booked>, ApiError> {
    let kid = k.kondor_id.trim();
    if kid.is_empty() || kid.len() > 40 || kid.chars().any(char::is_control) {
        return Err(bad("a Kondor id of up to 40 printable characters is required"));
    }
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    require_booker(&mut tx).await?;
    sqlx::query("SELECT pmu_set_kondor($1, $2, $3)").bind(kind).bind(id).bind(kid).execute(&mut *tx).await?;
    tx.commit().await?;
    audit::write(&state.pool, Some(auth.user.id), &auth.user.username, "pmu_kondor", &AuditCtx::new(Some(peer), headers), Some(serde_json::json!({ "kind": kind, "id": id, "kondor": kid }))).await;
    Ok(Json(Booked { id: id.to_string(), message: format!("Kondor id {kid} attached") }))
}

pub async fn lot_kondor(State(state): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Extension(auth): Extension<AuthUser>, Path(id): Path<Uuid>, Json(k): Json<KondorAttach>) -> Result<Json<Booked>, ApiError> {
    kondor(&state, &auth, "lot", &id.to_string(), &k, peer, &headers).await
}

pub async fn repo_kondor(State(state): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Extension(auth): Extension<AuthUser>, Path(id): Path<Uuid>, Json(k): Json<KondorAttach>) -> Result<Json<Booked>, ApiError> {
    kondor(&state, &auth, "repo", &id.to_string(), &k, peer, &headers).await
}

pub async fn sale_kondor(State(state): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Extension(auth): Extension<AuthUser>, Path(sale): Path<String>, Json(k): Json<KondorAttach>) -> Result<Json<Booked>, ApiError> {
    kondor(&state, &auth, "sale", sale.trim(), &k, peer, &headers).await
}

#[cfg(test)]
mod tests {
    use super::isin_valid;

    #[test]
    fn isin_check_digits() {
        assert!(isin_valid("US0378331005"));
        assert!(isin_valid("US5949181045"));
        assert!(isin_valid("us0378331005"), "case does not matter");
        assert!(!isin_valid("US0378331006"));
        assert!(!isin_valid("US037833100"));
        assert!(!isin_valid("1S0378331005"));
    }
}
