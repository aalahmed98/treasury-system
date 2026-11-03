pub mod blotter;
pub mod login;
pub mod main;
pub mod positions;
pub mod status_bar;

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
