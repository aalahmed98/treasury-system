//! What needs attention right now, computed on the client from the data it already holds (breaks,
//! feeds, deals, the live stream). Rebuilt only when one of those changes; the server-computed list
//! with limit breaches replaces it once the ALM endpoints exist.

use chrono::{DateTime, Utc};
use domain::calendar::Calendar;
use domain::{BreakKind, BreakStatus, DealStatus};

use crate::alm_state::AlmTool;
use crate::app::{App, AttentionItem, Severity, Tab, Target};
use crate::ui::fmt::{Health, feed_health, fmt_age, recon_health};

/// The reconciler's clock when known (replayed worlds run on it), else the server's, else ours.
pub fn reference_now(app: &App) -> DateTime<Utc> {
    app.feeds.as_ref().map(|f| f.recon.as_ref().map(|r| r.now).unwrap_or(f.server_time)).unwrap_or_else(Utc::now)
}

pub fn build(app: &App) -> Vec<AttentionItem> {
    let now = reference_now(app);
    let since = |t: DateTime<Utc>| fmt_age((now - t).num_seconds().max(0));
    let mut items = Vec::new();

    // Open breaks: a few individually, many as one line per kind.
    for kind in BreakKind::ALL {
        let open: Vec<_> = app.breaks.rows().iter().filter(|r| r.view.kind == *kind && r.view.status != BreakStatus::Resolved).collect();
        if open.is_empty() {
            continue;
        }
        let (severity, what) = match kind {
            BreakKind::Missing => (Severity::Critical, "past its SLA"),
            BreakKind::Ambiguous => (Severity::Warning, "needs a decision"),
            BreakKind::Mismatch => (Severity::Warning, "disagrees across sources"),
            BreakKind::Orphan => (Severity::Info, "has no golden deal"),
            BreakKind::Duplicate => (Severity::Info, "looks like a repeat"),
        };
        if open.len() <= 3 {
            for r in open {
                let subject = r.view.deal_ref.clone().unwrap_or_else(|| r.source.clone());
                items.push(AttentionItem {
                    severity,
                    title: format!("{} {subject}", kind.as_str().to_lowercase()),
                    detail: format!("{} {what}{}", r.view.desk_code, r.view.field.as_deref().map(|f| format!(" on {f}")).unwrap_or_default()),
                    since: since(r.view.detected_at),
                    at: r.view.detected_at,
                    target: Target::Break(r.view.id),
                });
            }
        } else {
            let newest = open.iter().map(|r| r.view.detected_at).max().unwrap_or(now);
            items.push(AttentionItem {
                severity,
                title: format!("{} {} breaks", open.len(), kind.as_str().to_lowercase()),
                detail: format!("each {what}; open the Breaks tab to work through them"),
                since: since(newest),
                at: newest,
                target: Target::Tab(Tab::Breaks),
            });
        }
    }

    // Feeds and the reconciler.
    if let Some(f) = &app.feeds {
        for feed in &f.feeds {
            if matches!(feed.source, domain::Source::Internal | domain::Source::User) {
                continue;
            }
            let age = feed.last_heartbeat_at.map(|hb| (now - hb).num_seconds().max(0));
            let severity = match feed_health(age) {
                Health::Bad | Health::Unknown => Severity::Critical,
                Health::Warn => Severity::Warning,
                Health::Good => continue,
            };
            items.push(AttentionItem {
                severity,
                title: format!("{} feed stale", feed.source.as_str()),
                detail: match age {
                    Some(a) => format!("last heartbeat {} ago; deals from this source may be missing", fmt_age(a)),
                    None => "never heartbeated since the reconciler started".into(),
                },
                since: age.map(fmt_age).unwrap_or_else(|| "—".into()),
                at: feed.last_heartbeat_at.unwrap_or(now),
                target: Target::Tab(Tab::Overview),
            });
        }
        match &f.recon {
            Some(r) => {
                let lag = (f.server_time - r.last_tick_at).num_seconds().max(0);
                if recon_health(lag) != Health::Good {
                    items.push(AttentionItem {
                        severity: if recon_health(lag) == Health::Bad { Severity::Critical } else { Severity::Warning },
                        title: "reconciler lagging".into(),
                        detail: format!("last tick {} ago with {} events pending; matches and breaks are behind", fmt_age(lag), r.pending),
                        since: fmt_age(lag),
                        at: r.last_tick_at,
                        target: Target::Tab(Tab::Overview),
                    });
                }
            }
            None => items.push(AttentionItem {
                severity: Severity::Critical,
                title: "reconciler down".into(),
                detail: "the reconciliation worker has never reported; nothing is being matched".into(),
                since: "—".into(),
                at: now,
                target: Target::Tab(Tab::Overview),
            }),
        }
    }

    if app.stream == crate::net::StreamState::Disconnected {
        items.push(AttentionItem {
            severity: Severity::Warning,
            title: "live updates offline".into(),
            detail: "the event stream is not connected; the blotter refreshes on reconnect".into(),
            since: "—".into(),
            at: now,
            target: Target::Tab(Tab::Overview),
        });
    }

    // Deals still provisional a business day after they were dealt.
    let latest = app.store.rows().iter().map(|r| r.deal.trade_date).max();
    if let Some(latest) = latest {
        let cutoff = Calendar::default().add_business_days(latest, -1);
        let stale: Vec<_> = app
            .store
            .rows()
            .iter()
            .filter(|r| !r.deal.confirmed && r.deal.status != DealStatus::Cancelled && r.deal.trade_date < cutoff)
            .collect();
        if !stale.is_empty() {
            let oldest = stale.iter().map(|r| r.deal.updated_at).min().unwrap_or(now);
            items.push(AttentionItem {
                severity: Severity::Warning,
                title: format!("{} deals unconfirmed beyond a business day", stale.len()),
                detail: "the back office has not confirmed them; the blotter filter shows provisional deals".into(),
                since: since(oldest),
                at: oldest,
                target: Target::Provisional,
            });
        }
    }

    items.sort_by(|a, b| a.severity.cmp(&b.severity).then_with(|| b.at.cmp(&a.at)));
    items
}

/// The server's list, formatted for the bell. Replaces the local one once it arrives.
pub fn from_server(app: &App, items: &[domain::AttentionItem]) -> Vec<AttentionItem> {
    let now = reference_now(app);
    items
        .iter()
        .map(|i| AttentionItem {
            severity: match i.severity {
                domain::AttentionSeverity::Critical => Severity::Critical,
                domain::AttentionSeverity::Warning => Severity::Warning,
                domain::AttentionSeverity::Info => Severity::Info,
            },
            title: i.title.clone(),
            detail: i.detail.clone(),
            since: i.since.map(|t| fmt_age((now - t).num_seconds().max(0))).unwrap_or_else(|| "—".into()),
            at: i.since.unwrap_or(now),
            target: match &i.target {
                domain::AttentionTarget::Deal { id } => Target::Deal(*id),
                domain::AttentionTarget::Break { id } => Target::Break(*id),
                domain::AttentionTarget::Breaks => Target::Tab(Tab::Breaks),
                domain::AttentionTarget::Provisional => Target::Provisional,
                domain::AttentionTarget::Overview => Target::Tab(Tab::Overview),
                domain::AttentionTarget::Alm { tool } => Target::AlmTool(AlmTool::from_name(tool).unwrap_or(AlmTool::Dashboard)),
            },
        })
        .collect()
}
