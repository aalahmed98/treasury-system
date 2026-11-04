//! The visual system: one `Palette` of semantic colours per theme, the egui `Style` built from it,
//! the bundled typeface and the type scale. Feature code asks `pal(ui)` for colours and the palette
//! for text styles; it never holds a `Color32`, a size or a font of its own. DESIGN.md documents
//! the choices.
//!
//! Colours are warm-tinted neutrals (OKLCH hue 30, toward the NBB red) with one accent, ink, for
//! selection and primary actions, and a fixed status set. The NBB red is the wordmark's alone, so
//! on every data surface red keeps meaning breach.

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, RichText, Shadow, Stroke, Style, TextStyle, Theme, Visuals};
use egui::style::{ScrollStyle, Selection, WidgetVisuals};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub dark: bool,
    /// The page behind everything.
    pub canvas: Color32,
    /// Top bar, cards, tables.
    pub surface: Color32,
    /// Side panels, striped rows, the status bar.
    pub surface_alt: Color32,
    /// Popups, menus and windows.
    pub raised: Color32,
    pub hairline: Color32,
    pub border: Color32,
    pub hover_wash: Color32,
    pub active_wash: Color32,
    pub text: Color32,
    pub text_2: Color32,
    pub muted: Color32,
    pub faint: Color32,
    /// The accent: selected chip, active tab, primary button.
    pub ink: Color32,
    pub ink_text: Color32,
    pub selection: Color32,
    pub good: Color32,
    pub good_soft: Color32,
    pub warn: Color32,
    pub warn_soft: Color32,
    pub bad: Color32,
    pub bad_soft: Color32,
    pub info: Color32,
    pub info_soft: Color32,
    /// The NBB red, for the wordmark only.
    pub brand: Color32,
    pub violet: Color32,
    pub magenta: Color32,
    /// Categorical chart series in fixed order, never cycled.
    pub series: [Color32; 6],

    // Role names the views were written against; the same colours under their older names.
    pub positive: Color32,
    pub negative: Color32,
    pub warning: Color32,
    pub critical: Color32,
    pub booked: Color32,
    pub orphan: Color32,
    pub ambiguous: Color32,
    pub duplicate: Color32,
    pub grid: Color32,
    pub axis: Color32,
    pub card: Color32,
    pub card_stroke: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const LIGHT: Palette = Palette {
    dark: false,
    canvas: rgb(0xf6f3f2),
    surface: rgb(0xfefcfc),
    surface_alt: rgb(0xfaf7f6),
    raised: rgb(0xfefdfd),
    hairline: rgb(0xe2dddc),
    border: rgb(0xc9c2c1),
    hover_wash: rgb(0xf1ebea),
    active_wash: rgb(0xe8e1e0),
    text: rgb(0x201917),
    text_2: rgb(0x59504f),
    muted: rgb(0x746b69),
    faint: rgb(0xa59c9b),
    ink: rgb(0x261d1b),
    ink_text: rgb(0xfcf9f9),
    selection: rgb(0xece2e0),
    good: rgb(0x00792b),
    good_soft: rgb(0xddf6e0),
    warn: rgb(0x9a6000),
    warn_soft: rgb(0xffedcb),
    bad: rgb(0xbe2323),
    bad_soft: rgb(0xffe6e2),
    info: rgb(0x1f68bc),
    info_soft: rgb(0xe1f0ff),
    brand: rgb(0xd62223),
    violet: rgb(0x654db6),
    magenta: rgb(0xb53c7f),
    series: [rgb(0x2a78d6), rgb(0xeb6834), rgb(0x1baf7a), rgb(0xeda100), rgb(0xe87ba4), rgb(0x008300)],
    positive: rgb(0x00792b),
    negative: rgb(0xbe2323),
    warning: rgb(0x9a6000),
    critical: rgb(0xbe2323),
    booked: rgb(0x1f68bc),
    orphan: rgb(0x654db6),
    ambiguous: rgb(0x1f68bc),
    duplicate: rgb(0xb53c7f),
    grid: rgb(0xe2dddc),
    axis: rgb(0xc9c2c1),
    card: rgb(0xfefcfc),
    card_stroke: rgb(0xe2dddc),
};

