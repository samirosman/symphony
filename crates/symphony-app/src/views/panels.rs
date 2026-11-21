//! Side panels: Tool Properties, Colour, Layer Properties, Preferences, and
//! placeholders for views that aren't built yet.

use eframe::egui;
use symphony_pen::DeviceInfo;

use crate::keymap::{Action, Profile};
use crate::scene::{set_value, Scene};
use symphony_ui::icons::{self, Icon};
use symphony_ui::theme;

pub fn tool_properties(ui: &mut egui::Ui, scene: &mut Scene, device: Option<&DeviceInfo>) {
    ui.heading("Brush");
    ui.add(egui::Slider::new(&mut scene.brush.max_width, 0.5..=60.0).text("Maximum size"));
    ui.add(egui::Slider::new(&mut scene.brush.min_width, 0.0..=20.0).text("Minimum size"));
    if scene.brush.min_width > scene.brush.max_width {
        scene.brush.min_width = scene.brush.max_width;
    }
    ui.colored_label(
        theme::TEXT_SECONDARY,
        "Pen pressure sets the size between minimum and maximum.",
    );
    ui.separator();
    ui.heading("Tablet");
    let (text, colour) = match device {
        Some(d) if d.in_proximity => ("Pen in range", theme::STATE_OK),
        Some(_) => (
            "Pen input ready. Bring the pen near the tablet.",
            theme::TEXT_SECONDARY,
        ),
        None => (
            "No pen support on this system yet. Using the mouse.",
            theme::TEXT_SECONDARY,
        ),
    };
    ui.colored_label(colour, text);
}

pub fn colour(ui: &mut egui::Ui, scene: &mut Scene) {
    ui.colored_label(
        theme::TEXT_SECONDARY,
        "Editing a swatch recolours every stroke that uses it.",
    );
    ui.add_space(4.0);
    for i in 0..scene.palette.len() {
        ui.horizontal(|ui| {
            let selected = scene.brush.colour == i;
            ui.color_edit_button_srgb(&mut scene.palette[i].rgb);
            if ui
                .selectable_label(selected, scene.palette[i].name)
                .clicked()
            {
                scene.brush.colour = i;
            }
        });
    }
}

pub fn layer_properties(ui: &mut egui::Ui, scene: &mut Scene, animate: &mut bool) {
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

/// Preferences: Workflow (DESIGN §11) and AI (DESIGN §15).
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
        ui.add_space(20.0);
        ui.label(egui::RichText::new(what).strong());
        ui.colored_label(
            theme::TEXT_SECONDARY,
            format!("Coming in {milestone}. See docs/ROADMAP.md."),
        );
    });
}
