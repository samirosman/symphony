//! Waiting, made visible (DESIGN §17): one spinner, one progress bar and a
//! loading screen, shared by the app and the launcher (D17.4).

use std::f32::consts::TAU;

use egui::{self, Pos2, Rect, Response, Sense, Shape, Stroke, Vec2};

use crate::theme;

/// One full turn every 1.1 s: steady and calm.
const TURN_SECONDS: f64 = 1.1;

fn reduced_motion(ui: &egui::Ui) -> bool {
    ui.style().animation_time <= 0.0
}

/// The Symphony spinner: a cel-blue arc turning steadily (D17.4). With
/// reduced motion it's a still arc (D17.9).
pub fn spinner(ui: &mut egui::Ui, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_spinner(ui, rect);
    response
}

pub fn paint_spinner(ui: &egui::Ui, rect: Rect) {
    let still = reduced_motion(ui);
    let t = if still { 0.0 } else { ui.input(|i| i.time) };
    let start = (t / TURN_SECONDS).fract() as f32 * TAU;
    let r = rect.width().min(rect.height()) / 2.0 - 1.5;
    let c = rect.center();
    let track: Vec<Pos2> = (0..=32)
        .map(|i| c + Vec2::angled(i as f32 / 32.0 * TAU) * r)
        .collect();
    ui.painter()
        .add(Shape::line(track, Stroke::new(2.0, theme::LINE_SUBTLE)));
    let arc: Vec<Pos2> = (0..=24)
        .map(|i| c + Vec2::angled(start + i as f32 / 24.0 * TAU * 0.7) * r)
        .collect();
    ui.painter()
        .add(Shape::line(arc, Stroke::new(2.0, theme::ACCENT_SELECT)));
    if !still {
        ui.ctx().request_repaint();
    }
}

/// The Symphony progress bar (D17.3). `fraction` = None is indeterminate:
/// a segment sweeps across, for work that can't be counted.
pub fn progress_bar(ui: &mut egui::Ui, fraction: Option<f32>, width: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 6.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, theme::BG_INPUT);
    match fraction {
        Some(f) => {
            let w = rect.width() * f.clamp(0.0, 1.0);
            if w > 0.5 {
                p.rect_filled(
                    Rect::from_min_size(rect.min, Vec2::new(w, rect.height())),
                    3.0,
                    theme::ACCENT_SELECT,
                );
            }
        }
        None => {
            let t = if reduced_motion(ui) {
                0.25
            } else {
                (ui.input(|i| i.time) / 1.4).fract() as f32
            };
            let seg = rect.width() * 0.3;
            let x = rect.left() - seg + (rect.width() + seg) * t;
            let r = Rect::from_min_max(
                Pos2::new(x.max(rect.left()), rect.top()),
                Pos2::new((x + seg).min(rect.right()), rect.bottom()),
            );
            if r.width() > 0.5 {
                p.rect_filled(r, 3.0, theme::ACCENT_SELECT);
            }
            ui.ctx().request_repaint();
        }
    }
    response
}

/// Elapsed time in words, shown once a wait passes 3 s (DESIGN §17.1).
pub fn elapsed_label(seconds: f64) -> Option<String> {
    (seconds >= 3.0).then(|| {
        let s = seconds as u64;
        if s < 60 {
            format!("{s} s")
        } else {
            format!("{} min {:02} s", s / 60, s % 60)
        }
    })
}

/// A designed loading screen (D17.7): the Symphony name, the current step
/// in words, progress, and elapsed time once it passes 3 s. Fills `ui`.
pub fn loading_screen(
    ui: &mut egui::Ui,
    step: &str,
    detail: Option<&str>,
    fraction: Option<f32>,
    elapsed: f64,
) {
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, theme::BG_BASE);
    let card = Rect::from_center_size(
        rect.center(),
        Vec2::new(360.0_f32.min(rect.width() - 32.0), 150.0),
    );
    ui.painter()
        .rect_filled(card, theme::RADIUS_CARD as f32, theme::BG_PANEL);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(card.shrink(24.0)));
    child.vertical(|ui| {
        ui.label(
            egui::RichText::new("Symphony")
                .font(egui::FontId::proportional(theme::SIZE_HEADING))
                .color(theme::TEXT_PRIMARY),
        );
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            spinner(ui, 16.0);
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(step)
                    .font(theme::body())
                    .color(theme::TEXT_PRIMARY),
            );
        });
        ui.add_space(10.0);
        progress_bar(ui, fraction, ui.available_width());
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if let Some(d) = detail {
                ui.label(
                    egui::RichText::new(d)
                        .font(theme::dense())
                        .color(theme::TEXT_SECONDARY),
                );
            }
            if let Some(e) = elapsed_label(elapsed) {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(e)
                            .font(theme::dense())
                            .color(theme::TEXT_SECONDARY),
                    );
                });
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::elapsed_label;

    #[test]
    fn elapsed_shows_only_after_three_seconds() {
        assert_eq!(elapsed_label(1.0), None);
        assert_eq!(elapsed_label(12.4).as_deref(), Some("12 s"));
        assert_eq!(elapsed_label(75.0).as_deref(), Some("1 min 15 s"));
    }
}
