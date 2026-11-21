//! Timeline and Xsheet: two views over the same exposure data.
//! Click a cell to go to that frame and select that layer (D8.9).

use eframe::egui::{self, Align2, Pos2, Rect, Sense, Stroke, Vec2};
use symphony_core::anim::Value;

use crate::icons::{self, Icon};
use crate::keymap::{self, Action, Profile};
use crate::scene::Scene;
use crate::theme;

/// Transport bar shared by both views.
pub fn transport(ui: &mut egui::Ui, scene: &mut Scene, profile: Profile) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if icons::button(ui, Icon::FirstFrame, false, true, "First frame").clicked() {
            scene.frame = 1;
        }
        if icons::button(
            ui,
            Icon::PrevFrame,
            false,
            true,
            &keymap::tooltip(profile, Action::PrevFrame),
        )
        .clicked()
        {
            scene.frame = scene.frame.saturating_sub(1).max(1);
        }
        let icon = if scene.playing {
            Icon::Pause
        } else {
            Icon::Play
        };
        if icons::button(
            ui,
            icon,
            scene.playing,
            true,
            &keymap::tooltip(profile, Action::PlayPause),
        )
        .clicked()
        {
            scene.playing = !scene.playing;
        }
        if icons::button(
            ui,
            Icon::NextFrame,
            false,
            true,
            &keymap::tooltip(profile, Action::NextFrame),
        )
        .clicked()
        {
            scene.frame = (scene.frame + 1).min(scene.length);
        }
        if icons::button(ui, Icon::LastFrame, false, true, "Last frame").clicked() {
            scene.frame = scene.length;
        }
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add(
            egui::DragValue::new(&mut scene.frame)
                .range(1..=scene.length)
                .prefix("Frame "),
        );
        ui.add(
            egui::DragValue::new(&mut scene.length)
                .range(1..=10_000)
                .prefix("Length "),
        );
        ui.add(
            egui::DragValue::new(&mut scene.fps)
                .range(1.0..=120.0)
                .suffix(" fps"),
        );
    });
}

fn peg_has_key(scene: &Scene, layer: usize, frame: u32) -> bool {
    let Some(p) = scene.peg_params(layer) else {
        return false;
    };
    let vals = [
        Some(&p.x),
        Some(&p.y),
        Some(&p.rotation),
        p.scale_x.as_ref(),
        p.scale_y.as_ref(),
    ];
    vals.into_iter()
        .flatten()
        .any(|v| matches!(v, Value::Keys(k) if k.iter().any(|(f, _)| *f == frame)))
}

/// Keyframe marker: amber diamond (D4.1 amber = time, D4.5 shape too).
fn key_diamond(painter: &egui::Painter, c: Pos2, r: f32) {
    let pts = vec![
        c + Vec2::new(0.0, -r),
        c + Vec2::new(r, 0.0),
        c + Vec2::new(0.0, r),
        c + Vec2::new(-r, 0.0),
    ];
    painter.add(egui::Shape::convex_polygon(
        pts,
        theme::ACCENT_TIME,
        Stroke::NONE,
    ));
}

