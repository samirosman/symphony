//! Symphony desktop app.
//!
//! Shortcuts (Harmony-style; still need checking against Harmony 27's list):
//! - `,` / `.`  previous / next frame
//! - `` ` ``    maximise the focused view (again to restore)
//! - Cmd/Ctrl+Z undo last stroke

mod scene;
mod views;
mod workspace;

use eframe::egui;
use egui_dock::{DockArea, DockState, Style};
use serde::{Deserialize, Serialize};
use symphony_pen::{DeviceInfo, PenInput, PenSample};

use scene::Scene;
use views::{camera, node, panels, timing};
use workspace::{View, Workspace};

#[derive(Serialize, Deserialize)]
struct Saved {
    workspace: Workspace,
    layouts: Vec<(Workspace, DockState<View>)>,
}

struct SymphonyApp {
    scene: Scene,
    workspace: Workspace,
    layouts: Vec<(Workspace, DockState<View>)>,
    pen: Option<PenInput>,
    camera: camera::CanvasState,
    drawing: camera::CanvasState,
    node_view: node::NodeViewState,
    animate: bool,
    maximised: Option<View>,
    last_time: Option<f64>,
    frame_clock: f64,
}

impl SymphonyApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let pen = match PenInput::attach(cc) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("Pen input: {e}. Falling back to mouse.");
                None
            }
        };
        let saved: Option<Saved> = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY));
        let (workspace, mut layouts) = match saved {
            Some(s) => (s.workspace, s.layouts),
            None => (Workspace::Default, Vec::new()),
        };
        for ws in Workspace::ALL {
            if !layouts.iter().any(|(w, _)| *w == ws) {
                layouts.push((ws, ws.preset()));
            }
        }
        SymphonyApp {
            scene: Scene::demo(),
            workspace,
            layouts,
            pen,
            camera: Default::default(),
            drawing: Default::default(),
            node_view: Default::default(),
            animate: true,
            maximised: None,
            last_time: None,
            frame_clock: 0.0,
        }
    }

    fn layout_mut(&mut self) -> &mut DockState<View> {
        let ws = self.workspace;
        &mut self
            .layouts
            .iter_mut()
            .find(|(w, _)| *w == ws)
            .expect("every workspace has a layout")
            .1
    }

    fn show_view(&mut self, view: View) {
        let layout = self.layout_mut();
        match layout.find_tab(&view) {
            Some(path) => {
                let _ = layout.set_active_tab(path);
            }
            None => layout.push_to_focused_leaf(view),
        }
    }

    fn tick_playback(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let dt = self.last_time.map_or(0.0, |t| now - t);
        self.last_time = Some(now);
        if !self.scene.playing {
            self.frame_clock = 0.0;
            return;
        }
        self.frame_clock += dt;
        let step = 1.0 / self.scene.fps as f64;
        while self.frame_clock >= step {
            self.frame_clock -= step;
            self.scene.frame = if self.scene.frame >= self.scene.length {
                1
            } else {
                self.scene.frame + 1
            };
        }
        ctx.request_repaint();
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (prev, next, max, undo) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Comma),
                i.key_pressed(egui::Key::Period),
                i.key_pressed(egui::Key::Backtick),
                i.modifiers.command && i.key_pressed(egui::Key::Z),
            )
        });
        if prev {
            self.scene.frame = self.scene.frame.saturating_sub(1).max(1);
        }
        if next {
            self.scene.frame = (self.scene.frame + 1).min(self.scene.length);
        }
        if undo {
            self.scene.undo();
        }
        if max {
            self.maximised = match self.maximised {
                Some(_) => None,
                None => self.layout_mut().find_active_focused().map(|(_, v)| *v),
            };
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.button("Undo stroke  (Cmd/Ctrl+Z)").clicked() {
                    self.scene.undo();
                }
            });
            ui.menu_button("Windows", |ui| {
                for v in View::ALL {
                    if ui.button(v.title()).clicked() {
                        self.show_view(v);
                    }
                }
            });
            ui.separator();
            ui.label("Workspace");
            let mut ws = self.workspace;
            egui::ComboBox::from_id_salt("workspace")
                .selected_text(ws.title())
                .show_ui(ui, |ui| {
                    for w in Workspace::ALL {
                        ui.selectable_value(&mut ws, w, w.title());
                    }
                });
            if ws != self.workspace {
                self.workspace = ws;
                self.maximised = None;
            }
            if ui
                .button("Reset")
                .on_hover_text("Put this workspace back to its preset")
                .clicked()
            {
                *self.layout_mut() = self.workspace.preset();
                self.maximised = None;
            }
        });
    }
}

