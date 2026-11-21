//! Timeline and Xsheet: two views over the same exposure data.
//! Click a cell to go to that frame and select that layer.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use symphony_core::anim::Value;

use crate::scene::Scene;

const CURRENT: Color32 = Color32::from_rgb(220, 60, 60);
const EXPOSED: Color32 = Color32::from_rgb(112, 132, 170);
const EXPOSED_SEL: Color32 = Color32::from_rgb(90, 140, 230);

/// Transport bar shared by both views.
pub fn transport(ui: &mut egui::Ui, scene: &mut Scene) {
    ui.horizontal(|ui| {
        if ui.button("⏮").on_hover_text("First frame").clicked() {
            scene.frame = 1;
        }
        if ui
            .button("◀")
            .on_hover_text("Previous frame ( , )")
            .clicked()
        {
            scene.frame = scene.frame.saturating_sub(1).max(1);
        }
        let play = if scene.playing { "⏸" } else { "▶" };
        if ui.button(play).on_hover_text("Play / pause").clicked() {
            scene.playing = !scene.playing;
        }
        if ui.button("▶|").on_hover_text("Next frame ( . )").clicked() {
            scene.frame = (scene.frame + 1).min(scene.length);
        }
        if ui.button("⏭").on_hover_text("Last frame").clicked() {
            scene.frame = scene.length;
        }
        ui.separator();
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

pub fn timeline(ui: &mut egui::Ui, scene: &mut Scene) {
    transport(ui, scene);
    ui.separator();
    const HEAD: f32 = 140.0;
    const CW: f32 = 14.0;
    const RH: f32 = 24.0;
    const RULER: f32 = 18.0;
    let rows = scene.layers.len();
    let size = Vec2::new(
        HEAD + scene.length as f32 * CW + 8.0,
        RULER + rows as f32 * RH + 4.0,
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
                painter.line_segment(
                    [
                        Pos2::new(x, o.y + if major { 4.0 } else { 11.0 }),
                        Pos2::new(x, o.y + RULER),
                    ],
                    Stroke::new(1.0, Color32::from_gray(110)),
                );
                if major {
                    painter.text(
                        Pos2::new(x + 2.0, o.y),
                        Align2::LEFT_TOP,
                        f.to_string(),
                        FontId::proportional(10.0),
                        Color32::from_gray(190),
                    );
                }
            }

            for (i, layer) in scene.layers.iter().enumerate() {
                let y = o.y + RULER + i as f32 * RH;
                let sel = i == scene.selected;
                let row = Rect::from_min_size(Pos2::new(o.x, y), Vec2::new(size.x, RH - 1.0));
                painter.rect_filled(
                    row,
                    0.0,
                    if sel {
                        Color32::from_gray(62)
                    } else {
                        Color32::from_gray(46)
                    },
                );
                let eye = if layer.visible { "👁" } else { "–" };
                painter.text(
                    Pos2::new(o.x + 6.0, y + RH / 2.0),
                    Align2::LEFT_CENTER,
                    eye,
                    FontId::proportional(12.0),
                    Color32::from_gray(200),
                );
                painter.text(
                    Pos2::new(o.x + 26.0, y + RH / 2.0),
                    Align2::LEFT_CENTER,
                    &layer.name,
                    FontId::proportional(13.0),
                    Color32::WHITE,
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
                            Pos2::new(frame_x(start) + 1.0, y + 3.0),
                            Pos2::new(frame_x(f) - 1.0, y + RH - 4.0),
                        );
                        painter.rect_filled(bar, 3.0, if sel { EXPOSED_SEL } else { EXPOSED });
                        painter.text(
                            bar.left_center() + Vec2::new(3.0, 0.0),
                            Align2::LEFT_CENTER,
                            d,
                            FontId::proportional(10.0),
                            Color32::WHITE,
                        );
                    }
                }
                for f in 1..=scene.length {
                    if peg_has_key(scene, i, f) {
                        let c = Pos2::new(frame_x(f) + CW / 2.0, y + RH - 5.0);
                        painter.circle_filled(c, 3.0, Color32::from_rgb(240, 200, 60));
                    }
                }
            }

            let cx = frame_x(scene.frame) + CW / 2.0;
            painter.line_segment(
                [Pos2::new(cx, o.y), Pos2::new(cx, o.y + size.y)],
                Stroke::new(2.0, CURRENT),
            );

            if let Some(p) = resp.interact_pointer_pos() {
                let rel = p - o;
                if rel.x >= HEAD {
                    scene.frame = (((rel.x - HEAD) / CW) as u32 + 1).clamp(1, scene.length);
                }
                if rel.y >= RULER && (resp.clicked() || resp.drag_started()) {
                    let row = ((rel.y - RULER) / RH) as usize;
                    if row < scene.layers.len() {
                        if rel.x < 22.0 && resp.clicked() {
                            scene.layers[row].visible = !scene.layers[row].visible;
                        }
                        scene.selected = row;
                    }
                }
            }
        });
}

