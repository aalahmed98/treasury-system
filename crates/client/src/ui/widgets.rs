//! The shared vocabulary of the client: cards, the stat strip, chips, tabs, badges, dots, table
//! headers and the primary button. Every page builds from these so the same thing looks the same
//! everywhere, in both themes.

use egui::{Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, Rect, Response, RichText, Sense, Stroke, StrokeKind, TextStyle, Ui};

use crate::ui::theme::{Palette, medium, pal, semibold};

/// The meaning a colour carries; never shown without a dot or a word beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Neutral,
    Good,
    Warn,
    Bad,
    Info,
}

impl Tone {
    pub fn color(self, p: &Palette) -> Color32 {
        match self {
            Tone::Neutral => p.text_2,
            Tone::Good => p.good,
            Tone::Warn => p.warn,
            Tone::Bad => p.bad,
            Tone::Info => p.info,
        }
    }

    pub fn soft(self, p: &Palette) -> Color32 {
        match self {
            Tone::Neutral => p.surface_alt,
            Tone::Good => p.good_soft,
            Tone::Warn => p.warn_soft,
            Tone::Bad => p.bad_soft,
            Tone::Info => p.info_soft,
        }
    }
}

// ---- containers -------------------------------------------------------------------------------

/// A card: surface, hairline, radius 8.
pub fn frame(p: &Palette) -> egui::Frame {
    egui::Frame::new().fill(p.surface).stroke(Stroke::new(1.0, p.hairline)).corner_radius(8).inner_margin(14)
}

/// A side panel's frame: the alternate surface with generous padding.
pub fn side_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new().fill(p.surface_alt).inner_margin(Margin { left: 16, right: 12, top: 14, bottom: 14 })
}

/// The page behind cards and tables.
pub fn canvas_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new().fill(p.canvas).inner_margin(Margin { left: 20, right: 20, top: 14, bottom: 8 })
}

/// A denser page frame for the table tabs.
pub fn table_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new().fill(p.canvas).inner_margin(Margin { left: 14, right: 14, top: 10, bottom: 6 })
}

/// A titled card of a fixed width; the height follows the content.
pub fn card(ui: &mut Ui, p: &Palette, title: &str, width: f32, body: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(egui::vec2(width + 30.0, 10.0), Layout::top_down(Align::Min), |ui| {
        frame(p).show(ui, |ui| {
            ui.set_width(width);
            ui.label(p.heading(title));
            ui.add_space(8.0);
            body(ui);
        });
    });
}

/// A row of cards that wraps and keeps every item's top on the same line.
pub fn wrap_row(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    ui.with_layout(Layout::left_to_right(Align::Min).with_main_wrap(true), body);
}

/// A section header: an eyebrow and its metadata, with air above it.
pub fn section(ui: &mut Ui, p: &Palette, title: &str, meta: &str) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.label(p.eyebrow(title));
        if !meta.is_empty() {
            ui.label(p.small(meta));
        }
    });
    ui.add_space(6.0);
}

// ---- stat strip -------------------------------------------------------------------------------

pub struct StatCell {
    pub label: String,
    pub value: String,
    pub sub: String,
    /// The standing dot beside the sub line, when the cell has one.
    pub dot: Option<Color32>,
    pub clickable: bool,
}

/// One surface with hairlines between the cells instead of a grid of identical tiles. Wraps into
/// further strips when the cells outnumber the width. Returns the index of a clicked cell.
pub fn stat_strip(ui: &mut Ui, p: &Palette, cells: &[StatCell], min_w: f32) -> Option<usize> {
    const H: f32 = 74.0;
    const PAD: f32 = 14.0;
    if cells.is_empty() {
        return None;
    }
    let avail = ui.available_width().max(min_w);
    let per_row = ((avail / min_w).floor() as usize).clamp(1, cells.len());
    let cell_w = (avail / per_row as f32).floor().min(250.0);
    let mut clicked = None;
    for (ri, chunk) in cells.chunks(per_row).enumerate() {
        if ri > 0 {
            ui.add_space(8.0);
        }
        let w = cell_w * chunk.len() as f32;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, H), Sense::click());
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let painter = ui.painter_at(rect);
        painter.rect(rect, 8.0, p.surface, Stroke::new(1.0, p.hairline), StrokeKind::Inside);
        let hover = resp.hover_pos();
        let mut any_hot = false;
        for (ci, c) in chunk.iter().enumerate() {
            let x0 = rect.left() + cell_w * ci as f32;
            let cr = Rect::from_min_size(egui::pos2(x0, rect.top()), egui::vec2(cell_w, H));
            let cp = painter.with_clip_rect(cr.shrink(1.0));
            let hot = c.clickable && hover.is_some_and(|h| cr.contains(h));
            any_hot |= hot;
            if hot {
                cp.rect_filled(cr.shrink(2.0), 6.0, p.hover_wash);
            }
            if ci > 0 {
                cp.vline(x0, (rect.top() + 12.0)..=(rect.bottom() - 12.0), Stroke::new(1.0, p.hairline));
            }
            let tx = x0 + PAD;
            cp.text(egui::pos2(tx, rect.top() + 11.0), Align2::LEFT_TOP, &c.label, FontId::new(11.0, medium()), p.muted);
            cp.text(egui::pos2(tx, rect.top() + 25.0), Align2::LEFT_TOP, &c.value, FontId::new(24.0, semibold()), p.text);
            let mut sx = tx;
            if let Some(d) = c.dot {
                cp.circle_filled(egui::pos2(sx + 3.5, rect.bottom() - 15.5), 3.5, d);
                sx += 12.0;
            }
            cp.text(egui::pos2(sx, rect.bottom() - 10.0), Align2::LEFT_BOTTOM, &c.sub, FontId::new(11.0, egui::FontFamily::Proportional), p.muted);
            if hot && resp.clicked() {
                clicked = Some(ri * per_row + ci);
            }
        }
        if any_hot {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    clicked
}

