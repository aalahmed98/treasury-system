//! Saved ALM scenarios: a rate shock, the what-if assumptions as changes against the bank's
//! parameters, and trial hedges, kept under a name. Private to their owner until shared with the
//! desk; a shared one may be put in the ALCO pack. A scenario is a recipe, recomputed on the
//! snapshot it is opened on, so nothing here is cached or official.

use alm::curve::{CustomShock, shock_label};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use domain::{AlmParams, AlmScenario, BsPosition, CompareRequest, CompareRow, ParamChange, RateScenario, SaveScenarioRequest, ScenarioBody, ScenarioFlag};
use sqlx::PgConnection;
use sqlx::types::Json as Jsonb;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db;
use crate::error::ApiError;
use crate::routes::alm::{Book, active_params, check_hedges, custom_from, custom_shock, entitled, hedged_positions, load_book, validate};
use crate::state::AppState;

type Row = (Uuid, String, Option<String>, bool, bool, bool, Jsonb<serde_json::Value>, DateTime<Utc>);

/// The caller's own scenarios first, then the shared ones, each by name; optionally only those
/// in the ALCO pack, or one by id.
const SELECT: &str = "SELECT s.id, s.name, u.display_name, s.owner = current_app_user(), s.shared, s.in_pack, s.body, s.updated_at \
     FROM alm_scenarios s LEFT JOIN users u ON u.id = s.owner \
     WHERE ($1 = false OR (s.in_pack AND s.shared)) AND ($2::uuid IS NULL OR s.id = $2) \
     ORDER BY (s.owner = current_app_user()) DESC, lower(s.name), s.id";

fn scenario(row: Row) -> AlmScenario {
    let (id, name, owner_name, mine, shared, in_pack, body, updated_at) = row;
    // A body an older build wrote still opens: what cannot be read is left at its default.
    let body = serde_json::from_value(body.0).unwrap_or_default();
    AlmScenario { id, name, owner_name: owner_name.unwrap_or_else(|| "a former user".into()), mine, shared, in_pack, body, updated_at }
}

/// Every scenario the caller may see: row-level security leaves out the rest.
async fn visible(conn: &mut PgConnection, pack_only: bool) -> Result<Vec<AlmScenario>, ApiError> {
    let rows: Vec<Row> = sqlx::query_as(SELECT).bind(pack_only).bind(None::<Uuid>).fetch_all(&mut *conn).await?;
    Ok(rows.into_iter().map(scenario).collect())
}

/// Set a leaf of a JSON document by its path, making the objects on the way; null removes it.
fn set_path(doc: &mut serde_json::Value, path: &[String], value: serde_json::Value) -> Result<(), String> {
    let Some((last, rest)) = path.split_last() else { return Err("an assumption needs a path".into()) };
    let mut cur = doc;
    for key in rest {
        let Some(obj) = cur.as_object_mut() else { return Err(format!("{} is not a group of assumptions", path.join("."))) };
        cur = obj.entry(key.clone()).or_insert_with(|| serde_json::Value::Object(Default::default()));
    }
    let Some(obj) = cur.as_object_mut() else { return Err(format!("{} is not a group of assumptions", path.join("."))) };
    if value.is_null() {
        obj.remove(last);
    } else {
        obj.insert(last.clone(), value);
    }
    Ok(())
}

/// The bank's parameters with a scenario's changes applied, checked as a saved version would be.
pub fn apply_changes(params: &AlmParams, changes: &[ParamChange]) -> Result<AlmParams, ApiError> {
    if changes.is_empty() {
        return Ok(params.clone());
    }
    let bad = |m: String| ApiError::BadRequest(m);
    let mut doc = serde_json::to_value(params).map_err(|e| ApiError::Internal(e.into()))?;
    for c in changes {
        if c.path.is_empty() || c.path.len() > 6 || c.path.iter().any(|k| k.is_empty() || k.len() > 80) {
            return Err(bad("an assumption's path is not valid".into()));
        }
        set_path(&mut doc, &c.path, c.value.clone()).map_err(bad)?;
    }
    let changed: AlmParams = serde_json::from_value(doc).map_err(|e| bad(format!("an assumption no longer fits the parameters: {e}")))?;
    validate(&changed)?;
    Ok(changed)
}

