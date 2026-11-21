//! Timeline and Xsheet: two views over the same exposure data.
//! Click a cell to go to that frame and select that layer (D8.9).
//! The amber playhead and frame tag are Symphony's signature (D3.9).

use eframe::egui::{self, Align2, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};
use symphony_core::anim::Value;
use symphony_ui::icons::{self, Icon};
use symphony_ui::theme;

use crate::keymap::{self, Action, Profile};
use crate::scene::Scene;

/// View header: transport and timing fields (D3.2).
pub fn transport(ui: &mut egui::Ui, scene: &mut Scene, profile: Profile) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(theme::CARD_PAD, 6))
        .show(ui, |ui| {
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
                ui.add_space(16.0);
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

/// Keyframe: amber diamond (amber = time, and a shape as well, D4.5).
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
        Stroke::new(1.0, theme::BG_PANEL),
    ));
}

/// The amber frame tag on a ruler (D3.9).
fn frame_tag(painter: &egui::Painter, centre: Pos2, frame: u32) {
    let text = frame.to_string();
    let w = 10.0 + 7.0 * text.len() as f32;
    let r = Rect::from_center_size(centre, Vec2::new(w, 16.0));
    painter.rect_filled(r, theme::RADIUS_CONTROL as f32, theme::ACCENT_TIME);
    painter.text(
        r.center(),
        Align2::CENTER_CENTER,
        text,
        theme::numbers(),
        theme::BG_BASE,
    );
}

