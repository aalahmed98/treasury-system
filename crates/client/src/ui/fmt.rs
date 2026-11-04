//! Formatting helpers shared by every view. Numbers are formatted once when data arrives, never
//! per frame; these are the only places the formats live.

use chrono::{DateTime, Local, NaiveDate, Utc};
use egui::{Align, Color32, Layout};
use rust_decimal::Decimal;

use crate::ui::theme::Palette;

pub fn fmt_local(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%d-%b %H:%M:%S").to_string()
}

pub fn fmt_time(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%H:%M:%S").to_string()
}

pub fn fmt_date(d: NaiveDate) -> String {
    d.format("%a %d %b").to_string()
}

/// 1234567.5 -> "1,234,567.5"
pub fn fmt_amount(d: Decimal) -> String {
    let d = d.normalize();
    let s = d.abs().to_string();
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s.as_str(), None),
    };
    let mut out = String::with_capacity(s.len() + s.len() / 3 + 1);
    if d.is_sign_negative() {
        out.push('-');
    }
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if let Some(f) = frac {
        out.push('.');
        out.push_str(f);
    }
    out
}

/// 1_234_567_890 -> "1.23bn", 45_600_000 -> "45.6m", 1_200 -> "1.20k"; sign kept.
pub fn fmt_money_short(v: f64) -> String {
    let a = v.abs();
    let (num, suffix) = if a >= 1e9 {
        (v / 1e9, "bn")
    } else if a >= 1e6 {
        (v / 1e6, "m")
    } else if a >= 1e3 {
        (v / 1e3, "k")
    } else {
        (v, "")
    };
    let digits = if suffix.is_empty() || num.abs() >= 100.0 {
        format!("{num:.0}")
    } else if num.abs() >= 10.0 {
        format!("{num:.1}")
    } else {
        format!("{num:.2}")
    };
    format!("{digits}{suffix}")
}

#[allow(dead_code)]
pub fn fmt_pct(v: f64, dp: usize) -> String {
    format!("{v:.dp$}%")
}

pub fn fmt_age(secs: i64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

pub fn right(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui)) {
    ui.with_layout(Layout::right_to_left(Align::Center), f);
}

/// Freshness of a feed or the reconciler, from an age in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Good,
    Warn,
    Bad,
    Unknown,
}

impl Health {
    pub fn color(self, p: &Palette) -> Color32 {
        match self {
            Health::Good => p.positive,
            Health::Warn => p.warning,
            Health::Bad => p.negative,
            Health::Unknown => p.muted,
        }
    }
}

/// A feed heartbeats every couple of minutes; three minutes without one is a warning, five is a fault.
pub fn feed_health(age_secs: Option<i64>) -> Health {
    match age_secs {
        None => Health::Unknown,
        Some(a) if a < 180 => Health::Good,
        Some(a) if a < 300 => Health::Warn,
        Some(_) => Health::Bad,
    }
}

/// The reconciler ticks every ten seconds; half a minute of silence is a warning, two minutes a fault.
pub fn recon_health(lag_secs: i64) -> Health {
    if lag_secs < 30 {
        Health::Good
    } else if lag_secs < 120 {
        Health::Warn
    } else {
        Health::Bad
    }
}
