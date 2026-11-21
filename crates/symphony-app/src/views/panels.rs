//! Side panels: Tool Properties, Colour, Layer Properties, and the views
//! that exist in the layout but aren't built yet.

use eframe::egui;
use symphony_pen::DeviceInfo;

use crate::scene::{set_value, Scene};

pub fn tool_properties(ui: &mut egui::Ui, scene: &mut Scene, device: Option<&DeviceInfo>) {
    ui.heading("Brush");
    ui.add(egui::Slider::new(&mut scene.brush.max_width, 0.5..=60.0).text("Maximum size"));
    ui.add(egui::Slider::new(&mut scene.brush.min_width, 0.0..=20.0).text("Minimum size"));
    if scene.brush.min_width > scene.brush.max_width {
        scene.brush.min_width = scene.brush.max_width;
    }
    ui.label("Pressure sets the size between minimum and maximum.");
    ui.separator();
    ui.heading("Tablet");
    match device {
        Some(d) if d.in_proximity => {
            ui.label(format!("Pen in range · vendor {:#06x}", d.vendor_id));
        }
        Some(_) => {
            ui.label("Pen input ready. Bring the pen near the tablet.");
        }
        None => {
            ui.label("No native pen backend on this OS yet. Mouse only.");
        }
    }
}

pub fn colour(ui: &mut egui::Ui, scene: &mut Scene) {
    ui.label("Editing a swatch recolours every stroke that uses it.");
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
    ui.label(format!("Peg at frame {frame}"));
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
            .add(egui::DragValue::new(&mut rot).speed(0.5).suffix("°"))
            .changed()
        {
            set_value(&mut p.rotation, frame, rot, *animate);
        }
        ui.end_row();
    });
}

pub fn planned(ui: &mut egui::Ui, what: &str, milestone: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(20.0);
        ui.label(egui::RichText::new(what).strong());
        ui.label(format!("Coming in {milestone}. See docs/ROADMAP.md."));
    });
}