pub fn timeline(ui: &mut egui::Ui, scene: &mut Scene, profile: Profile) {
    transport(ui, scene, profile);
    const HEAD: f32 = 176.0;
    const RULER: f32 = 28.0;
    let cw = theme::FRAME_CELL;
    let rh = theme::ROW;
    let rows = scene.layers.len();
    let width = HEAD + scene.length as f32 * cw + 16.0;

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let size = Vec2::new(
                width.max(ui.available_width()),
                (RULER + rows as f32 * rh + 8.0).max(ui.available_height()),
            );
            let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
            let o = resp.rect.min;
            let frame_x = |f: u32| o.x + HEAD + (f - 1) as f32 * cw;
            let rows_bottom = o.y + RULER + rows as f32 * rh;

            // Every 5th frame gets a faint column line, so timing reads at a glance.
            for f in (5..=scene.length).step_by(5) {
                let x = frame_x(f) + cw;
                painter.line_segment(
                    [Pos2::new(x, o.y + RULER), Pos2::new(x, rows_bottom)],
                    Stroke::new(1.0, theme::LINE_SUBTLE),
                );
            }

            // Ruler.
            for f in 1..=scene.length {
                let x = frame_x(f) + cw / 2.0;
                let major = f % 5 == 0 || f == 1;
                let top = if major { 16.0 } else { 21.0 };
                painter.line_segment(
                    [Pos2::new(x, o.y + top), Pos2::new(x, o.y + RULER - 2.0)],
                    Stroke::new(1.0, theme::LINE_STRONG),
                );
                if major {
                    painter.text(
                        Pos2::new(x, o.y + 4.0),
                        Align2::CENTER_TOP,
                        f.to_string(),
                        theme::numbers(),
                        theme::TEXT_SECONDARY,
                    );
                }
            }

            for (i, layer) in scene.layers.iter().enumerate() {
                let y = o.y + RULER + i as f32 * rh;
                let sel = i == scene.selected;
                let row = Rect::from_min_size(
                    Pos2::new(o.x + 4.0, y + 1.0),
                    Vec2::new(width - 8.0, rh - 2.0),
                );
                if sel {
                    painter.rect_filled(row, theme::RADIUS_CONTROL as f32, theme::BG_RAISED);
                    let edge = Rect::from_min_size(row.min, Vec2::new(3.0, row.height()));
                    painter.rect_filled(
                        edge,
                        CornerRadius {
                            nw: 2,
                            sw: 2,
                            ne: 0,
                            se: 0,
                        },
                        theme::ACCENT_SELECT,
                    );
                }
                let eye = if layer.visible {
                    Icon::Eye
                } else {
                    Icon::EyeOff
                };
                let eye_rect =
                    Rect::from_center_size(Pos2::new(o.x + 22.0, y + rh / 2.0), Vec2::splat(16.0));
                icons::paint(
                    &painter,
                    eye_rect,
                    eye,
                    if layer.visible {
                        theme::TEXT_SECONDARY
                    } else {
                        theme::TEXT_DISABLED
                    },
                );
                let name_col = if layer.visible {
                    theme::TEXT_PRIMARY
                } else {
                    theme::TEXT_SECONDARY
                };
                painter.text(
                    Pos2::new(o.x + 40.0, y + rh / 2.0),
                    Align2::LEFT_CENTER,
                    &layer.name,
                    theme::body(),
                    name_col,
                );

                // Exposure blocks: one per run of the same drawing, neutral so the
                // amber playhead stays the brightest thing on the timeline.
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
                        let block = Rect::from_min_max(
                            Pos2::new(frame_x(start) + 1.0, y + 5.0),
                            Pos2::new(frame_x(f) - 1.0, y + rh - 5.0),
                        );
                        let fill = if sel {
                            theme::LINE_STRONG
                        } else {
                            theme::BG_HOVER
                        };
                        painter.rect_filled(block, theme::RADIUS_CONTROL as f32, fill);
                        if block.width() > 14.0 {
                            painter.text(
                                block.left_center() + Vec2::new(5.0, 0.0),
                                Align2::LEFT_CENTER,
                                d,
                                theme::dense(),
                                theme::TEXT_PRIMARY,
                            );
                        }
                    }
                }
                for f in 1..=scene.length {
                    if peg_has_key(scene, i, f) {
                        key_diamond(
                            &painter,
                            Pos2::new(frame_x(f) + cw / 2.0, y + rh / 2.0),
                            4.5,
                        );
                    }
                }
            }

            // Playhead and frame tag (D3.9).
            let cx = frame_x(scene.frame) + cw / 2.0;
            painter.line_segment(
                [Pos2::new(cx, o.y + RULER - 4.0), Pos2::new(cx, rows_bottom)],
                Stroke::new(1.5, theme::ACCENT_TIME),
            );
            frame_tag(&painter, Pos2::new(cx, o.y + RULER - 12.0), scene.frame);

            if let Some(p) = resp.interact_pointer_pos() {
                let rel = p - o;
                if rel.x >= HEAD {
                    scene.frame = (((rel.x - HEAD) / cw) as u32 + 1).clamp(1, scene.length);
                }
                if rel.y >= RULER && (resp.clicked() || resp.drag_started()) {
                    let row = ((rel.y - RULER) / rh) as usize;
                    if row < scene.layers.len() {
                        if rel.x < 32.0 && resp.clicked() {
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
    const NUM: f32 = 48.0;
    const CW: f32 = 72.0;
    const HEAD: f32 = 28.0;
    let rh = theme::XSHEET_ROW;
    let cols = scene.layers.len();
    let width = NUM + cols as f32 * CW + 8.0;

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let size = Vec2::new(
                width.max(ui.available_width()),
                HEAD + scene.length as f32 * rh + 8.0,
            );
            let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
            let o = resp.rect.min;
            let row_y = |f: u32| o.y + HEAD + (f - 1) as f32 * rh;

            for f in 1..=scene.length {
                let y = row_y(f);
                if f % 2 == 0 {
                    painter.rect_filled(
                        Rect::from_min_size(
                            Pos2::new(o.x + NUM, y),
                            Vec2::new(cols as f32 * CW, rh),
                        ),
                        0.0,
                        theme::BG_RAISED,
                    );
                }
                if f % 8 == 0 {
                    // Paper exposure sheets mark every 8 frames; we keep that rhythm.
                    painter.line_segment(
                        [
                            Pos2::new(o.x + NUM, y + rh),
                            Pos2::new(o.x + NUM + cols as f32 * CW, y + rh),
                        ],
                        Stroke::new(1.0, theme::LINE_STRONG),
                    );
                }
                if f != scene.frame {
                    painter.text(
                        Pos2::new(o.x + NUM - 10.0, y + rh / 2.0),
                        Align2::RIGHT_CENTER,
                        f.to_string(),
                        theme::numbers(),
                        theme::TEXT_SECONDARY,
                    );
                }
            }

            for (i, layer) in scene.layers.iter().enumerate() {
                let x = o.x + NUM + i as f32 * CW;
                let sel = i == scene.selected;
                let head = Rect::from_min_size(
                    Pos2::new(x + 2.0, o.y + 2.0),
                    Vec2::new(CW - 4.0, HEAD - 6.0),
                );
                painter.rect_filled(
                    head,
                    theme::RADIUS_CONTROL as f32,
                    if sel {
                        theme::ACCENT_SELECT_FILL
                    } else {
                        theme::BG_RAISED
                    },
                );
                painter.text(
                    head.center(),
                    Align2::CENTER_CENTER,
                    &layer.name,
                    theme::body(),
                    if sel {
                        theme::ON_ACCENT
                    } else {
                        theme::TEXT_PRIMARY
                    },
                );
                painter.line_segment(
                    [Pos2::new(x, o.y + HEAD), Pos2::new(x, o.y + size.y)],
                    Stroke::new(1.0, theme::LINE_SUBTLE),
                );

                let Some(col) = scene.xsheet.column(&layer.name) else {
                    continue;
                };
                for f in 1..=scene.length {
                    let Some(d) = col.get(f) else { continue };
                    let y = row_y(f);
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

            // Current frame: amber line across the sheet and the frame tag (D3.9).
            let y = row_y(scene.frame) + rh / 2.0;
            painter.line_segment(
                [
                    Pos2::new(o.x + NUM, y),
                    Pos2::new(o.x + NUM + cols as f32 * CW, y),
                ],
                Stroke::new(1.0, theme::ACCENT_TIME.gamma_multiply(0.6)),
            );
            frame_tag(&painter, Pos2::new(o.x + NUM / 2.0 + 2.0, y), scene.frame);

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