struct Viewer<'a> {
    scene: &'a mut Scene,
    camera: &'a mut camera::CanvasState,
    drawing: &'a mut camera::CanvasState,
    node_view: &'a mut node::NodeViewState,
    animate: &'a mut bool,
    samples: &'a [PenSample],
    device: Option<&'a DeviceInfo>,
}

impl Viewer<'_> {
    fn view(&mut self, ui: &mut egui::Ui, v: View) {
        match v {
            View::Camera => camera::ui(ui, self.scene, self.camera, self.samples, false),
            View::Drawing => camera::ui(ui, self.scene, self.drawing, self.samples, true),
            View::Timeline => timing::timeline(ui, self.scene),
            View::Xsheet => timing::xsheet(ui, self.scene),
            View::Node => node::ui(ui, self.scene, self.node_view),
            View::ToolProperties => panels::tool_properties(ui, self.scene, self.device),
            View::Colour => panels::colour(ui, self.scene),
            View::LayerProperties => panels::layer_properties(ui, self.scene, self.animate),
            View::Function => panels::planned(ui, "Function view (curve editor)", "M6"),
            View::Library => panels::planned(ui, "Library (templates and symbols)", "M6"),
        }
    }
}

impl egui_dock::TabViewer for Viewer<'_> {
    type Tab = View;

    fn id(&mut self, tab: &mut View) -> egui::Id {
        egui::Id::unique(("symphony-view", *tab))
    }

    fn title(&mut self, tab: &mut View) -> egui::WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut View) {
        self.view(ui, *tab);
    }

    fn scroll_bars(&self, tab: &View) -> [bool; 2] {
        // Canvas-like views manage their own space.
        match tab {
            View::Camera | View::Drawing | View::Node | View::Timeline | View::Xsheet => {
                [false, false]
            }
            _ => [false, true],
        }
    }
}

impl eframe::App for SymphonyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let samples = match &self.pen {
            Some(p) => p.drain(),
            None => camera::pointer_samples(&ctx),
        };
        let device = self.pen.as_ref().map(|p| p.device());
        self.tick_playback(&ctx);
        self.shortcuts(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.top_bar(ui));

        let ws = self.workspace;
        let maximised = self.maximised;
        let layout = &mut self
            .layouts
            .iter_mut()
            .find(|(w, _)| *w == ws)
            .expect("layout")
            .1;
        let mut viewer = Viewer {
            scene: &mut self.scene,
            camera: &mut self.camera,
            drawing: &mut self.drawing,
            node_view: &mut self.node_view,
            animate: &mut self.animate,
            samples: &samples,
            device: device.as_ref(),
        };
        egui::CentralPanel::default().show(ui, |ui| match maximised {
            Some(v) => viewer.view(ui, v),
            None => {
                DockArea::new(layout)
                    .style(Style::from_egui(ui.style()))
                    .show_add_buttons(false)
                    .show_inside(ui, &mut viewer);
            }
        });

        let pending = self.pen.as_ref().is_some_and(|p| p.has_pending());
        if pending || self.camera.busy() || self.drawing.busy() {
            ctx.request_repaint();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let saved = Saved {
            workspace: self.workspace,
            layouts: self.layouts.clone(),
        };
        eframe::set_value(storage, eframe::APP_KEY, &saved);
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Symphony")
            .with_inner_size([1400.0, 860.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Symphony",
        options,
        Box::new(|cc| Ok(Box::new(SymphonyApp::new(cc)))),
    )
}
