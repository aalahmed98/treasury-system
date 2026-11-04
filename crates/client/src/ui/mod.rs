pub mod alm;
pub mod blotter;
pub mod breaks;
pub mod charts;
pub mod fmt;
pub mod ladder;
pub mod login;
pub mod overview;
pub mod positions;
pub mod shell;
pub mod status_bar;
pub mod theme;
pub mod ticket;
pub mod widgets;

use egui::Color32;

pub use theme::{Palette, pal};

pub fn status_color(p: &Palette, status: domain::DealStatus) -> Color32 {
    use domain::DealStatus::*;
    match status {
        Executed => p.warning,
        Booked => p.booked,
        Confirmed => p.positive,
        Settled => p.muted,
        Cancelled => p.negative,
    }
}

pub fn match_color(p: &Palette, m: domain::MatchStatus) -> Color32 {
    match m {
        domain::MatchStatus::Matched => p.positive,
        domain::MatchStatus::Partial => p.warning,
        domain::MatchStatus::Unmatched => p.muted,
        domain::MatchStatus::Break => p.negative,
    }
}

pub fn break_color(p: &Palette, k: domain::BreakKind) -> Color32 {
    use domain::BreakKind::*;
    match k {
        Missing => p.warning,
        Mismatch => p.negative,
        Orphan => p.orphan,
        Ambiguous => p.ambiguous,
        Duplicate => p.duplicate,
    }
}
