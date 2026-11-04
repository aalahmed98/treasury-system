//! Chart helpers: thin marks with gaps, a hairline baseline, labels in the muted ink, palette
//! series colours assigned in fixed order, hover tooltips, and axes that read in both themes.
//! Views pass pre-formatted data and never compute inside the draw closure.

use egui::{Color32, FontId, Sense, Stroke, Ui};
use egui_plot::{Legend, Line, Plot, PlotPoints};

use crate::ui::fmt::fmt_money_short;
use crate::ui::theme::{Palette, medium, pal};

fn quiet(plot: Plot<'_>) -> Plot<'_> {
    plot.allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .show_background(false)
        .show_grid([false, true])
}

fn label_font() -> FontId {
    FontId::new(11.0, egui::FontFamily::Proportional)
}

fn value_font() -> FontId {
    FontId::new(11.0, medium())
}

/// Category bars drawn directly: thin rounded bars on a baseline, the category under each bar,
/// the value above it, and a tooltip on hover. No axis machinery, so labels always show and the
/// chart reads in both themes.
pub fn bars(ui: &mut Ui, data: &[(String, f64)], color: Color32, height: f32, fmt: fn(f64) -> String) {
    let p = pal(ui);
    if data.is_empty() {
        ui.label(p.muted("no data"));
        return;
    }
    let width = ui.available_width().max(80.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    let plot = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 16.0), egui::pos2(rect.right(), rect.bottom() - 18.0));
    let lo = data.iter().map(|(_, v)| *v).fold(0.0_f64, f64::min);
    let hi = data.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max);
    let span = (hi - lo).max(1e-9);
    let y_of = |v: f64| plot.bottom() - plot.height() * ((v - lo) / span) as f32;
    let base_y = y_of(0.0);
    let slot = plot.width() / data.len() as f32;
    let bar_w = (slot * 0.56).clamp(6.0, 40.0);
    let hover = resp.hover_pos();
    let mut tip = None;
    for (i, (label, v)) in data.iter().enumerate() {
        let cx = plot.left() + slot * (i as f32 + 0.5);
        let y = y_of(*v);
        let (top, bottom) = if *v >= 0.0 { (y, base_y) } else { (base_y, y) };
        let bar = egui::Rect::from_min_max(egui::pos2(cx - bar_w / 2.0, top.min(bottom)), egui::pos2(cx + bar_w / 2.0, bottom.max(top)));
        let hovered = hover.is_some_and(|h| h.x >= cx - slot / 2.0 && h.x < cx + slot / 2.0);
        let radius = if *v >= 0.0 { egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 } } else { egui::CornerRadius { nw: 0, ne: 0, sw: 3, se: 3 } };
        painter.rect_filled(bar, radius, if hovered { color } else { color.gamma_multiply(0.88) });
        painter.text(egui::pos2(cx, rect.bottom()), egui::Align2::CENTER_BOTTOM, label, label_font(), p.muted);
        if *v != 0.0 {
            // Always above the bar's upper edge: for a negative bar that is just above the
            // baseline, so the value never collides with the category label underneath.
            painter.text(egui::pos2(cx, top.min(bottom) - 2.0), egui::Align2::CENTER_BOTTOM, fmt(*v), value_font(), p.text_2);
        }
        if hovered {
            tip = Some(format!("{label}: {}", fmt(*v)));
        }
    }
    painter.line_segment([egui::pos2(plot.left(), base_y), egui::pos2(plot.right(), base_y)], Stroke::new(1.0, p.border));
    if let Some(t) = tip {
        resp.on_hover_text(t);
    }
}

/// One or more lines over a numeric x axis. Two or more series get a legend.
pub fn lines(ui: &mut Ui, id: &str, series: &[(String, Vec<[f64; 2]>)], height: f32, y_fmt: fn(f64) -> String, x_fmt: impl Fn(f64) -> String + 'static) {
    if series.iter().all(|(_, pts)| pts.is_empty()) {
        ui.label(pal(ui).muted("no data"));
        return;
    }
    let p = pal(ui);
    let mut plot = quiet(Plot::new(id)).height(height).y_axis_formatter(move |m, _| y_fmt(m.value)).x_axis_formatter(move |m, _| x_fmt(m.value));
    if series.len() >= 2 {
        plot = plot.legend(Legend::default());
    }
    plot.show(ui, |pu| {
        for (i, (name, pts)) in series.iter().enumerate() {
            pu.line(Line::new(name.clone(), PlotPoints::from(pts.clone())).color(p.series[i % p.series.len()]).width(1.8));
        }
    });
}

