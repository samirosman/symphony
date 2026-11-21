//! Side panels: Tool Properties, Colour, Layer Properties, Preferences, and
//! placeholders for views that aren't built yet.

use eframe::egui;
use symphony_core::graph::PegParams;
use symphony_pen::DeviceInfo;
use symphony_ui::icons::{self, Icon};
use symphony_ui::theme;

use crate::keymap::{Action, Profile};
use crate::session::{BrushSettings, Session, Tool};

/// A peg parameter row: label, which value, drag speed, unit suffix.
type PegField = (
    &'static str,
    fn(&mut PegParams) -> &mut symphony_core::anim::Value,
    f32,
    &'static str,
);

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

fn slider(ui: &mut egui::Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>) {
    hint(ui, label);
    ui.spacing_mut().slider_width = (ui.available_width() - 72.0).max(80.0);
    ui.add(egui::Slider::new(v, range));
}

/// A tapered sample stroke that shows what the settings will do.
fn preview(ui: &mut egui::Ui, s: &Session, b: &BrushSettings) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 56.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, theme::RADIUS_CONTROL as f32, theme::CANVAS_STAGE);
    let [r, g, bl] = s.doc.swatch_rgb(s.swatch);
    let ink = egui::Color32::from_rgb(r, g, bl);
    let n = 48;
    let (minw, maxw) = (b.min_width, b.max_width.min(rect.height() - 12.0));
    let pt = |i: usize| {
        let t = i as f32 / n as f32;
        let x = rect.left() + 16.0 + t * (rect.width() - 32.0);
        let y = rect.center().y + (t * std::f32::consts::TAU).sin() * 8.0;
        let pressure =
            symphony_vector::pressure_curve((t * std::f32::consts::PI).sin(), s.pressure_gamma);
        (egui::pos2(x, y), minw + (maxw - minw) * pressure)
    };
    for i in 0..n {
        let ((a, wa), (c, wc)) = (pt(i), pt(i + 1));
        let w = (wa + wc) * 0.5;
        p.line_segment([a, c], egui::Stroke::new(w, ink));
        p.circle_filled(c, w * 0.5, ink);
    }
}

fn brush_controls(ui: &mut egui::Ui, s: &mut Session, pencil: bool) {
    let mut b = if pencil { s.pencil } else { s.brush };
    preview(ui, s, &b);
    ui.add_space(8.0);
    slider(ui, "Maximum size", &mut b.max_width, 0.5..=60.0);
    slider(ui, "Minimum size", &mut b.min_width, 0.0..=60.0);
    b.min_width = b.min_width.min(b.max_width);
    slider(ui, "Smoothing", &mut b.smoothing, 0.0..=0.95);
    hint(ui, "Pen pressure sets the size between minimum and maximum. Smoothing steadies the line as you draw.");
    if pencil {
        s.pencil = b;
    } else {
        s.brush = b;
    }
}

pub fn tool_properties(ui: &mut egui::Ui, s: &mut Session, device: Option<&DeviceInfo>) {
    padded(ui, |ui| {
        match s.tool {
            Tool::Brush => {
                section(ui, "Brush");
                brush_controls(ui, s, false);
            }
            Tool::Pencil => {
                section(ui, "Pencil");
                brush_controls(ui, s, true);
            }
            Tool::Eraser => {
                section(ui, "Eraser");
                hint(ui, "Removes whole strokes it touches. The eraser end of your pen works with any tool.");
            }
            Tool::Hand => {
                section(ui, "Hand");
                hint(
                    ui,
                    "Drag to pan. Hold Space with any tool to pan, Shift+Space to rotate.",
                );
            }
            Tool::Zoom => {
                section(ui, "Zoom");
                hint(ui, "Click to zoom in, Option/Alt-click to zoom out. Pinch or Cmd/Ctrl+scroll also zoom.");
            }
            other => {
                section(ui, &format!("{other:?}"));
                hint(ui, "Not built yet.");
            }
        }
        ui.add_space(12.0);
        section(ui, "Tablet");
        slider(ui, "Pressure response", &mut s.pressure_gamma, 0.3..=3.0);
        hint(ui, "Lower makes light strokes heavier. 1.0 is linear.");
        let text = match device {
            Some(d) if d.in_proximity => "Pen in range",
            Some(_) => "Pen input ready. Bring the pen near the tablet.",
            None => "No pen support on this system yet. Using the mouse.",
        };
        hint(ui, text);
    });
}

