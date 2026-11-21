//! Side panels: Tool Properties, Colour, Layer Properties, Preferences, and
//! placeholders for views that aren't built yet.

use eframe::egui;
use symphony_pen::DeviceInfo;

use crate::keymap::{Action, Profile};
use crate::scene::{set_value, Scene};
use symphony_ui::icons::{self, Icon};
use symphony_ui::theme;

/// Card content padding (D6.2).
fn padded<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .inner_margin(egui::Margin::same(theme::CARD_PAD))
        .show(ui, add)
        .inner
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(title)
            .font(theme::body())
            .color(theme::TEXT_PRIMARY),
    );
    ui.add_space(2.0);
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .font(theme::dense())
            .color(theme::TEXT_SECONDARY),
    );
}

/// A tapered sample stroke that shows what the brush settings will do.
fn brush_preview(ui: &mut egui::Ui, scene: &Scene) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 56.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, theme::RADIUS_CONTROL as f32, theme::CANVAS_STAGE);
    let [r, g, b] = scene
        .palette
        .get(scene.brush.colour)
        .map(|s| s.rgb)
        .unwrap_or_default();
    let ink = egui::Color32::from_rgb(r, g, b);
    let n = 48;
    let (minw, maxw) = (
        scene.brush.min_width,
        scene.brush.max_width.min(rect.height() - 12.0),
    );
    let pt = |i: usize| {
        let t = i as f32 / n as f32;
        let x = rect.left() + 16.0 + t * (rect.width() - 32.0);
        let y = rect.center().y + (t * std::f32::consts::TAU).sin() * 8.0;
        let pressure = (t * std::f32::consts::PI).sin();
        (egui::pos2(x, y), minw + (maxw - minw) * pressure)
    };
    for i in 0..n {
        let ((a, wa), (b2, wb)) = (pt(i), pt(i + 1));
        let w = (wa + wb) * 0.5;
        p.line_segment([a, b2], egui::Stroke::new(w, ink));
        p.circle_filled(b2, w * 0.5, ink);
    }
}

pub fn tool_properties(ui: &mut egui::Ui, scene: &mut Scene, device: Option<&DeviceInfo>) {
    padded(ui, |ui| {
        section(ui, "Brush");
        brush_preview(ui, scene);
        ui.add_space(8.0);
        ui.spacing_mut().slider_width = (ui.available_width() - 72.0).max(80.0);
        hint(ui, "Maximum size");
        ui.add(egui::Slider::new(&mut scene.brush.max_width, 0.5..=60.0));
        hint(ui, "Minimum size");
        ui.add(egui::Slider::new(&mut scene.brush.min_width, 0.0..=20.0));
        if scene.brush.min_width > scene.brush.max_width {
            scene.brush.min_width = scene.brush.max_width;
        }
        hint(
            ui,
            "Pen pressure sets the size between minimum and maximum.",
        );
        ui.add_space(12.0);
        section(ui, "Tablet");
        let text = match device {
            Some(d) if d.in_proximity => "Pen in range",
            Some(_) => "Pen input ready. Bring the pen near the tablet.",
            None => "No pen support on this system yet. Using the mouse.",
        };
        hint(ui, text);
    });
}