/// A horizontal bar from zero to `max` filled to `value`, with a hard limit mark and an optional
/// soft trigger mark. For a floor (`floor = true`) the value must stay above the limit; for a
/// ceiling it must stay below. The fill turns bad on a breach and warn near the limit.
#[allow(clippy::too_many_arguments)]
pub fn limit_gauge(ui: &mut Ui, value: f64, limit: f64, trigger: Option<f64>, max: f64, floor: bool, p: &Palette, width: f32) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 16.0), Sense::hover());
    let painter = ui.painter();
    let track = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width(), 8.0));
    painter.rect_filled(track, 4.0, p.hairline);
    let max = max.max(value).max(limit).max(1e-9);
    let x = |v: f64| rect.left() + rect.width() * (v / max).clamp(0.0, 1.0) as f32;
    let breach = if floor { value < limit } else { value > limit };
    let warn = if floor { trigger.is_some_and(|t| value < t) } else { value > limit * 0.9 };
    let fill = if breach {
        p.bad
    } else if warn {
        p.warn
    } else {
        p.good
    };
    painter.rect_filled(egui::Rect::from_min_max(track.min, egui::pos2(x(value).max(track.left() + 4.0), track.max.y)), 4.0, fill);
    painter.line_segment([egui::pos2(x(limit), rect.top()), egui::pos2(x(limit), rect.bottom())], Stroke::new(2.0, p.text));
    if let Some(t) = trigger {
        painter.line_segment([egui::pos2(x(t), rect.top() + 2.0), egui::pos2(x(t), rect.bottom() - 2.0)], Stroke::new(1.5, p.muted));
    }
    resp.on_hover_text(match trigger {
        Some(t) => format!("value {value:.1} · trigger {t:.1} · limit {limit:.1}"),
        None => format!("value {value:.1} · limit {limit:.1}"),
    });
}

/// A waterfall: totals as full bars from the baseline, steps as floating bars from the running
/// level, each labelled with its value.
pub fn waterfall(ui: &mut Ui, steps: &[(String, f64, bool)], p: &Palette, height: f32) {
    if steps.is_empty() {
        ui.label(p.muted("no data"));
        return;
    }
    let width = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    let plot = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 16.0), egui::pos2(rect.right(), rect.bottom() - 18.0));
    // Running levels.
    let mut level = 0.0;
    let mut spans: Vec<(f64, f64)> = Vec::with_capacity(steps.len());
    for (_, v, total) in steps {
        if *total {
            spans.push((0.0, *v));
            level = *v;
        } else {
            spans.push((level, level + v));
            level += v;
        }
    }
    let lo = spans.iter().map(|(a, b)| a.min(*b)).fold(0.0_f64, f64::min);
    let hi = spans.iter().map(|(a, b)| a.max(*b)).fold(0.0_f64, f64::max);
    let span = (hi - lo).max(1e-9);
    let y_of = |v: f64| plot.bottom() - plot.height() * ((v - lo) / span) as f32;
    painter.line_segment([egui::pos2(plot.left(), y_of(0.0)), egui::pos2(plot.right(), y_of(0.0))], Stroke::new(1.0, p.border));
    let slot = plot.width() / steps.len() as f32;
    let bar_w = (slot * 0.56).clamp(6.0, 56.0);
    let hover = resp.hover_pos();
    let mut tip = None;
    for (i, ((label, v, total), (a, b))) in steps.iter().zip(spans.iter()).enumerate() {
        let cx = plot.left() + slot * (i as f32 + 0.5);
        let (y0, y1) = (y_of(*a), y_of(*b));
        let bar = egui::Rect::from_min_max(egui::pos2(cx - bar_w / 2.0, y0.min(y1)), egui::pos2(cx + bar_w / 2.0, y0.max(y1)));
        let color = if *total { p.series[0] } else if *v >= 0.0 { p.good } else { p.bad };
        let hovered = hover.is_some_and(|h| h.x >= cx - slot / 2.0 && h.x < cx + slot / 2.0);
        painter.rect_filled(bar, 3.0, if hovered { color } else { color.gamma_multiply(0.88) });
        if i + 1 < steps.len() {
            let next_cx = cx + slot;
            painter.line_segment([egui::pos2(cx + bar_w / 2.0, y_of(*b)), egui::pos2(next_cx - bar_w / 2.0, y_of(*b))], Stroke::new(1.0, p.border));
        }
        painter.text(egui::pos2(cx, rect.bottom()), egui::Align2::CENTER_BOTTOM, label, label_font(), p.muted);
        let text = if *total { fmt_money_short(*v) } else if *v >= 0.0 { format!("+{}", fmt_money_short(*v)) } else { fmt_money_short(*v) };
        painter.text(egui::pos2(cx, bar.top() - 2.0), egui::Align2::CENTER_BOTTOM, &text, value_font(), p.text_2);
        if hovered {
            tip = Some(format!("{label}: {text}"));
        }
    }
    if let Some(t) = tip {
        resp.on_hover_text(t);
    }
}

