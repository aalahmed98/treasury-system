//! Chart helpers over egui_plot: thin marks, a recessive grid, palette series colours assigned in
//! fixed order, hover tooltips, and axes that read in both themes. Views pass pre-formatted data
//! and never compute inside the draw closure.

use egui::{Color32, Sense, Stroke, Ui};
use egui_plot::{Legend, Line, Plot, PlotPoints};

use crate::ui::theme::{Palette, pal};

fn quiet(plot: Plot<'_>) -> Plot<'_> {
    plot.allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .show_background(false)
        .show_grid([false, true])
}

/// Category bars drawn directly: thin rounded bars on a baseline, the category under each bar,
/// the value above it, and a tooltip on hover. No axis machinery, so labels always show and the
/// chart reads in both themes.
pub fn bars(ui: &mut Ui, data: &[(String, f64)], color: Color32, height: f32, fmt: fn(f64) -> String) {
    let p = pal(ui);
    if data.is_empty() {
        ui.label(egui::RichText::new("no data").color(p.muted));
        return;
    }
    let width = ui.available_width().max(80.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    let font = egui::FontId::proportional(11.0);
    let plot = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 16.0), egui::pos2(rect.right(), rect.bottom() - 16.0));
    let lo = data.iter().map(|(_, v)| *v).fold(0.0_f64, f64::min);
    let hi = data.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max);
    let span = (hi - lo).max(1e-9);
    let y_of = |v: f64| plot.bottom() - plot.height() * ((v - lo) / span) as f32;
    let base_y = y_of(0.0);
    painter.line_segment([egui::pos2(plot.left(), base_y), egui::pos2(plot.right(), base_y)], Stroke::new(1.0, p.axis));
    let slot = plot.width() / data.len() as f32;
    let bar_w = (slot * 0.6).clamp(6.0, 44.0);
    let hover = resp.hover_pos();
    let mut tip = None;
    for (i, (label, v)) in data.iter().enumerate() {
        let cx = plot.left() + slot * (i as f32 + 0.5);
        let y = y_of(*v);
        let (top, bottom) = if *v >= 0.0 { (y, base_y) } else { (base_y, y) };
        let bar = egui::Rect::from_min_max(egui::pos2(cx - bar_w / 2.0, top.min(bottom)), egui::pos2(cx + bar_w / 2.0, bottom.max(top)));
        let hovered = hover.is_some_and(|h| h.x >= cx - slot / 2.0 && h.x < cx + slot / 2.0);
        let radius = if *v >= 0.0 { egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 } } else { egui::CornerRadius { nw: 0, ne: 0, sw: 3, se: 3 } };
        painter.rect_filled(bar, radius, if hovered { color } else { color.gamma_multiply(0.85) });
        painter.text(egui::pos2(cx, rect.bottom()), egui::Align2::CENTER_BOTTOM, label, font.clone(), p.muted);
        if *v != 0.0 {
            // Always above the bar's upper edge: for a negative bar that is just above the
            // baseline, so the value never collides with the category label underneath.
            painter.text(egui::pos2(cx, top.min(bottom) - 1.0), egui::Align2::CENTER_BOTTOM, fmt(*v), font.clone(), ui.visuals().text_color());
        }
        if hovered {
            tip = Some(format!("{label}: {}", fmt(*v)));
        }
    }
    if let Some(t) = tip {
        resp.on_hover_text(t);
    }
}

/// One or more lines over a numeric x axis. Two or more series get a legend.
#[allow(dead_code)]
pub fn lines(ui: &mut Ui, id: &str, series: &[(String, Vec<[f64; 2]>)], height: f32, y_fmt: fn(f64) -> String, x_fmt: impl Fn(f64) -> String + 'static) {
    if series.iter().all(|(_, pts)| pts.is_empty()) {
        ui.label(egui::RichText::new("no data").color(pal(ui).muted));
        return;
    }
    let p = pal(ui);
    let mut plot = quiet(Plot::new(id)).height(height).y_axis_formatter(move |m, _| y_fmt(m.value)).x_axis_formatter(move |m, _| x_fmt(m.value));
    if series.len() >= 2 {
        plot = plot.legend(Legend::default());
    }
    plot.show(ui, |pu| {
        for (i, (name, pts)) in series.iter().enumerate() {
            pu.line(Line::new(name.clone(), PlotPoints::from(pts.clone())).color(p.series[i % p.series.len()]).width(2.0));
        }
    });
}

/// A tile-sized trend line with no axes.
#[allow(dead_code)]
pub fn sparkline(ui: &mut Ui, values: &[f64], color: Color32, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if values.len() < 2 {
        return;
    }
    let (min, max) = values.iter().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let span = if (max - min).abs() < f64::EPSILON { 1.0 } else { max - min };
    let pts: Vec<egui::Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = rect.left() + rect.width() * i as f32 / (values.len() - 1) as f32;
            let y = rect.bottom() - rect.height() * ((v - min) / span) as f32;
            egui::pos2(x, y)
        })
        .collect();
    ui.painter().add(egui::Shape::line(pts, Stroke::new(1.5, color)));
}

/// A horizontal bar from zero to `max` filled to `value`, with a hard limit mark and an optional
/// soft trigger mark. The fill turns negative below the limit and warning below the trigger.
pub fn limit_gauge(ui: &mut Ui, value: f64, limit: f64, trigger: Option<f64>, max: f64, p: &Palette, width: f32) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 14.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, p.card_stroke);
    let max = max.max(value).max(limit).max(1e-9);
    let x = |v: f64| rect.left() + rect.width() * (v / max).clamp(0.0, 1.0) as f32;
    let fill = if value < limit {
        p.negative
    } else if trigger.is_some_and(|t| value < t) {
        p.warning
    } else {
        p.positive
    };
    painter.rect_filled(egui::Rect::from_min_max(rect.min, egui::pos2(x(value), rect.max.y)), 3.0, fill);
    painter.line_segment([egui::pos2(x(limit), rect.top() - 2.0), egui::pos2(x(limit), rect.bottom() + 2.0)], Stroke::new(2.0, p.negative));
    if let Some(t) = trigger {
        painter.line_segment([egui::pos2(x(t), rect.top() - 2.0), egui::pos2(x(t), rect.bottom() + 2.0)], Stroke::new(1.5, p.warning));
    }
    resp.on_hover_text(match trigger {
        Some(t) => format!("value {value:.1} · trigger {t:.1} · limit {limit:.1}"),
        None => format!("value {value:.1} · limit {limit:.1}"),
    });
}