/// Swatch grid. Editing a swatch recolours every stroke that uses it (E2.3).
pub fn colour(ui: &mut egui::Ui, scene: &mut Scene) {
    padded(ui, |ui| {
        let chip = 32.0;
        let gap = 6.0;
        let per_row = (((ui.available_width() + gap) / (chip + gap)).floor() as usize).max(1);
        for row in scene
            .palette
            .chunks(per_row)
            .enumerate()
            .map(|(r, c)| (r, c.len()))
            .collect::<Vec<_>>()
        {
            let (r, len) = row;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for k in 0..len {
                    let i = r * per_row + k;
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(chip, chip), egui::Sense::click());
                    let [cr, cg, cb] = scene.palette[i].rgb;
                    let p = ui.painter();
                    p.rect_filled(
                        rect,
                        theme::RADIUS_CONTROL as f32,
                        egui::Color32::from_rgb(cr, cg, cb),
                    );
                    if scene.brush.colour == i {
                        p.rect_stroke(
                            rect.expand(2.5),
                            6.0,
                            egui::Stroke::new(2.0, theme::ACCENT_SELECT),
                            egui::StrokeKind::Middle,
                        );
                    } else if resp.hovered() {
                        p.rect_stroke(
                            rect.expand(1.5),
                            5.0,
                            egui::Stroke::new(1.0, theme::LINE_STRONG),
                            egui::StrokeKind::Middle,
                        );
                    }
                    if resp.on_hover_text(scene.palette[i].name).clicked() {
                        scene.brush.colour = i;
                    }
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
        ui.add_space(8.0);
        let i = scene.brush.colour;
        ui.horizontal(|ui| {
            ui.color_edit_button_srgb(&mut scene.palette[i].rgb);
            ui.label(scene.palette[i].name);
        });
        hint(ui, "Editing a swatch recolours every stroke that uses it.");
    });
}

pub fn layer_properties(ui: &mut egui::Ui, scene: &mut Scene, animate: &mut bool) {
    padded(ui, |ui| layer_properties_inner(ui, scene, animate));
}

fn layer_properties_inner(ui: &mut egui::Ui, scene: &mut Scene, animate: &mut bool) {
    let i = scene.selected;
    let frame = scene.frame;
    ui.heading(scene.layers[i].name.clone());
    ui.checkbox(&mut scene.layers[i].visible, "Visible");
    ui.checkbox(animate, "Animate mode (edits set keyframes)");
    ui.separator();
    ui.colored_label(theme::TEXT_SECONDARY, format!("Peg at frame {frame}"));
    let Some(p) = scene.peg_params_mut(i) else {
        return;
    };
    let mut x = p.x.at(frame);
    let mut y = p.y.at(frame);
    let mut rot = p.rotation.at(frame);
    egui::Grid::new("peg").num_columns(2).show(ui, |ui| {
        ui.label("Position X");
        if ui.add(egui::DragValue::new(&mut x).speed(1.0)).changed() {
            set_value(&mut p.x, frame, x, *animate);
        }
        ui.end_row();
        ui.label("Position Y");
        if ui.add(egui::DragValue::new(&mut y).speed(1.0)).changed() {
            set_value(&mut p.y, frame, y, *animate);
        }
        ui.end_row();
        ui.label("Rotation");
        if ui
            .add(egui::DragValue::new(&mut rot).speed(0.5).suffix(" deg"))
            .changed()
        {
            set_value(&mut p.rotation, frame, rot, *animate);
        }
        ui.end_row();
    });
}

/// Preferences: Workflow (DESIGN §11) and AI (DESIGN §16).
pub fn preferences(ui: &mut egui::Ui, profile: &mut Profile, use_no_ai: &mut bool) {
    ui.heading("AI");
    ui.checkbox(use_no_ai, "Use no AI");
    ui.colored_label(
        theme::TEXT_SECONDARY,
        "Symphony has no generative AI. It has no assisted tools yet either, so nothing in it uses AI.",
    );
    ui.hyperlink_to(
        "How Symphony uses AI",
        "https://github.com/tdemers218/symphony/blob/main/docs/AI-POLICY.md",
    );
    ui.separator();
    ui.heading("Workflow");
    for p in [Profile::Harmony, Profile::Symphony] {
        ui.radio_value(profile, p, p.title());
        ui.colored_label(theme::TEXT_SECONDARY, p.blurb());
        ui.add_space(4.0);
    }
    ui.separator();
    ui.heading("Shortcuts");
    egui::Grid::new("shortcuts")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            for a in Action::ALL {
                ui.label(a.name()).on_hover_text(a.id());
                let l = crate::keymap::label(*profile, a);
                ui.colored_label(
                    theme::TEXT_SECONDARY,
                    if l.is_empty() { "None".to_owned() } else { l },
                );
                ui.end_row();
            }
        });
    ui.colored_label(
        theme::TEXT_SECONDARY,
        "Harmony shortcuts are being checked against Harmony 27.",
    );
}

/// Help → Icon sheet: every icon side by side for review (D7.7).
pub fn icon_sheet(ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        for icon in Icon::ALL {
            icons::button(ui, icon, false, true, &format!("{icon:?}"));
        }
    });
    ui.horizontal_wrapped(|ui| {
        for icon in Icon::ALL {
            icons::button(ui, icon, true, true, &format!("{icon:?} (active)"));
        }
    });
}

pub fn planned(ui: &mut egui::Ui, what: &str, milestone: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(32.0);
        ui.label(egui::RichText::new(what).strong());
        ui.colored_label(
            theme::TEXT_SECONDARY,
            format!("Coming in {milestone}. See docs/ROADMAP.md."),
        );
    });
}
