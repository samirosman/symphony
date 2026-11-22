//! Swatch editor with a hue slider and two saturation/value layouts.
use eframe::egui::{self, Color32, Pos2, Sense, Stroke};
use egui::ecolor::Hsva;

pub fn swatch(ui: &mut egui::Ui, rgba: &mut [u8; 4]) -> egui::Response {
    picker(ui, rgba, true)
}

pub fn rgb(ui: &mut egui::Ui, rgb: &mut [u8; 3]) -> egui::Response {
    let [r, g, b] = *rgb;
    let mut rgba = [r, g, b, 255];
    let response = picker(ui, &mut rgba, false);
    *rgb = [rgba[0], rgba[1], rgba[2]];
    response
}

fn picker(ui: &mut egui::Ui, rgba: &mut [u8; 4], edit_opacity: bool) -> egui::Response {
    let [r, g, b, a] = *rgba;
    let ink = Color32::from_rgb(r, g, b);
    let mut chip = ui
        .add(
            egui::Button::new("")
                .min_size(egui::vec2(28.0, 18.0))
                .fill(ink),
        )
        .on_hover_text(format!(
            "Edit colour · {}% opacity",
            (u16::from(a) * 100 + 127) / 255
        ));
    egui::Popup::menu(&chip)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let mode_id = egui::Id::unique("swatch-picker-triangle");
            let mut triangle = ui.data(|d| d.get_temp::<bool>(mode_id)).unwrap_or(false);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut triangle, false, "Square");
                ui.selectable_value(&mut triangle, true, "Triangle");
            });
            ui.data_mut(|d| d.insert_temp(mode_id, triangle));
            let hue_id = chip.id.with("hue");
            let mut hsv = Hsva::from_srgba_unmultiplied(*rgba);
            if hsv.s == 0.0 || hsv.v == 0.0 {
                hsv.h = ui.data(|d| d.get_temp::<f32>(hue_id)).unwrap_or(hsv.h);
            }
            let mut hue = hsv.h * 360.0;
            let hue_changed = symphony_ui::widgets::precise_slider(
                ui,
                egui::Slider::new(&mut hue, 0.0..=360.0)
                    .step_by(1.0)
                    .text("Hue")
                    .suffix("°"),
            )
            .changed();
            hsv.h = hue / 360.0;
            ui.data_mut(|d| d.insert_temp(hue_id, hsv.h));
            let picked = saturation_value(ui, &mut hsv, triangle);
            let mut opacity = hsv.a * 100.0;
            let alpha_changed = edit_opacity
                && symphony_ui::widgets::precise_slider(
                    ui,
                    egui::Slider::new(&mut opacity, 0.0..=100.0)
                        .step_by(1.0)
                        .text("Opacity")
                        .suffix("%"),
                )
                .changed();
            if hue_changed || picked || alpha_changed {
                hsv.a = opacity / 100.0;
                let changed = hsv.to_srgba_unmultiplied();
                if changed != *rgba {
                    *rgba = changed;
                    chip.mark_changed();
                }
            }
        });
    chip
}

fn saturation_value(ui: &mut egui::Ui, hsv: &mut Hsva, triangle: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(200.0, 130.0), Sense::click_and_drag());
    let point = |x: f32, y: f32| {
        Pos2::new(
            rect.left() + x * rect.width(),
            rect.top() + y * rect.height(),
        )
    };
    let pure = Color32::from(Hsva::new(hsv.h, 1.0, 1.0, 1.0));
    let mut mesh = egui::Mesh::default();
    if triangle {
        // Barycentric weights: hue and white form the top edge, black the tip.
        mesh.colored_vertex(point(0.0, 0.0), Color32::WHITE);
        mesh.colored_vertex(point(1.0, 0.0), pure);
        mesh.colored_vertex(point(0.5, 1.0), Color32::BLACK);
        mesh.add_triangle(0, 1, 2);
    } else {
        const ROWS: u32 = 24;
        for row in 0..=ROWS {
            let y = row as f32 / ROWS as f32;
            for x in [0.0, 1.0] {
                mesh.colored_vertex(
                    point(x, y),
                    Color32::from(Hsva::new(hsv.h, x, 1.0 - y, 1.0)),
                );
            }
            if row > 0 {
                let i = 2 * row;
                mesh.add_triangle(i - 2, i - 1, i);
                mesh.add_triangle(i, i - 1, i + 1);
            }
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    let mut changed = false;
    if response.clicked() || response.dragged() {
        if let Some(pos) = response.interact_pointer_pos() {
            let y = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            let x = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            hsv.v = 1.0 - y;
            hsv.s = if triangle {
                if hsv.v > 0.0 {
                    ((x - y * 0.5) / hsv.v).clamp(0.0, 1.0)
                } else {
                    hsv.s
                }
            } else {
                x
            };
            changed = true;
        }
    }
    let x = if triangle {
        hsv.s * hsv.v + (1.0 - hsv.v) / 2.0
    } else {
        hsv.s
    };
    let marker = point(x, 1.0 - hsv.v);
    ui.painter()
        .circle_stroke(marker, 5.0, Stroke::new(3.0, Color32::BLACK));
    ui.painter()
        .circle_stroke(marker, 5.0, Stroke::new(1.4, Color32::WHITE));
    changed
}