/// Two bars per category (for example actual against budget), with a small legend.
pub fn bars_pair(ui: &mut Ui, data: &[(String, f64, f64)], names: (&str, &str), p: &Palette, height: f32, fmt: fn(f64) -> String) {
    if data.is_empty() {
        ui.label(p.muted("no data"));
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, (name, color)) in [(names.0, p.series[0]), (names.1, p.series[3])].into_iter().enumerate() {
            if i > 0 {
                ui.add_space(8.0);
            }
            let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 14.0), Sense::hover());
            ui.painter().rect_filled(egui::Rect::from_center_size(r.center(), egui::vec2(10.0, 10.0)), 2.0, color);
            ui.label(p.small(name));
        }
    });
    let width = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    let plot = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 8.0), egui::pos2(rect.right(), rect.bottom() - 18.0));
    let lo = data.iter().flat_map(|(_, a, b)| [*a, *b]).fold(0.0_f64, f64::min);
    let hi = data.iter().flat_map(|(_, a, b)| [*a, *b]).fold(0.0_f64, f64::max);
    let span = (hi - lo).max(1e-9);
    let y_of = |v: f64| plot.bottom() - plot.height() * ((v - lo) / span) as f32;
    let base_y = y_of(0.0);
    let slot = plot.width() / data.len() as f32;
    let bar_w = (slot * 0.3).clamp(4.0, 26.0);
    let hover = resp.hover_pos();
    let mut tip = None;
    for (i, (label, a, b)) in data.iter().enumerate() {
        let cx = plot.left() + slot * (i as f32 + 0.5);
        for (j, (v, color)) in [(*a, p.series[0]), (*b, p.series[3])].into_iter().enumerate() {
            let x = cx + (j as f32 - 0.5) * (bar_w + 2.0);
            let y = y_of(v);
            let r = egui::Rect::from_min_max(egui::pos2(x - bar_w / 2.0, y.min(base_y)), egui::pos2(x + bar_w / 2.0, y.max(base_y)));
            painter.rect_filled(r, 2.0, color.gamma_multiply(0.9));
        }
        painter.text(egui::pos2(cx, rect.bottom()), egui::Align2::CENTER_BOTTOM, label, label_font(), p.muted);
        if hover.is_some_and(|h| h.x >= cx - slot / 2.0 && h.x < cx + slot / 2.0) {
            tip = Some(format!("{label}: {} {} · {} {}", names.0, fmt(*a), names.1, fmt(*b)));
        }
    }
    painter.line_segment([egui::pos2(plot.left(), base_y), egui::pos2(plot.right(), base_y)], Stroke::new(1.0, p.border));
    if let Some(t) = tip {
        resp.on_hover_text(t);
    }
}
