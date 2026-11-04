pub mod blotter;
pub mod breaks;
pub mod ladder;
pub mod login;
pub mod main;
pub mod positions;
pub mod status_bar;
pub mod ticket;

use egui::Color32;

pub const GREEN: Color32 = Color32::from_rgb(0x2e, 0xb8, 0x6c);
pub const AMBER: Color32 = Color32::from_rgb(0xe0, 0xa0, 0x2b);
pub const RED: Color32 = Color32::from_rgb(0xe0, 0x4b, 0x4b);
pub const MUTED: Color32 = Color32::from_gray(140);

pub fn status_color(status: domain::DealStatus) -> Color32 {
    use domain::DealStatus::*;
    match status {
        Executed => AMBER,
        Booked => Color32::from_rgb(0x4d, 0x9d, 0xe0),
        Confirmed => GREEN,
        Settled => MUTED,
        Cancelled => RED,
    }
}

pub fn match_color(m: domain::MatchStatus) -> Color32 {
    match m {
        domain::MatchStatus::Matched => GREEN,
        domain::MatchStatus::Partial => AMBER,
        domain::MatchStatus::Unmatched => MUTED,
        domain::MatchStatus::Break => RED,
    }
}

pub fn break_color(k: domain::BreakKind) -> Color32 {
    use domain::BreakKind::*;
    match k {
        Missing => AMBER,
        Mismatch => RED,
        Orphan => Color32::from_rgb(0x9b, 0x6d, 0xe0),
        Ambiguous => Color32::from_rgb(0x4d, 0x9d, 0xe0),
        Duplicate => Color32::from_rgb(0xe0, 0x6d, 0xb0),
    }
}
