//! Symphony desktop app. Window anatomy follows docs/DESIGN.md §3: menu bar,
//! workspace bar, Tools toolbar on the left, dock area, status bar.
//! Shortcuts come only from the keymap (ENGINEERING E4.3).

mod keymap;
mod scene;
mod views;
mod workspace;

use eframe::egui;
use egui_dock::{DockArea, DockState};
use serde::{Deserialize, Serialize};
use symphony_pen::{DeviceInfo, PenInput, PenSample};

use keymap::{Action, Profile};
use scene::{Scene, Tool};
use symphony_ui::icons::{self, Icon};
use symphony_ui::theme;
use views::{camera, node, panels, timing};
use workspace::{View, Workspace};

#[derive(Serialize, Deserialize)]
struct Saved {
    workspace: Workspace,
    layouts: Vec<(Workspace, DockState<View>)>,
    #[serde(default)]
    profile: Profile,
}

struct SymphonyApp {
    scene: Scene,
    workspace: Workspace,
    layouts: Vec<(Workspace, DockState<View>)>,
    profile: Profile,
    pen: Option<PenInput>,
    camera: camera::CanvasState,
    drawing: camera::CanvasState,
    node_view: node::NodeViewState,
    animate: bool,
    maximised: Option<View>,
    show_preferences: bool,
    show_icon_sheet: bool,
    last_time: Option<f64>,
    frame_clock: f64,
}

fn tool_icon(t: Tool) -> Icon {
    match t {
        Tool::Select => Icon::Select,
        Tool::Transform => Icon::Transform,
        Tool::Brush => Icon::Brush,
        Tool::Pencil => Icon::Pencil,
        Tool::Eraser => Icon::Eraser,
        Tool::Paint => Icon::Paint,
        Tool::Dropper => Icon::Dropper,
        Tool::Hand => Icon::Hand,
        Tool::Zoom => Icon::Zoom,
    }
}

fn tool_action(t: Tool) -> Action {
    match t {
        Tool::Select => Action::ToolSelect,
        Tool::Transform => Action::ToolTransform,
        Tool::Brush => Action::ToolBrush,
        Tool::Pencil => Action::ToolPencil,
        Tool::Eraser => Action::ToolEraser,
        Tool::Paint => Action::ToolPaint,
        Tool::Dropper => Action::ToolDropper,
        Tool::Hand => Action::ToolHand,
        Tool::Zoom => Action::ToolZoom,
    }
}