// ---- controls ---------------------------------------------------------------------------------

fn pill(ui: &mut Ui, text: &str, font: FontId, pad: egui::Vec2, height: f32) -> (Rect, Response, std::sync::Arc<egui::Galley>) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER);
    let size = egui::vec2(galley.size().x + pad.x * 2.0, height.max(galley.size().y + pad.y * 2.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    (rect, resp, galley)
}

/// A pill: ink when selected, a wash on hover, secondary text at rest.
pub fn chip(ui: &mut Ui, p: &Palette, selected: bool, text: &str) -> Response {
    chip_impl(ui, p, selected, text, false)
}

fn chip_impl(ui: &mut Ui, p: &Palette, selected: bool, text: &str, outlined: bool) -> Response {
    let (rect, resp, galley) = pill(ui, text, FontId::new(13.0, medium()), egui::vec2(10.0, 4.0), 26.0);
    if ui.is_rect_visible(rect) {
        let down = resp.is_pointer_button_down_on();
        let (fill, fg) = if selected {
            (p.ink, p.ink_text)
        } else if down {
            (p.active_wash, p.text)
        } else if resp.hovered() || resp.has_focus() {
            (p.hover_wash, p.text)
        } else {
            (Color32::TRANSPARENT, p.text_2)
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 13.0, fill);
        if outlined && !selected {
            painter.rect_stroke(rect, 13.0, Stroke::new(1.0, p.border), StrokeKind::Inside);
        }
        if resp.has_focus() {
            painter.rect_stroke(rect, 13.0, Stroke::new(1.5, p.text), StrokeKind::Outside);
        }
        let pos = rect.center() - galley.size() / 2.0;
        painter.galley(pos, galley, fg);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A chip that toggles a flag.
pub fn chip_toggle(ui: &mut Ui, p: &Palette, on: &mut bool, text: &str) -> Response {
    let resp = chip_impl(ui, p, *on, text, true);
    if resp.clicked() {
        *on = !*on;
    }
    resp
}

/// A top-bar tab: text with a 2 px ink underline when active, and an optional badge.
pub fn tab(ui: &mut Ui, p: &Palette, selected: bool, text: &str, badge: Option<(&str, Tone)>) -> Response {
    let font = FontId::new(13.0, medium());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER);
    let bfont = FontId::new(11.0, medium());
    let bgalley = badge.map(|(t, _)| ui.painter().layout_no_wrap(t.to_owned(), bfont, Color32::PLACEHOLDER));
    let pad = 10.0;
    let bw = bgalley.as_ref().map(|g| g.size().x + 12.0 + 6.0).unwrap_or(0.0);
    let size = egui::vec2(galley.size().x + bw + pad * 2.0, 40.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if resp.hovered() && !selected {
            painter.rect_filled(rect.shrink2(egui::vec2(2.0, 7.0)), 6.0, p.hover_wash);
        }
        let fg = if selected || resp.hovered() { p.text } else { p.text_2 };
        let y = rect.center().y - galley.size().y / 2.0;
        painter.galley(egui::pos2(rect.left() + pad, y), galley.clone(), fg);
        if let (Some(g), Some((_, tone))) = (bgalley, badge) {
            let x = rect.left() + pad + galley.size().x + 6.0;
            let r = Rect::from_min_size(egui::pos2(x, rect.center().y - 9.0), egui::vec2(g.size().x + 12.0, 18.0));
            painter.rect_filled(r, 9.0, tone.soft(p));
            painter.galley(r.center() - g.size() / 2.0, g, tone.color(p));
        }
        if selected {
            painter.rect_filled(Rect::from_min_max(egui::pos2(rect.left() + 4.0, rect.bottom() - 2.0), egui::pos2(rect.right() - 4.0, rect.bottom())), 1.0, p.ink);
        }
        if resp.has_focus() {
            painter.rect_stroke(rect.shrink2(egui::vec2(2.0, 7.0)), 6.0, Stroke::new(1.5, p.text), StrokeKind::Outside);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small pill in a tone's soft colour with its strong text.
pub fn badge(ui: &mut Ui, p: &Palette, text: &str, tone: Tone) -> Response {
    let (rect, resp, galley) = pill(ui, text, FontId::new(11.0, medium()), egui::vec2(7.0, 2.0), 18.0);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, 9.0, tone.soft(p));
        if tone == Tone::Neutral {
            painter.rect_stroke(rect, 9.0, Stroke::new(1.0, p.hairline), StrokeKind::Inside);
        }
        painter.galley(rect.center() - galley.size() / 2.0, galley, tone.color(p));
    }
    resp
}

/// A badge with a dot: the standing of a metric, the severity of an item.
pub fn status_badge(ui: &mut Ui, p: &Palette, text: &str, tone: Tone) -> Response {
    let (rect, resp, galley) = pill(ui, text, FontId::new(11.0, medium()), egui::vec2(7.0, 2.0), 18.0);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let r = Rect::from_min_max(rect.min, egui::pos2(rect.right() + 11.0, rect.bottom()));
        painter.rect_filled(r, 9.0, tone.soft(p));
        painter.circle_filled(egui::pos2(rect.left() + 9.0, rect.center().y), 3.0, tone.color(p));
        painter.galley(egui::pos2(rect.left() + 16.0, rect.center().y - galley.size().y / 2.0), galley, tone.color(p));
    }
    ui.add_space(11.0);
    resp
}

/// A 7 px dot, for a status beside its label.
pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 14.0), Sense::hover());
    ui.painter().circle_filled(egui::pos2(rect.left() + 3.5, rect.center().y + 0.5), 3.5, color);
}