pub const DARK: Palette = Palette {
    dark: true,
    canvas: rgb(0x0e0a0a),
    surface: rgb(0x161312),
    surface_alt: rgb(0x1e1b1a),
    raised: rgb(0x242120),
    hairline: rgb(0x2c2827),
    border: rgb(0x474140),
    hover_wash: rgb(0x282322),
    active_wash: rgb(0x322c2b),
    text: rgb(0xebe7e6),
    text_2: rgb(0xada6a5),
    muted: rgb(0x8c8483),
    faint: rgb(0x5b5352),
    ink: rgb(0xe8e3e2),
    ink_text: rgb(0x16100f),
    selection: rgb(0x3a302f),
    good: rgb(0x4ec36f),
    good_soft: rgb(0x15301b),
    warn: rgb(0xf0b135),
    warn_soft: rgb(0x3b2b0d),
    bad: rgb(0xf66d67),
    bad_soft: rgb(0x47211e),
    info: rgb(0x60aaf3),
    info_soft: rgb(0x142a41),
    brand: rgb(0xed4b43),
    violet: rgb(0xa495f0),
    magenta: rgb(0xf080b8),
    series: [rgb(0x3987e5), rgb(0xd95926), rgb(0x199e70), rgb(0xc98500), rgb(0xd55181), rgb(0x008300)],
    positive: rgb(0x4ec36f),
    negative: rgb(0xf66d67),
    warning: rgb(0xf0b135),
    critical: rgb(0xf66d67),
    booked: rgb(0x60aaf3),
    orphan: rgb(0xa495f0),
    ambiguous: rgb(0x60aaf3),
    duplicate: rgb(0xf080b8),
    grid: rgb(0x2c2827),
    axis: rgb(0x474140),
    card: rgb(0x161312),
    card_stroke: rgb(0x2c2827),
};

pub fn pal(ui: &egui::Ui) -> Palette {
    if ui.visuals().dark_mode { DARK } else { LIGHT }
}

// ---- type scale -------------------------------------------------------------------------------

const FONT_REGULAR: &[u8] = include_bytes!("../../assets/fonts/InterTreasury-Regular.ttf");
const FONT_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/InterTreasury-Medium.ttf");
const FONT_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/InterTreasury-SemiBold.ttf");

pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub const TITLE: &str = "title";
pub const STAT: &str = "stat";
pub const HERO: &str = "hero";
pub const LABEL: &str = "label";

fn named(name: &str) -> TextStyle {
    TextStyle::Name(name.into())
}

/// Medium weight for emphasis: egui's `strong` only changes colour.
pub fn strong(text: impl Into<String>) -> RichText {
    RichText::new(text).family(medium())
}

impl Palette {
    /// Page title, 20 semibold.
    pub fn title(&self, text: impl Into<String>) -> RichText {
        RichText::new(text).text_style(named(TITLE)).color(self.text)
    }

    /// Card and panel titles, 15 semibold.
    pub fn heading(&self, text: impl Into<String>) -> RichText {
        RichText::new(text).text_style(TextStyle::Heading).color(self.text)
    }

    /// The one number a tool is about, 32 semibold.
    pub fn hero(&self, text: impl Into<String>) -> RichText {
        RichText::new(text).text_style(named(HERO)).color(self.text)
    }

    /// Table headers and form labels, 12 medium.
    pub fn label(&self, text: impl Into<String>) -> RichText {
        RichText::new(text).text_style(named(LABEL)).color(self.text_2)
    }

    /// Section headers: 11 medium, capitals, tracked.
    pub fn eyebrow(&self, text: impl Into<String>) -> RichText {
        let s: String = text.into();
        RichText::new(s.to_uppercase()).text_style(named(LABEL)).size(11.0).extra_letter_spacing(0.7).color(self.muted)
    }

    pub fn muted(&self, text: impl Into<String>) -> RichText {
        RichText::new(text).color(self.muted)
    }

    pub fn small(&self, text: impl Into<String>) -> RichText {
        RichText::new(text).small().color(self.muted)
    }

    pub fn secondary(&self, text: impl Into<String>) -> RichText {
        RichText::new(text).color(self.text_2)
    }

    /// Soft popup shadow: light mode casts one, dark mode relies on the lighter surface.
    pub fn shadow(&self) -> Shadow {
        if self.dark {
            Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(110) }
        } else {
            Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(26) }
        }
    }
}

/// Fonts and both themes' styles, once at start-up.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [("inter", FONT_REGULAR), ("inter-medium", FONT_MEDIUM), ("inter-semibold", FONT_SEMIBOLD)] {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    // Keep egui's icon and emoji fonts behind ours for the glyphs Inter lacks.
    let fallback: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let family = |first: &str| {
        let mut v = vec![first.to_owned()];
        v.extend(fallback.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Proportional, family("inter"));
    fonts.families.insert(medium(), family("inter-medium"));
    fonts.families.insert(semibold(), family("inter-semibold"));
    ctx.set_fonts(fonts);
    ctx.style_mut_of(Theme::Light, |s| style(s, &LIGHT));
    ctx.style_mut_of(Theme::Dark, |s| style(s, &DARK));
}

