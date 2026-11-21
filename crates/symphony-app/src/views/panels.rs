//! Side panels: Tool Properties, Colour, Layer Properties, Preferences, and
//! placeholders for views that aren't built yet.

use eframe::egui;
use symphony_core::{graph::PegParams, id::Id};
use symphony_pen::DeviceInfo;
use symphony_ui::icons::{self, Icon};
use symphony_ui::{theme, widgets};

use crate::keymap::{Action, Profile};
use crate::session::{BrushSettings, PaintMode, Session, Tool};

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
            Tool::Select | Tool::Transform => {
                let select = s.tool == Tool::Select;
                section(ui, if select { "Select" } else { "Transform" });
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut s.marquee_mode, false, "Lasso");
                    ui.selectable_value(&mut s.marquee_mode, true, "Marquee");
                });
                hint(
                    ui,
                    "Hold Option/Alt to use the other mode for one selection.",
                );
                widgets::checkbox(ui, &mut s.easy_drag, "Easy drag");
                hint(
                    ui,
                    "Drag the selection from anywhere inside its box, not only on the artwork.",
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Flip horizontal").clicked() {
                        if select {
                            s.flip_selection(true);
                        } else {
                            let l = s.selected;
                            s.adjust_peg(l, |v| v.scale_x = -v.scale_x);
                            s.end_gesture();
                        }
                    }
                    if ui.button("Flip vertical").clicked() {
                        if select {
                            s.flip_selection(false);
                        } else {
                            let l = s.selected;
                            s.adjust_peg(l, |v| v.scale_y = -v.scale_y);
                            s.end_gesture();
                        }
                    }
                });
                if select {
                    let n = s.selection.strokes.len();
                    hint(
                        ui,
                        &format!("{n} stroke{} selected", if n == 1 { "" } else { "s" }),
                    );
                    if ui.add_enabled(n > 0, egui::Button::new("Delete")).clicked() {
                        s.delete_selection();
                    }
                } else {
                    hint(ui, "Click artwork to pick its layer. Drag to move, corner handles to scale, just outside a corner to rotate. Animate mode sets keys.");
                }
            }
            Tool::Paint => {
                section(ui, "Paint");
                for mode in [
                    PaintMode::Paint,
                    PaintMode::PaintUnpainted,
                    PaintMode::Repaint,
                    PaintMode::Unpaint,
                ] {
                    widgets::radio_value(ui, &mut s.paint_mode, mode, mode.title());
                }
                hint(ui, match s.paint_mode {
                    PaintMode::Paint => "Fills a closed area, or recolours it if it's already painted. Click a line to recolour the line.",
                    PaintMode::PaintUnpainted => "Fills only empty areas and leaves painted ones alone.",
                    PaintMode::Repaint => "Recolours only areas that are already painted.",
                    PaintMode::Unpaint => "Removes the colour from an area.",
                });
                ui.add_space(6.0);
                widgets::checkbox(ui, &mut s.respect_protected, "Respect protected colours");
                hint(
                    ui,
                    "Leaves colours marked as protected in the Colour view alone (Shift+S).",
                );
                widgets::checkbox(ui, &mut s.show_strokes, "Show strokes");
                hint(ui, "Shows the invisible lines that close areas (K).");
            }
            Tool::CloseGap => {
                section(ui, "Close Gap");
                hint(ui, "Draw a rough line near a gap. Symphony joins the two nearest line ends with an invisible stroke, so the area can be painted.");
                widgets::checkbox(ui, &mut s.show_strokes, "Show strokes");
            }
            Tool::Dropper => {
                section(ui, "Dropper");
                hint(ui, "Click a line or painted area to pick up its colour.");
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

/// A colour chip edits the paint; its name selects the swatch for drawing.
pub fn colour(ui: &mut egui::Ui, s: &mut Session) {
    padded(ui, |ui| {
        let state = ui.scope_id().with(("colour-edit", &s.key));
        let rename = state.with("rename");
        let removal = state.with("remove");
        ui.horizontal(|ui| {
            if icons::button(ui, Icon::Plus, false, true, "Add colour").clicked() {
                s.add_swatch();
                ui.data_mut(|d| d.insert_temp(rename, (s.swatch, String::from("New colour"))));
            }
            let options = &s.doc.palette.swatches;
            if icons::button(ui, Icon::Minus, false, options.len() > 1, "Remove colour").clicked() {
                if let Some(other) = options.iter().find(|sw| sw.id != s.swatch) {
                    ui.data_mut(|d| d.insert_temp(removal, (s.swatch, other.id)));
                }
            }
            ui.label(&s.doc.palette.name);
        });
        let swatches = s.doc.palette.swatches.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, sw) in swatches.iter().enumerate() {
                ui.push_id(sw.id.0, |ui| {
                    ui.horizontal(|ui| {
                        // Harmony marks a protected colour with a red bar
                        // beside it. The space is kept so rows line up.
                        let (bar, _) = ui.allocate_exact_size(
                            egui::vec2(3.0, ui.spacing().interact_size.y),
                            egui::Sense::hover(),
                        );
                        if sw.protected {
                            ui.painter().rect_filled(bar, 1.0, theme::STATE_DANGER);
                            ui.interact(bar, ui.scope_id().with("protected"), egui::Sense::hover())
                                .on_hover_text("Protected colour");
                        }
                        let mut rgb = sw.rgb;
                        let chip = ui
                            .color_edit_button_srgb(&mut rgb)
                            .on_hover_text("Edit colour");
                        if chip.clicked() || chip.changed() {
                            s.swatch = sw.id;
                        }
                        if chip.changed() {
                            s.set_swatch(sw.id, rgb);
                        }
                        let draft = ui.data(|d| d.get_temp::<(Id, String)>(rename));
                        if let Some((id, mut text)) = draft.filter(|(id, _)| *id == sw.id) {
                            let edit = ui.text_edit_singleline(&mut text);
                            if !edit.has_focus() && !edit.lost_focus() {
                                edit.request_focus();
                            }
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                ui.data_mut(|d| d.remove::<(Id, String)>(rename));
                            } else if edit.lost_focus() {
                                s.rename_swatch(id, &text);
                                ui.data_mut(|d| d.remove::<(Id, String)>(rename));
                            } else {
                                ui.data_mut(|d| d.insert_temp(rename, (id, text)));
                            }
                        } else {
                            let row = ui.selectable_label(s.swatch == sw.id, &sw.name);
                            if row.clicked() {
                                s.swatch = sw.id;
                            }
                            if row.double_clicked() {
                                ui.data_mut(|d| d.insert_temp(rename, (sw.id, sw.name.clone())));
                            }
                            row.context_menu(|ui| {
                                if ui.button("Rename").clicked() {
                                    ui.data_mut(|d| {
                                        d.insert_temp(rename, (sw.id, sw.name.clone()))
                                    });
                                    ui.close();
                                }
                                let protect = if sw.protected {
                                    "Unprotect Colour"
                                } else {
                                    "Protect Colour"
                                };
                                if ui.button(protect).clicked() {
                                    s.toggle_protect_swatch(sw.id);
                                    ui.close();
                                }
                                for (label, delta, enabled) in [
                                    ("Move up", -1, index > 0),
                                    ("Move down", 1, index + 1 < swatches.len()),
                                ] {
                                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                                        s.move_swatch(sw.id, delta);
                                        ui.close();
                                    }
                                }
                            });
                        }
                    })
                });
            }
        });
        let pending = ui.data(|d| d.get_temp::<(Id, Id)>(removal));
        if let Some((id, mut next)) = pending {
            if let Some(sw_name) = s.doc.palette.get(id).map(|sw| sw.name.clone()) {
                egui::Modal::new(removal).show(ui.ctx(), |ui| {
                    ui.heading(format!("Remove {}?", sw_name));
                    ui.label("Use this colour for its strokes and fills:");
                    egui::ComboBox::from_id_salt("replacement")
                        .selected_text(s.doc.palette.get(next).map_or("", |sw| sw.name.as_str()))
                        .show_ui(ui, |ui| {
                            for item in swatches.iter().filter(|item| item.id != id) {
                                ui.selectable_value(&mut next, item.id, &item.name);
                            }
                        });
                    ui.data_mut(|d| d.insert_temp(removal, (id, next)));
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            ui.data_mut(|d| d.remove::<(Id, Id)>(removal));
                        }
                        if ui.button("Remove colour").clicked() {
                            s.delete_swatch(id, next);
                            ui.data_mut(|d| d.remove::<(Id, Id)>(removal));
                        }
                    });
                });
            }
        }
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
        if widgets::checkbox(ui, &mut visible, "Visible").changed() {
            s.toggle_visible(i);
        }
        widgets::checkbox(ui, &mut s.animate, "Animate mode (edits set keyframes)");
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
pub fn preferences(
    ui: &mut egui::Ui,
    profile: &mut Profile,
    use_no_ai: &mut bool,
    brush_size_cursor: &mut bool,
) {
    ui.heading("Tools");
    widgets::checkbox(ui, brush_size_cursor, "Brush size cursor");
    hint(
        ui,
        "Shows the brush size as a circle around the cursor, as in Harmony.",
    );
    ui.separator();
    ui.heading("AI");
    widgets::checkbox(ui, use_no_ai, "Use no AI");
    hint(ui, "Symphony has no generative AI. It has no assisted tools yet either, so nothing in it uses AI.");
    ui.hyperlink_to(
        "How Symphony uses AI",
        "https://github.com/tdemers218/symphony/blob/main/docs/AI-POLICY.md",
    );
    ui.separator();
    ui.heading("Workflow");
    for p in [Profile::Harmony, Profile::Symphony] {
        widgets::radio_value(ui, profile, p, p.title());
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
        "Harmony shortcuts follow Harmony 27's published defaults.",
    );
}

/// Help > Icon sheet: every icon side by side for review (D7.7).
pub fn icon_sheet(ui: &mut egui::Ui) {
    egui::Grid::new("icon-proof")
        .spacing(egui::vec2(6.0, 4.0))
        .show(ui, |ui| {
            for group in Icon::ALL.chunks(9) {
                for &icon in group {
                    ui.vertical_centered(|ui| {
                        ui.set_width(74.0);
                        icons::button(ui, icon, false, true, &format!("{icon:?}"));
                        icons::tool_button(ui, icon, true, true, &format!("{icon:?}"));
                        let label = match icon {
                            Icon::FirstFrame => "First frame",
                            Icon::PrevFrame => "Previous frame",
                            Icon::NextFrame => "Next frame",
                            Icon::LastFrame => "Last frame",
                            Icon::EyeOff => "Hidden",
                            Icon::Eye => "Visible",
                            Icon::Onion => "Onion skin",
                            Icon::RotateLeft => "Rotate left",
                            Icon::RotateRight => "Rotate right",
                            Icon::ZoomOut => "Zoom out",
                            Icon::CloseGap => "Close gap",
                            _ => &format!("{icon:?}"),
                        };
                        ui.small(label);
                    });
                }
                ui.end_row();
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
