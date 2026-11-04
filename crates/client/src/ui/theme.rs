//! One palette of semantic colours, resolved from the active theme so every view reads in light
//! and dark mode. Views never hold a `Color32` of their own: they ask `pal(ui)` each frame (a
//! branch on `dark_mode`, nothing more) and name the role they mean, not a colour.
//!
//! Light values are darker steps of the same hues so coloured text clears 4.5:1 on egui's light
//! panels; the six chart series are the validated categorical order from the data-viz reference
//! palette, assigned in fixed order and never cycled.

use egui::Color32;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Good: confirmed, matched, inflow, live.
    pub positive: Color32,
    /// Bad: breaks, outflow, errors.
    pub negative: Color32,
    /// Needs a look: provisional, pending, ageing.
    pub warning: Color32,
    /// The highest attention severity.
    pub critical: Color32,
    pub info: Color32,
    pub muted: Color32,
    pub booked: Color32,
    pub orphan: Color32,
    pub ambiguous: Color32,
    pub duplicate: Color32,
    /// Categorical chart series in fixed order.
    pub series: [Color32; 6],
    pub grid: Color32,
    pub axis: Color32,
    /// Card and tile background, one step off the panel.
    pub card: Color32,
    pub card_stroke: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const DARK: Palette = Palette {
    positive: rgb(0x2eb86c),
    negative: rgb(0xe04b4b),
    warning: rgb(0xe0a02b),
    critical: rgb(0xff6b6b),
    info: rgb(0x4d9de0),
    muted: rgb(0x8c8c8c),
    booked: rgb(0x4d9de0),
    orphan: rgb(0x9b6de0),
    ambiguous: rgb(0x4d9de0),
    duplicate: rgb(0xe06db0),
    series: [rgb(0x3987e5), rgb(0xd95926), rgb(0x199e70), rgb(0xc98500), rgb(0xd55181), rgb(0x008300)],
    grid: rgb(0x2c2c2a),
    axis: rgb(0x383835),
    card: rgb(0x232323),
    card_stroke: rgb(0x343434),
};

pub const LIGHT: Palette = Palette {
    positive: rgb(0x1a7f3f),
    negative: rgb(0xc22f2f),
    warning: rgb(0x9c6500),
    critical: rgb(0xb42828),
    info: rgb(0x1f5fb0),
    muted: rgb(0x6e6e6e),
    booked: rgb(0x1f5fb0),
    orphan: rgb(0x6a43c0),
    ambiguous: rgb(0x1f5fb0),
    duplicate: rgb(0xb8407f),
    series: [rgb(0x2a78d6), rgb(0xeb6834), rgb(0x1baf7a), rgb(0xeda100), rgb(0xe87ba4), rgb(0x008300)],
    grid: rgb(0xe1e0d9),
    axis: rgb(0xc3c2b7),
    card: rgb(0xffffff),
    card_stroke: rgb(0xdcdcdc),
};

pub fn pal(ui: &egui::Ui) -> Palette {
    if ui.visuals().dark_mode { DARK } else { LIGHT }
}

/// What the client remembers between runs: preferences only, never data and never credentials.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientPrefs {
    pub theme: egui::ThemePreference,
}