fn style(s: &mut Style, p: &Palette) {
    use FontFamily::Proportional as P;
    s.text_styles = [
        (TextStyle::Small, FontId::new(11.0, P)),
        (TextStyle::Body, FontId::new(13.0, P)),
        // Figures: the bundled font has tabular digits on by default, so columns align.
        (TextStyle::Monospace, FontId::new(13.0, P)),
        (TextStyle::Button, FontId::new(13.0, medium())),
        (TextStyle::Heading, FontId::new(15.0, semibold())),
        (named(TITLE), FontId::new(20.0, semibold())),
        (named(STAT), FontId::new(24.0, semibold())),
        (named(HERO), FontId::new(32.0, semibold())),
        (named(LABEL), FontId::new(12.0, medium())),
    ]
    .into();
    s.drag_value_text_style = TextStyle::Body;
    s.animation_time = 0.1;

    let sp = &mut s.spacing;
    sp.item_spacing = egui::vec2(8.0, 6.0);
    sp.button_padding = egui::vec2(10.0, 5.0);
    sp.interact_size = egui::vec2(36.0, 24.0);
    sp.indent = 16.0;
    sp.menu_margin = Margin::same(6);
    sp.window_margin = Margin::same(14);
    sp.combo_width = 120.0;
    sp.text_edit_width = 220.0;
    sp.icon_width = 14.0;
    sp.icon_width_inner = 8.0;
    sp.icon_spacing = 6.0;
    sp.tooltip_width = 360.0;
    sp.menu_width = 180.0;
    sp.slider_width = 120.0;
    sp.scroll = ScrollStyle::thin();

    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    v.override_text_color = None;
    v.weak_text_color = Some(p.muted);
    v.panel_fill = p.canvas;
    v.window_fill = p.raised;
    v.window_stroke = Stroke::new(1.0, p.hairline);
    v.window_corner_radius = CornerRadius::same(10);
    v.window_shadow = p.shadow();
    v.popup_shadow = p.shadow();
    v.menu_corner_radius = CornerRadius::same(8);
    v.extreme_bg_color = p.surface;
    v.text_edit_bg_color = Some(p.surface);
    v.faint_bg_color = p.surface_alt;
    v.code_bg_color = p.surface_alt;
    v.hyperlink_color = p.info;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.bad;
    v.selection = Selection { bg_fill: p.selection, stroke: Stroke::new(1.0, p.text) };
    v.striped = false;
    v.slider_trailing_fill = true;
    v.collapsing_header_frame = false;
    v.window_highlight_topmost = false;
    let r = CornerRadius::same(6);
    v.widgets.noninteractive = WidgetVisuals {
        weak_bg_fill: p.surface,
        bg_fill: p.surface,
        bg_stroke: Stroke::new(1.0, p.hairline),
        fg_stroke: Stroke::new(1.0, p.text),
        corner_radius: r,
        expansion: 0.0,
    };
    v.widgets.inactive = WidgetVisuals {
        weak_bg_fill: p.surface,
        bg_fill: p.surface_alt,
        bg_stroke: Stroke::new(1.0, p.hairline),
        fg_stroke: Stroke::new(1.0, p.text),
        corner_radius: r,
        expansion: 0.0,
    };
    v.widgets.hovered = WidgetVisuals {
        weak_bg_fill: p.hover_wash,
        bg_fill: p.hover_wash,
        bg_stroke: Stroke::new(1.0, p.border),
        fg_stroke: Stroke::new(1.5, p.text),
        corner_radius: r,
        expansion: 0.0,
    };
    v.widgets.active = WidgetVisuals {
        weak_bg_fill: p.active_wash,
        bg_fill: p.active_wash,
        bg_stroke: Stroke::new(1.0, p.ink),
        fg_stroke: Stroke::new(2.0, p.text),
        corner_radius: r,
        expansion: 0.0,
    };
    v.widgets.open = WidgetVisuals {
        weak_bg_fill: p.surface_alt,
        bg_fill: p.surface,
        bg_stroke: Stroke::new(1.0, p.hairline),
        fg_stroke: Stroke::new(1.0, p.text),
        corner_radius: r,
        expansion: 0.0,
    };
    s.visuals = v;
}

/// What the client remembers between runs: preferences only, never data and never credentials.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientPrefs {
    pub theme: egui::ThemePreference,
}