impl SymphonyApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
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
        let (workspace, mut layouts, profile) = match saved {
            Some(s) => (s.workspace, s.layouts, s.profile),
            None => (Workspace::Default, Vec::new(), Profile::Harmony),
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
            profile,
            pen,
            camera: Default::default(),
            drawing: Default::default(),
            node_view: Default::default(),
            animate: true,
            maximised: None,
            show_preferences: false,
            show_icon_sheet: false,
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

    fn run(&mut self, action: Action) {
        match action {
            Action::PrevFrame => self.scene.frame = self.scene.frame.saturating_sub(1).max(1),
            Action::NextFrame => self.scene.frame = (self.scene.frame + 1).min(self.scene.length),
            Action::PlayPause => self.scene.playing = !self.scene.playing,
            Action::Undo => self.scene.undo(),
            Action::MaximiseView => {
                self.maximised = match self.maximised {
                    Some(_) => None,
                    None => self.layout_mut().find_active_focused().map(|(_, v)| *v),
                };
            }
            _ => {
                if let Some(t) = Tool::ALL.into_iter().find(|t| tool_action(*t) == action) {
                    if t.planned_for().is_none() {
                        self.scene.tool = t;
                    }
                }
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        for a in Action::ALL {
            if keymap::pressed(ctx, self.profile, a) {
                self.run(a);
            }
        }
    }

    /// Menu item that shows its shortcut (D8.7).
    fn menu_action(&mut self, ui: &mut egui::Ui, action: Action) {
        let btn =
            egui::Button::new(action.name()).shortcut_text(keymap::label(self.profile, action));
        if ui.add(btn).clicked() {
            self.run(action);
            ui.close();
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Quit Symphony").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                self.menu_action(ui, Action::Undo);
                ui.separator();
                if ui.button("Preferences").clicked() {
                    self.show_preferences = true;
                    ui.close();
                }
            });
            ui.menu_button("Play", |ui| {
                self.menu_action(ui, Action::PlayPause);
                self.menu_action(ui, Action::PrevFrame);
                self.menu_action(ui, Action::NextFrame);
            });
            ui.menu_button("Windows", |ui| {
                for v in View::ALL {
                    if ui.button(v.title()).clicked() {
                        self.show_view(v);
                        ui.close();
                    }
                }
                ui.separator();
                self.menu_action(ui, Action::MaximiseView);
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Icon sheet").clicked() {
                    self.show_icon_sheet = true;
                    ui.close();
                }
            });
        });
    }

    fn workspace_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.colored_label(theme::TEXT_SECONDARY, "Workspace");
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
                .button("Reset workspace")
                .on_hover_text("Put this workspace back to its preset layout")
                .clicked()
            {
                *self.layout_mut() = self.workspace.preset();
                self.maximised = None;
            }
        });
    }

    fn tools_strip(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for t in Tool::ALL {
                let action = tool_action(t);
                let mut tip = keymap::tooltip(self.profile, action);
                if let Some(m) = t.planned_for() {
                    tip.push_str(&format!("\nComing in {m}"));
                }
                let enabled = t.planned_for().is_none();
                if icons::button(ui, tool_icon(t), self.scene.tool == t, enabled, &tip).clicked() {
                    self.scene.tool = t;
                }
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, device: Option<&DeviceInfo>) {
        ui.horizontal(|ui| {
            let s = &self.scene;
            ui.colored_label(
                theme::TEXT_SECONDARY,
                format!("Frame {} of {}", s.frame, s.length),
            );
            ui.separator();
            ui.colored_label(
                theme::TEXT_SECONDARY,
                format!("Layer {}", s.column_name(s.selected)),
            );
            ui.separator();
            ui.colored_label(
                theme::TEXT_SECONDARY,
                format!("Tool {}", tool_action(s.tool).name()),
            );
            if self.animate {
                ui.separator();
                // D8.5: a visible mode, in its token colour (amber = time).
                ui.colored_label(theme::ACCENT_TIME, "Animate mode on");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.colored_label(
                    theme::TEXT_SECONDARY,
                    format!("{} profile", self.profile.title()),
                );
                ui.separator();
                let pen = match device {
                    Some(d) if d.in_proximity => "Pen in range",
                    Some(_) => "Pen ready",
                    None => "Mouse only",
                };
                ui.colored_label(theme::TEXT_SECONDARY, pen);
            });
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
    profile: Profile,
}

impl Viewer<'_> {
    fn view(&mut self, ui: &mut egui::Ui, v: View) {
        match v {
            View::Camera => camera::ui(ui, self.scene, self.camera, self.samples, false),
            View::Drawing => camera::ui(ui, self.scene, self.drawing, self.samples, true),
            View::Timeline => timing::timeline(ui, self.scene, self.profile),
            View::Xsheet => timing::xsheet(ui, self.scene, self.profile),
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

        let bar = egui::Frame::new()
            .fill(theme::BG_RAISED)
            .inner_margin(egui::Margin::symmetric(8, 4));
        egui::Panel::top("menu")
            .frame(bar)
            .show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("workspace")
            .frame(bar)
            .show(ui, |ui| self.workspace_bar(ui));
        egui::Panel::bottom("status")
            .frame(bar)
            .show(ui, |ui| self.status_bar(ui, device.as_ref()));
        let strip = egui::Frame::new()
            .fill(theme::BG_RAISED)
            .inner_margin(egui::Margin::symmetric(0, 8));
        egui::Panel::left("tools")
            .frame(strip)
            .exact_size(theme::TOOLS_STRIP)
            .resizable(false)
            .show(ui, |ui| self.tools_strip(ui));

        let mut open = self.show_preferences;
        egui::Window::new("Preferences")
            .open(&mut open)
            .resizable(false)
            .show(&ctx, |ui| panels::preferences(ui, &mut self.profile));
        self.show_preferences = open;
        let mut open = self.show_icon_sheet;
        egui::Window::new("Icon sheet")
            .open(&mut open)
            .show(&ctx, panels::icon_sheet);
        self.show_icon_sheet = open;

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
            profile: self.profile,
        };
        let central = egui::Frame::new().fill(theme::BG_BASE);
        egui::CentralPanel::default()
            .frame(central)
            .show(ui, |ui| match maximised {
                Some(v) => viewer.view(ui, v),
                None => {
                    DockArea::new(layout)
                        .style(theme::dock_style(ui.style()))
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
            profile: self.profile,
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
