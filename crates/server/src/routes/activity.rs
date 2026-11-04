//! Recent activity on the caller's desks (through the definer function, since the audit and
//! history tables stay closed to the API role) and the server-computed attention list.

use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{DateTime, NaiveDate, Utc};
use domain::calendar::Calendar;
use domain::{ActivityQuery, ActivityRow, AttentionItem, AttentionSeverity, AttentionTarget};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db;
use crate::error::ApiError;
use crate::routes::alm;
use crate::state::AppState;

pub async fn activity(State(state): State<AppState>, Extension(auth): Extension<AuthUser>, Query(q): Query<ActivityQuery>) -> Result<Json<Vec<ActivityRow>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as i32;
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let rows: Vec<(DateTime<Utc>, String, String, String, Option<Uuid>, Option<i64>, String)> =
        sqlx::query_as("SELECT at, kind, actor, summary, deal_id, break_id, desk_code FROM recent_activity($1)").bind(limit).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(
        rows.into_iter()
            .map(|(at, kind, actor, summary, deal_id, break_id, desk_code)| ActivityRow { at, kind, actor, summary, deal_id, break_id, desk_code })
            .collect(),
    ))
}

fn item(severity: AttentionSeverity, title: String, detail: String, since: Option<DateTime<Utc>>, target: AttentionTarget) -> AttentionItem {
    AttentionItem { severity, title, detail, since, target }
}

fn age(now: DateTime<Utc>, t: DateTime<Utc>) -> String {
    let s = (now - t).num_seconds().max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86_400)
    }
}