fn check_body(body: &ScenarioBody, params: &AlmParams) -> Result<(), ApiError> {
    if body.changes.len() > 400 {
        return Err(ApiError::BadRequest("a scenario may change at most 400 assumptions".into()));
    }
    custom_from(body.custom_shock_bp, &body.custom_shock_buckets)?;
    check_hedges(&body.hedges)?;
    apply_changes(params, &body.changes).map(|_| ())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// `Parallel up, 2 trial hedges, 3 assumptions changed`.
fn summary(scenario: RateScenario, custom: CustomShock, hedges: usize, changes: usize) -> String {
    let mut parts = vec![shock_label(scenario, custom).unwrap_or_else(|| "Today's curve".into())];
    if hedges > 0 {
        parts.push(plural(hedges, "trial hedge", "trial hedges"));
    }
    if changes > 0 {
        parts.push(plural(changes, "assumption changed", "assumptions changed"));
    }
    parts.join(", ")
}

/// One simulation on a balance sheet: the stress row and the gap's estimate of the effect on NII.
fn run(book: &Book, positions: &[BsPosition], params: &AlmParams, scenario: RateScenario, custom: CustomShock) -> (domain::StressScenarioRow, Option<f64>) {
    let row = alm::stress::single(positions, params, &book.spots, &book.curves, book.as_of, scenario, custom, None);
    let gap = alm::gap::under(positions, params, &book.spots, &book.curves, book.as_of, scenario, custom, None);
    (row, gap.nii_effect_12m)
}

fn saved_row(book: &Book, bank: &AlmParams, s: &AlmScenario) -> CompareRow {
    let outcome = (|| -> Result<(String, domain::StressScenarioRow, Option<f64>), ApiError> {
        let custom = custom_from(s.body.custom_shock_bp, &s.body.custom_shock_buckets)?.unwrap_or(CustomShock::parallel(0.0));
        let params = apply_changes(bank, &s.body.changes)?;
        let positions = hedged_positions(book, &s.body.hedges)?;
        let (row, gap) = run(book, &positions, &params, s.body.scenario, custom);
        Ok((summary(s.body.scenario, custom, s.body.hedges.len(), s.body.changes.len()), row, gap))
    })();
    let (summary, row, gap_nii_effect, error) = match outcome {
        Ok((summary, row, gap)) => (summary, Some(row), gap, None),
        Err(ApiError::BadRequest(m)) => (String::new(), None, None, Some(m)),
        Err(e) => (String::new(), None, None, Some(e.to_string())),
    };
    CompareRow { id: Some(s.id), name: s.name.clone(), author: Some(s.owner_name.clone()), summary, row, gap_nii_effect, error }
}

pub async fn list(State(state): State<AppState>, Extension(auth): Extension<AuthUser>) -> Result<Json<Vec<AlmScenario>>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let out = visible(&mut tx, false).await?;
    tx.commit().await?;
    Ok(Json(out))
}

pub async fn save(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<SaveScenarioRequest>) -> Result<Json<AlmScenario>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let (_, bank) = active_params(&mut tx).await?;
    check_body(&req.body, &bank)?;
    let body = serde_json::to_value(&req.body).map_err(|e| ApiError::Internal(e.into()))?;
    let id: Uuid = sqlx::query_scalar("SELECT alm_scenario_save($1, $2, $3)").bind(req.id).bind(req.name.trim()).bind(Jsonb(body)).fetch_one(&mut *tx).await?;
    let row: Row = sqlx::query_as(SELECT).bind(false).bind(Some(id)).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(scenario(row)))
}

pub async fn share(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Path(id): Path<Uuid>, Json(flag): Json<ScenarioFlag>) -> Result<StatusCode, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    sqlx::query("SELECT alm_scenario_share($1, $2)").bind(id).bind(flag.on).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pack(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Path(id): Path<Uuid>, Json(flag): Json<ScenarioFlag>) -> Result<StatusCode, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    sqlx::query("SELECT alm_scenario_pack($1, $2)").bind(id).bind(flag.on).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    sqlx::query("SELECT alm_scenario_delete($1)").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The scenarios side by side on one snapshot. For the workbench: the current unsaved simulation
/// first, then every saved one the caller can see. For the ALCO pack: the ones put in the pack.
pub async fn compare(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Json(req): Json<CompareRequest>) -> Result<Json<Vec<CompareRow>>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    entitled(&mut tx).await?;
    let book = load_book(&mut tx, req.as_of).await?;
    let (_, bank) = active_params(&mut tx).await?;
    let saved = visible(&mut tx, req.pack).await?;
    tx.commit().await?;

    let mut out = Vec::with_capacity(saved.len() + 1);
    if let Some(cur) = req.current.as_ref().filter(|_| !req.pack) {
        let custom = custom_shock(cur)?.unwrap_or(CustomShock::parallel(0.0));
        let scenario = cur.scenario.unwrap_or(RateScenario::Base);
        let hedges = cur.hedges.as_deref().unwrap_or(&[]);
        let params = cur.params.as_ref().unwrap_or(&bank);
        let positions = hedged_positions(&book, hedges)?;
        let (row, gap) = run(&book, &positions, params, scenario, custom);
        let mut line = summary(scenario, custom, hedges.len(), 0);
        if cur.params.is_some() {
            line.push_str(", what-if assumptions");
        }
        out.push(CompareRow { id: None, name: "Current simulation".into(), author: None, summary: line, row: Some(row), gap_nii_effect: gap, error: None });
    }
    out.extend(saved.iter().take(40).map(|s| saved_row(&book, &bank, s)));
    Ok(Json(out))
}