/// A table header cell.
pub fn th(ui: &mut Ui, text: &str) -> Response {
    let p = pal(ui);
    ui.label(p.label(text))
}

/// A sortable table header cell; `on` marks the active sort.
pub fn th_sort(ui: &mut Ui, on: bool, text: &str) -> Response {
    let p = pal(ui);
    let color = if on { p.text } else { p.text_2 };
    ui.add(egui::Label::new(RichText::new(text).text_style(TextStyle::Name("label".into())).color(color)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The one primary action on a screen: ink fill, light text.
pub fn primary(ui: &mut Ui, p: &Palette, enabled: bool, text: &str) -> Response {
    let (fill, fg) = if enabled { (p.ink, p.ink_text) } else { (p.hover_wash, p.faint) };
    let b = egui::Button::new(RichText::new(text).color(fg).family(medium())).fill(fill).stroke(Stroke::NONE).corner_radius(CornerRadius::same(6)).min_size(egui::vec2(0.0, 28.0));
    ui.add_enabled(enabled, b)
}

/// The primary action at a given size, for a form.
pub fn primary_sized(ui: &mut Ui, p: &Palette, enabled: bool, text: &str, size: egui::Vec2) -> Response {
    let (fill, fg) = if enabled { (p.ink, p.ink_text) } else { (p.hover_wash, p.faint) };
    let b = egui::Button::new(RichText::new(text).color(fg).family(medium())).fill(fill).stroke(Stroke::NONE).corner_radius(CornerRadius::same(6)).min_size(size);
    ui.add_enabled(enabled, b)
}

/// A quiet secondary button.
pub fn secondary(ui: &mut Ui, p: &Palette, enabled: bool, text: &str) -> Response {
    let b = egui::Button::new(RichText::new(text).family(medium()).color(if enabled { p.text } else { p.faint })).min_size(egui::vec2(0.0, 28.0));
    ui.add_enabled(enabled, b)
}

/// A segmented control over the theme preference.
pub fn theme_switch(ui: &mut Ui, p: &Palette) {
    let current = ui.ctx().options(|o| o.theme_preference);
    let mut pick = None;
    // In a right-to-left cluster the options are added in reverse so they still read Auto, Light, Dark.
    let rtl = ui.layout().main_dir() == egui::Direction::RightToLeft;
    let mut options = [(egui::ThemePreference::System, "Auto"), (egui::ThemePreference::Light, "Light"), (egui::ThemePreference::Dark, "Dark")];
    if rtl {
        options.reverse();
    }
    egui::Frame::new().fill(p.surface_alt).stroke(Stroke::new(1.0, p.hairline)).corner_radius(14).inner_margin(2).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.horizontal(|ui| {
            for (pref, text) in options {
                if small_chip(ui, p, current == pref, text).clicked() {
                    pick = Some(pref);
                }
            }
        });
    });
    if let Some(pref) = pick {
        ui.ctx().set_theme(pref);
    }
}

/// A chip at the small size, for controls inside the top bar.
pub fn small_chip(ui: &mut Ui, p: &Palette, selected: bool, text: &str) -> Response {
    let (rect, resp, galley) = pill(ui, text, FontId::new(11.0, medium()), egui::vec2(8.0, 3.0), 22.0);
    if ui.is_rect_visible(rect) {
        let (fill, fg) = if selected {
            (p.ink, p.ink_text)
        } else if resp.hovered() {
            (p.hover_wash, p.text)
        } else {
            (Color32::TRANSPARENT, p.text_2)
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 11.0, fill);
        painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