/// Swatch grid. Editing a swatch recolours every stroke that uses it (E2.3).
pub fn colour(ui: &mut egui::Ui, s: &mut Session) {
    padded(ui, |ui| {
        let chip = 32.0;
        let gap = 6.0;
        let per_row = (((ui.available_width() + gap) / (chip + gap)).floor() as usize).max(1);
        let swatches: Vec<_> = s
            .doc
            .palette
            .swatches
            .iter()
            .map(|w| (w.id, w.name.clone(), w.rgb))
            .collect();
        for row in swatches.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (id, name, [r, g, b]) in row {
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(chip, chip), egui::Sense::click());
                    let p = ui.painter();
                    p.rect_filled(
                        rect,
                        theme::RADIUS_CONTROL as f32,
                        egui::Color32::from_rgb(*r, *g, *b),
                    );
                    if s.swatch == *id {
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
                    if resp.on_hover_text(name.as_str()).clicked() {
                        s.swatch = *id;
                    }
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
        ui.add_space(8.0);
        let id = s.swatch;
        let mut rgb = s.doc.swatch_rgb(id);
        let name = s
            .doc
            .palette
            .get(id)
            .map(|w| w.name.clone())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                s.set_swatch(id, rgb);
            }
            ui.label(name);
        });
        hint(ui, "Editing a swatch recolours every stroke that uses it.");
    });
}

pub fn layer_properties(ui: &mut egui::Ui, s: &mut Session) {
    padded(ui, |ui| {
        let i = s.selected;
        let frame = s.frame;
        let id = ui
            .scope_id()
            .with(("layer-name", s.doc.layers.get(i).map(|l| l.column)));
        let mut name = ui
            .data_mut(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| s.layer_name(i).to_owned());
        let r = ui.add(egui::TextEdit::singleline(&mut name).font(theme::body()));
        if r.changed() {
            ui.data_mut(|d| d.insert_temp(id, name.clone()));
        }
        if r.lost_focus() {
            s.rename_layer(i, &name);
            ui.data_mut(|d| d.remove::<String>(id));
        }
        let mut visible = s.layer_visible(i);
        if ui.checkbox(&mut visible, "Visible").changed() {
            s.toggle_visible(i);
        }
        ui.checkbox(&mut s.animate, "Animate mode (edits set keyframes)");
        ui.separator();
        hint(ui, &format!("Peg at frame {frame}"));
        let Some(p) = s.doc.peg(i).cloned() else {
            return;
        };
        let fields: [PegField; 3] = [
            ("Position X", |p| &mut p.x, 1.0, ""),
            ("Position Y", |p| &mut p.y, 1.0, ""),
            ("Rotation", |p| &mut p.rotation, 0.5, " deg"),
        ];
        egui::Grid::new("peg").num_columns(2).show(ui, |ui| {
            for (label, pick, speed, suffix) in fields {
                ui.label(label);
                let mut copy = p.clone();
                let mut v = pick(&mut copy).at(frame);
                let r = ui.add(egui::DragValue::new(&mut v).speed(speed).suffix(suffix));
                if r.changed() {
                    s.set_peg_value(i, pick, v);
                }
                if r.drag_stopped() || r.lost_focus() {
                    s.end_gesture();
                }
                ui.end_row();
            }
        });
    });
}

/// Preferences: Workflow (DESIGN §11) and AI (DESIGN §16).
pub fn preferences(ui: &mut egui::Ui, profile: &mut Profile, use_no_ai: &mut bool) {
    ui.heading("AI");
    ui.checkbox(use_no_ai, "Use no AI");
    hint(ui, "Symphony has no generative AI. It has no assisted tools yet either, so nothing in it uses AI.");
    ui.hyperlink_to(
        "How Symphony uses AI",
        "https://github.com/tdemers218/symphony/blob/main/docs/AI-POLICY.md",
    );
    ui.separator();
    ui.heading("Workflow");
    for p in [Profile::Harmony, Profile::Symphony] {
        ui.radio_value(profile, p, p.title());
        hint(ui, p.blurb());
        ui.add_space(4.0);
    }
    ui.separator();
    ui.heading("Shortcuts");
    egui::ScrollArea::vertical()
        .max_height(320.0)
        .show(ui, |ui| {
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
        });
    hint(
        ui,
        "Harmony shortcuts are being checked against Harmony 27.",
    );
}

/// Help > Icon sheet: every icon side by side for review (D7.7).
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