pub async fn attention(State(state): State<AppState>, Extension(auth): Extension<AuthUser>) -> Result<Json<Vec<AttentionItem>>, ApiError> {
    let mut tx = db::begin_as(&state.pool, auth.user.id).await?;
    let now = db::business_now(&mut tx).await?;
    let today = now.date_naive();
    let mut items = Vec::new();

    // Open breaks: a few of a kind individually, many as one line per kind.
    let breaks: Vec<(i64, String, Option<String>, Option<String>, Option<String>, Option<String>, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT b.id, b.kind, d.deal_ref, b.source, b.source_ref, b.field, k.code, b.detected_at \
         FROM breaks b JOIN desks k ON k.id = b.desk_id LEFT JOIN deals d ON d.id = b.deal_id \
         WHERE b.status <> 'RESOLVED' ORDER BY b.detected_at DESC LIMIT 500",
    )
    .fetch_all(&mut *tx)
    .await?;
    for kind in ["MISSING", "AMBIGUOUS", "MISMATCH", "ORPHAN", "DUPLICATE"] {
        let of_kind: Vec<_> = breaks.iter().filter(|b| b.1 == kind).collect();
        if of_kind.is_empty() {
            continue;
        }
        let (severity, what) = match kind {
            "MISSING" => (AttentionSeverity::Critical, "past its SLA"),
            "AMBIGUOUS" => (AttentionSeverity::Warning, "needs a decision"),
            "MISMATCH" => (AttentionSeverity::Warning, "disagrees across sources"),
            "ORPHAN" => (AttentionSeverity::Info, "has no golden deal"),
            _ => (AttentionSeverity::Info, "looks like a repeat"),
        };
        if of_kind.len() <= 3 {
            for b in of_kind {
                let subject = b.2.clone().unwrap_or_else(|| format!("{} {}", b.3.clone().unwrap_or_default(), b.4.clone().unwrap_or_default()).trim().to_string());
                items.push(item(
                    severity,
                    format!("{} {subject}", kind.to_lowercase()),
                    format!("{} {what}{}", b.6, b.5.as_deref().map(|f| format!(" on {f}")).unwrap_or_default()),
                    Some(b.7),
                    AttentionTarget::Break { id: b.0 },
                ));
            }
        } else {
            let newest = of_kind.iter().map(|b| b.7).max();
            items.push(item(severity, format!("{} {} breaks", of_kind.len(), kind.to_lowercase()), format!("each {what}; open the Breaks tab to work through them"), newest, AttentionTarget::Breaks));
        }
    }

    // Feeds and the reconciler.
    let feeds: Vec<(String, Option<DateTime<Utc>>)> =
        sqlx::query_as("SELECT source, last_heartbeat_at FROM feed_status WHERE source NOT IN ('INTERNAL', 'USER') ORDER BY source").fetch_all(&mut *tx).await?;
    for (source, hb) in feeds {
        let secs = hb.map(|h| (now - h).num_seconds().max(0));
        let severity = match secs {
            Some(s) if s < 180 => continue,
            Some(s) if s < 300 => AttentionSeverity::Warning,
            _ => AttentionSeverity::Critical,
        };
        items.push(item(
            severity,
            format!("{source} feed stale"),
            match hb {
                Some(h) => format!("last heartbeat {} ago; deals from this source may be missing", age(now, h)),
                None => "never heartbeated since the reconciler started".into(),
            },
            hb,
            AttentionTarget::Overview,
        ));
    }
    let recon: Option<(DateTime<Utc>, i64)> = sqlx::query_scalar::<_, (DateTime<Utc>, i64)>("SELECT (last_tick_at, pending) FROM recon_status WHERE name = 'worker'")
        .fetch_optional(&mut *tx)
        .await
        .ok()
        .flatten();
    match recon {
        Some((tick, pending)) => {
            let lag = (Utc::now() - tick).num_seconds().max(0);
            if lag >= 30 {
                items.push(item(
                    if lag >= 120 { AttentionSeverity::Critical } else { AttentionSeverity::Warning },
                    "reconciler lagging".into(),
                    format!("last tick {} ago with {pending} events pending; matches and breaks are behind", age(Utc::now(), tick)),
                    Some(tick),
                    AttentionTarget::Overview,
                ));
            }
        }
        None => items.push(item(
            AttentionSeverity::Critical,
            "reconciler down".into(),
            "the reconciliation worker has never reported; nothing is being matched".into(),
            None,
            AttentionTarget::Overview,
        )),
    }

    // Deals still provisional a business day after they were dealt.
    let holidays: Vec<NaiveDate> = sqlx::query_scalar("SELECT day FROM calendar_holidays").fetch_all(&mut *tx).await?;
    let cal = Calendar::new(holidays);
    let latest: Option<NaiveDate> = sqlx::query_scalar("SELECT max(trade_date) FROM deals").fetch_one(&mut *tx).await?;
    if let Some(latest) = latest {
        let cutoff = cal.add_business_days(latest, -1);
        let (n, oldest): (i64, Option<DateTime<Utc>>) = sqlx::query_as(
            "SELECT count(*), min(updated_at) FROM deals WHERE NOT confirmed AND status <> 'CANCELLED' AND merged_into IS NULL AND trade_date < $1",
        )
        .bind(cutoff)
        .fetch_one(&mut *tx)
        .await?;
        if n > 0 {
            items.push(item(
                AttentionSeverity::Warning,
                format!("{n} deals unconfirmed beyond a business day"),
                "the back office has not confirmed them; the blotter filter shows provisional deals".into(),
                oldest,
                AttentionTarget::Provisional,
            ));
        }
    }

    // Large settlements on the next business day.
    let next = cal.add_business_days(today, 1);
    let flows: Vec<(String, Decimal)> =
        sqlx::query_as("SELECT ccy, sum(amount) FROM cash_flows_v WHERE flow_date = $1 GROUP BY ccy HAVING abs(sum(amount)) >= 50000000 ORDER BY abs(sum(amount)) DESC")
            .bind(next)
            .fetch_all(&mut *tx)
            .await?;
    for (ccy, net) in flows {
        let v = net.to_f64().unwrap_or(0.0);
        items.push(item(
            AttentionSeverity::Info,
            format!("{ccy} {} net on {}", short(v), next.format("%a %d %b")),
            "a large settlement is due on the next business day; check the ladder and the nostro balance".into(),
            None,
            AttentionTarget::Overview,
        ));
    }

    // ALM limits, for users who may see them.
    if alm::can_see_alm(&mut tx).await? {
        if let Some(o) = alm::build_overview(&state, &mut tx).await? {
            for l in &o.limits {
                if !(l.breach || l.warn) {
                    continue;
                }
                let tool = match l.metric.as_str() {
                    "LCR" => "lcr",
                    "NSFR" => "nsfr",
                    m if m.starts_with("Cumulative gap") => "gap",
                    m if m.starts_with("EVE") || m.starts_with("NII") => "stress",
                    _ => "funding",
                };
                let value = l.value.map(|v| format!("{v:.1}%")).unwrap_or_else(|| "—".into());
                items.push(item(
                    if l.breach { AttentionSeverity::Critical } else { AttentionSeverity::Warning },
                    format!("{} {}", l.metric, if l.breach { "outside its limit" } else { "near its limit" }),
                    format!("{value} against a limit of {:.1}%{}", l.limit, l.trigger.map(|t| format!(" and an early warning at {t:.1}%")).unwrap_or_default()),
                    None,
                    AttentionTarget::Alm { tool: tool.into() },
                ));
            }
        }
        let changed: Option<(i32, DateTime<Utc>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT p.version, p.created_at, u.display_name, p.note FROM alm_parameters p LEFT JOIN users u ON u.id = p.created_by \
             WHERE p.is_active AND p.created_at >= now() - interval '1 day' AND p.version > 1",
        )
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((v, at, by, note)) = changed {
            items.push(item(
                AttentionSeverity::Info,
                format!("ALM parameters changed to v{v}"),
                format!("{}: {}", by.unwrap_or_else(|| "someone".into()), note.unwrap_or_default()),
                Some(at),
                AttentionTarget::Alm { tool: "setup".into() },
            ));
        }
    }
    tx.commit().await?;

    items.sort_by(|a, b| a.severity.cmp(&b.severity).then_with(|| b.since.cmp(&a.since)));
    Ok(Json(items))
}

fn short(v: f64) -> String {
    let a = v.abs();
    if a >= 1e9 {
        format!("{:.2}bn", v / 1e9)
    } else if a >= 1e6 {
        format!("{:.1}m", v / 1e6)
    } else {
        format!("{v:.0}")
    }
}