pub fn timeline(ui: &mut egui::Ui, scene: &mut Scene, profile: Profile) {
    transport(ui, scene, profile);
    ui.separator();
    const HEAD: f32 = 140.0;
    const CW: f32 = 12.0;
    const RULER: f32 = 20.0;
    let rh = theme::ROW;
    let rows = scene.layers.len();
    let size = Vec2::new(
        HEAD + scene.length as f32 * CW + 8.0,
        RULER + rows as f32 * rh + 4.0,
    );

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
            let o = resp.rect.min;
            let frame_x = |f: u32| o.x + HEAD + (f - 1) as f32 * CW;

            // Ruler.
            for f in 1..=scene.length {
                let x = frame_x(f);
                let major = f % 5 == 0 || f == 1;
                let top = if major { 6.0 } else { 13.0 };
                painter.line_segment(
                    [Pos2::new(x, o.y + top), Pos2::new(x, o.y + RULER)],
                    Stroke::new(1.0, theme::LINE_STRONG),
                );
                if major {
                    painter.text(
                        Pos2::new(x + 2.0, o.y),
                        Align2::LEFT_TOP,
                        f.to_string(),
                        theme::numbers(),
                        theme::TEXT_SECONDARY,
                    );
                }
            }

            for (i, layer) in scene.layers.iter().enumerate() {
                let y = o.y + RULER + i as f32 * rh;
                let sel = i == scene.selected;
                let row = Rect::from_min_size(Pos2::new(o.x, y), Vec2::new(size.x, rh - 1.0));
                painter.rect_filled(
                    row,
                    0.0,
                    if sel {
                        theme::BG_HOVER
                    } else {
                        theme::BG_RAISED
                    },
                );
                let eye = if layer.visible {
                    Icon::Eye
                } else {
                    Icon::EyeOff
                };
                let eye_rect =
                    Rect::from_center_size(Pos2::new(o.x + 12.0, y + rh / 2.0), Vec2::splat(16.0));
                let eye_col = if layer.visible {
                    theme::TEXT_PRIMARY
                } else {
                    theme::TEXT_DISABLED
                };
                icons::paint(&painter, eye_rect, eye, eye_col);
                painter.text(
                    Pos2::new(o.x + 28.0, y + rh / 2.0),
                    Align2::LEFT_CENTER,
                    &layer.name,
                    theme::body(),
                    theme::TEXT_PRIMARY,
                );

                // Exposure bars: one bar per run of the same drawing.
                if let Some(col) = scene.xsheet.column(&layer.name) {
                    let mut f = 1;
                    while f <= scene.length {
                        let Some(d) = col.get(f) else {
                            f += 1;
                            continue;
                        };
                        let start = f;
                        while f <= scene.length && col.get(f) == Some(d) {
                            f += 1;
                        }
                        let bar = Rect::from_min_max(
                            Pos2::new(frame_x(start) + 1.0, y + 4.0),
                            Pos2::new(frame_x(f) - 1.0, y + rh - 5.0),
                        );
                        let (fill, text) = if sel {
                            (theme::ACCENT_SELECT_FILL, theme::ON_ACCENT)
                        } else {
                            (theme::LINE_STRONG, theme::TEXT_PRIMARY)
                        };
                        painter.rect_filled(bar, theme::RADIUS_CONTROL as f32, fill);
                        painter.text(
                            bar.left_center() + Vec2::new(3.0, 0.0),
                            Align2::LEFT_CENTER,
                            d,
                            theme::dense(),
                            text,
                        );
                    }
                }
                for f in 1..=scene.length {
                    if peg_has_key(scene, i, f) {
                        key_diamond(
                            &painter,
                            Pos2::new(frame_x(f) + CW / 2.0, y + rh - 5.0),
                            3.5,
                        );
                    }
                }
            }

            // Playhead (amber = time).
            let cx = frame_x(scene.frame) + CW / 2.0;
            painter.line_segment(
                [Pos2::new(cx, o.y), Pos2::new(cx, o.y + size.y)],
                Stroke::new(2.0, theme::ACCENT_TIME),
            );

            if let Some(p) = resp.interact_pointer_pos() {
                let rel = p - o;
                if rel.x >= HEAD {
                    scene.frame = (((rel.x - HEAD) / CW) as u32 + 1).clamp(1, scene.length);
                }
                if rel.y >= RULER && (resp.clicked() || resp.drag_started()) {
                    let row = ((rel.y - RULER) / rh) as usize;
                    if row < scene.layers.len() {
                        if rel.x < 24.0 && resp.clicked() {
                            scene.layers[row].visible = !scene.layers[row].visible;
                        }
                        scene.selected = row;
                    }
                }
            }
        });
}