pub fn xsheet(ui: &mut egui::Ui, scene: &mut Scene) {
    transport(ui, scene);
    ui.separator();
    const NUM: f32 = 40.0;
    const CW: f32 = 64.0;
    const RH: f32 = 18.0;
    const HEAD: f32 = 22.0;
    let cols = scene.layers.len();
    let size = Vec2::new(
        NUM + cols as f32 * CW + 4.0,
        HEAD + scene.length as f32 * RH + 4.0,
    );

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
            let o = resp.rect.min;

            for f in 1..=scene.length {
                let y = o.y + HEAD + (f - 1) as f32 * RH;
                let row = Rect::from_min_size(Pos2::new(o.x, y), Vec2::new(size.x, RH));
                let shade = if f == scene.frame {
                    Color32::from_rgb(110, 50, 50)
                } else if f % 2 == 0 {
                    Color32::from_gray(48)
                } else {
                    Color32::from_gray(44)
                };
                painter.rect_filled(row, 0.0, shade);
                painter.text(
                    Pos2::new(o.x + NUM - 6.0, y + RH / 2.0),
                    Align2::RIGHT_CENTER,
                    f.to_string(),
                    FontId::monospace(11.0),
                    Color32::from_gray(170),
                );
            }

            for (i, layer) in scene.layers.iter().enumerate() {
                let x = o.x + NUM + i as f32 * CW;
                let sel = i == scene.selected;
                let head = Rect::from_min_size(Pos2::new(x, o.y), Vec2::new(CW - 1.0, HEAD - 2.0));
                painter.rect_filled(
                    head,
                    2.0,
                    if sel {
                        EXPOSED_SEL
                    } else {
                        Color32::from_gray(70)
                    },
                );
                painter.text(
                    head.center(),
                    Align2::CENTER_CENTER,
                    &layer.name,
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
                painter.line_segment(
                    [Pos2::new(x, o.y), Pos2::new(x, o.y + size.y)],
                    Stroke::new(1.0, Color32::from_gray(30)),
                );

                let Some(col) = scene.xsheet.column(&layer.name) else {
                    continue;
                };
                for f in 1..=scene.length {
                    let Some(d) = col.get(f) else { continue };
                    let y = o.y + HEAD + (f - 1) as f32 * RH;
                    if f > 1 && col.get(f - 1) == Some(d) {
                        // Held cell: a vertical line, like a paper exposure sheet.
                        let cx = x + CW / 2.0;
                        painter.line_segment(
                            [Pos2::new(cx, y), Pos2::new(cx, y + RH)],
                            Stroke::new(1.5, Color32::from_gray(200)),
                        );
                    } else {
                        painter.text(
                            Pos2::new(x + CW / 2.0, y + RH / 2.0),
                            Align2::CENTER_CENTER,
                            d,
                            FontId::monospace(12.0),
                            Color32::WHITE,
                        );
                    }
                }
            }

            if let Some(p) = resp.interact_pointer_pos() {
                let rel = p - o;
                if rel.y >= HEAD {
                    scene.frame = (((rel.y - HEAD) / RH) as u32 + 1).clamp(1, scene.length);
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