pub fn xsheet(ui: &mut egui::Ui, scene: &mut Scene, profile: Profile) {
    transport(ui, scene, profile);
    ui.separator();
    const NUM: f32 = 40.0;
    const CW: f32 = 64.0;
    const HEAD: f32 = 24.0;
    let rh = theme::XSHEET_ROW;
    let cols = scene.layers.len();
    let size = Vec2::new(
        NUM + cols as f32 * CW + 4.0,
        HEAD + scene.length as f32 * rh + 4.0,
    );

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
            let o = resp.rect.min;

            for f in 1..=scene.length {
                let y = o.y + HEAD + (f - 1) as f32 * rh;
                let row = Rect::from_min_size(Pos2::new(o.x, y), Vec2::new(size.x, rh));
                let current = f == scene.frame;
                painter.rect_filled(
                    row,
                    0.0,
                    if f % 2 == 0 {
                        theme::BG_RAISED
                    } else {
                        theme::BG_PANEL
                    },
                );
                if current {
                    // Current frame: amber edge, not a fill, so cell text keeps its contrast.
                    painter.line_segment(
                        [row.left_top(), row.right_top()],
                        Stroke::new(1.0, theme::ACCENT_TIME),
                    );
                    painter.line_segment(
                        [row.left_bottom(), row.right_bottom()],
                        Stroke::new(1.0, theme::ACCENT_TIME),
                    );
                }
                let num_col = if current {
                    theme::ACCENT_TIME
                } else {
                    theme::TEXT_SECONDARY
                };
                painter.text(
                    Pos2::new(o.x + NUM - 6.0, y + rh / 2.0),
                    Align2::RIGHT_CENTER,
                    f.to_string(),
                    theme::numbers(),
                    num_col,
                );
            }

            for (i, layer) in scene.layers.iter().enumerate() {
                let x = o.x + NUM + i as f32 * CW;
                let sel = i == scene.selected;
                let head = Rect::from_min_size(Pos2::new(x, o.y), Vec2::new(CW - 1.0, HEAD - 2.0));
                painter.rect_filled(
                    head,
                    0.0,
                    if sel {
                        theme::ACCENT_SELECT_FILL
                    } else {
                        theme::BG_HOVER
                    },
                );
                let head_text = if sel {
                    theme::ON_ACCENT
                } else {
                    theme::TEXT_PRIMARY
                };
                painter.text(
                    head.center(),
                    Align2::CENTER_CENTER,
                    &layer.name,
                    theme::body(),
                    head_text,
                );
                painter.line_segment(
                    [Pos2::new(x, o.y), Pos2::new(x, o.y + size.y)],
                    Stroke::new(1.0, theme::LINE_SUBTLE),
                );

                let Some(col) = scene.xsheet.column(&layer.name) else {
                    continue;
                };
                for f in 1..=scene.length {
                    let Some(d) = col.get(f) else { continue };
                    let y = o.y + HEAD + (f - 1) as f32 * rh;
                    if f > 1 && col.get(f - 1) == Some(d) {
                        // Held cell: a vertical line, like a paper exposure sheet.
                        let cx = x + CW / 2.0;
                        painter.line_segment(
                            [Pos2::new(cx, y), Pos2::new(cx, y + rh)],
                            Stroke::new(1.5, theme::TEXT_SECONDARY),
                        );
                    } else {
                        painter.text(
                            Pos2::new(x + CW / 2.0, y + rh / 2.0),
                            Align2::CENTER_CENTER,
                            d,
                            theme::numbers(),
                            theme::TEXT_PRIMARY,
                        );
                    }
                }
            }

            if let Some(p) = resp.interact_pointer_pos() {
                let rel = p - o;
                if rel.y >= HEAD {
                    scene.frame = (((rel.y - HEAD) / rh) as u32 + 1).clamp(1, scene.length);
                }
                if rel.x >= NUM && (resp.clicked() || resp.drag_started()) {
                    let c = ((rel.x - NUM) / CW) as usize;
                    if c < scene.layers.len() {
                        scene.selected = c;
                    }
                }
            }
        });
}
