//! Symphony desktop app. Window anatomy follows docs/DESIGN.md §3: one top
//! bar (the title bar on macOS), the Tools toolbar card, the dock of view
//! cards, and the status bar. Shortcuts come only from the keymap
//! (ENGINEERING E4.3). Saving and autosave run off the main thread (E5.2).

mod codec;
mod keymap;
mod session;
mod shot;
mod views;
mod workspace;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use eframe::egui;
use egui_dock::{DockArea, DockState};
use serde::{Deserialize, Serialize};
use symphony_core::doc::Scene;
use symphony_io::Recovery;
use symphony_pen::{DeviceInfo, PenInput, PenSample};
use symphony_ui::feedback;
use symphony_ui::icons::{self, Icon};
use symphony_ui::theme;

use keymap::{Action, Profile};
use session::{Session, Tool};
use views::{camera, export, node, panels, timing};
use workspace::{View, Workspace};

/// Autosave interval, seconds (E7.5).
const AUTOSAVE_EVERY: f64 = 120.0;

#[derive(Serialize, Deserialize)]
struct Saved {
    workspace: Workspace,
    layouts: Vec<(Workspace, DockState<View>)>,
    #[serde(default)]
    profile: Profile,
    /// DESIGN D16.2: on by default.
    #[serde(default = "yes")]
    use_no_ai: bool,
    #[serde(default)]
    pressure_gamma: Option<f32>,
    #[serde(default)]
    brush_size_cursor: bool,
    /// Use Cisco's OpenH264 for movies (Cisco requires the user can turn it off).
    #[serde(default = "yes")]
    use_openh264: bool,
}

fn yes() -> bool {
    true
}

/// Something waiting on "Save changes?".
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    Quit,
    New,
    Open,
}

enum CodecMsg {
    Progress(u64, Option<u64>),
    Done(Result<PathBuf, String>),
}

/// Downloading Cisco's codec in the background (D17.5).
struct CodecDownload {
    rx: Receiver<CodecMsg>,
    done: u64,
    total: Option<u64>,
}

/// A project being read in the background (DESIGN D17.1).
struct Opening {
    path: PathBuf,
    started: f64,
    rx: Receiver<Result<Scene, String>>,
}

/// Confirmations clear after this many seconds (D17.6).
const NOTICE_SECONDS: f64 = 4.0;

/// Background save results: (version saved, path, result).
type SaveResult = (u64, PathBuf, Result<(), String>);

struct SymphonyApp {
    s: Session,
    workspace: Workspace,
    layouts: Vec<(Workspace, DockState<View>)>,
    profile: Profile,
    use_no_ai: bool,
    pen: Option<PenInput>,
    camera: camera::CanvasState,
    drawing: camera::CanvasState,
    node_view: node::NodeViewState,
    timing: timing::TimingState,
    export: export::ExportDialog,
    maximised: Option<View>,
    show_preferences: bool,
    show_icon_sheet: bool,
    last_time: Option<f64>,
    frame_clock: f64,
    shot: Option<shot::Shot>,
    recovery: Option<Recovery>,
    recovered: Vec<(String, Option<PathBuf>, Scene)>,
    pending: Option<Pending>,
    allow_close: bool,
    error: Option<String>,
    save_tx: Sender<SaveResult>,
    save_rx: Receiver<SaveResult>,
    autosave_at: f64,
    autosaved_version: u64,
    autosaving: Arc<AtomicBool>,
    title: String,
    saving_jobs: u32,
    use_openh264: bool,
    codec_download: Option<CodecDownload>,
    show_licences: bool,
    opening: Option<Opening>,
    notice_seen: Option<(String, f64)>,
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

fn recovery_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("Symphony").join("recovery"))
}

impl SymphonyApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        // Dark window frame on every OS (DESIGN D12.3).
        cc.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::SetTheme(egui::SystemTheme::Dark));
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
        let brush_size_cursor = saved.as_ref().is_some_and(|s| s.brush_size_cursor);
        let use_openh264 = saved.as_ref().is_none_or(|s| s.use_openh264);
        let (mut workspace, mut layouts, profile, use_no_ai, gamma) = match saved {
            Some(s) => (
                s.workspace,
                s.layouts,
                s.profile,
                s.use_no_ai,
                s.pressure_gamma,
            ),
            None => (Workspace::Default, Vec::new(), Profile::Harmony, true, None),
        };
        for ws in Workspace::ALL {
            if !layouts.iter().any(|(w, _)| *w == ws) {
                layouts.push((ws, ws.preset()));
            }
        }
        let shot = shot::Shot::from_env();
        let mut s = if shot.is_some() || std::env::var_os("SYMPHONY_DEMO_ART").is_some() {
            Session::demo()
        } else {
            Session::new_scene()
        };
        if let Some(g) = gamma {
            s.pressure_gamma = g;
        }
        s.brush_size_cursor = brush_size_cursor;
        // Screenshots use fresh presets, so design reviews are repeatable.
        if shot.is_some() {
            layouts = Workspace::ALL.iter().map(|w| (*w, w.preset())).collect();
            if let Ok(name) = std::env::var("SYMPHONY_WORKSPACE") {
                if let Some(w) = Workspace::ALL
                    .into_iter()
                    .find(|w| format!("{w:?}") == name)
                {
                    workspace = w;
                }
            }
        }
        let recovery = recovery_dir().map(Recovery::new);
        let recovered = match (&recovery, &shot) {
            (Some(r), None) => r.pending(),
            _ => Vec::new(),
        };
        let (save_tx, save_rx) = channel();
        let version = s.doc.version;
        SymphonyApp {
            s,
            workspace,
            layouts,
            profile,
            use_no_ai,
            pen,
            camera: Default::default(),
            drawing: Default::default(),
            node_view: Default::default(),
            timing: Default::default(),
            export: Default::default(),
            maximised: None,
            show_preferences: std::env::var_os("SYMPHONY_SHOT_PREFS").is_some(),
            show_icon_sheet: false,
            last_time: None,
            frame_clock: 0.0,
            shot,
            recovery,
            recovered,
            pending: None,
            allow_close: false,
            error: None,
            save_tx,
            save_rx,
            autosave_at: 0.0,
            autosaved_version: version,
            autosaving: Arc::new(AtomicBool::new(false)),
            title: String::new(),
            saving_jobs: 0,
            use_openh264,
            codec_download: None,
            show_licences: false,
            opening: None,
            notice_seen: None,
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

    // ---- Files -------------------------------------------------------------

    /// Run `p` now, or ask to save unsaved work first. Losing work can't be
    /// undone, so this is one of the few confirmations (D8.4).
    fn guard(&mut self, ctx: &egui::Context, p: Pending) {
        if self.s.dirty() {
            self.pending = Some(p);
        } else {
            self.proceed(ctx, p);
        }
    }

    fn proceed(&mut self, ctx: &egui::Context, p: Pending) {
        match p {
            Pending::Quit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Pending::New => self.replace_session(Session::new_scene()),
            Pending::Open => self.open_dialog(),
        }
    }

    fn replace_session(&mut self, next: Session) {
        if let Some(r) = &self.recovery {
            r.clear(&self.s.key);
        }
        let (gamma, cursor) = (self.s.pressure_gamma, self.s.brush_size_cursor);
        self.s = next;
        self.s.pressure_gamma = gamma;
        self.s.brush_size_cursor = cursor;
        self.autosaved_version = self.s.doc.version;
        self.camera.reset_view();
        self.drawing.reset_view();
    }

    fn open_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open a Symphony project (.symphony folder)")
            .pick_folder()
        else {
            return;
        };
        // Read in the background; a loading dialog shows meanwhile (D17.1).
        let (tx, rx) = channel();
        let p = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(symphony_io::open(&p).map_err(|e| e.0));
        });
        self.opening = Some(Opening {
            path,
            started: self.last_time.unwrap_or(0.0),
            rx,
        });
    }

    fn poll_open(&mut self) {
        let Some(o) = &self.opening else { return };
        match o.rx.try_recv() {
            Ok(Ok(doc)) => {
                let path = o.path.clone();
                self.opening = None;
                self.replace_session(Session::from_doc(doc, Some(path)));
                self.s.notice = Some(format!("Opened {}", self.s.name()));
            }
            Ok(Err(e)) => {
                self.opening = None;
                self.error = Some(e);
            }
            Err(_) => {}
        }
    }

    fn ask_save_path(&self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Save scene")
            .set_file_name(format!("{}.symphony", self.s.name()))
            .save_file()
            .map(|p| symphony_io::with_extension(&p))
    }

    fn save(&mut self, save_as: bool) {
        let path = match (&self.s.path, save_as) {
            (Some(p), false) => p.clone(),
            _ => match self.ask_save_path() {
                Some(p) => p,
                None => return,
            },
        };
        // Snapshot, then write in the background (E5.3, E6).
        let (doc, version, tx) = (self.s.doc.clone(), self.s.doc.version, self.save_tx.clone());
        std::thread::spawn(move || {
            let r = symphony_io::save(&doc, &path).map_err(|e| e.0);
            let _ = tx.send((version, path, r));
        });
        self.saving_jobs += 1;
    }

    /// Save synchronously. Only used right before quitting or replacing the scene.
    fn save_now(&mut self) -> bool {
        let Some(path) = self.s.path.clone().or_else(|| self.ask_save_path()) else {
            return false;
        };
        match symphony_io::save(&self.s.doc, &path) {
            Ok(()) => {
                self.s.saved_version = self.s.doc.version;
                self.s.path = Some(path);
                if let Some(r) = &self.recovery {
                    r.clear(&self.s.key);
                }
                true
            }
            Err(e) => {
                self.error = Some(e.0);
                false
            }
        }
    }

    fn poll_saves(&mut self) {
        while let Ok((version, path, result)) = self.save_rx.try_recv() {
            self.saving_jobs = self.saving_jobs.saturating_sub(1);
            match result {
                Ok(()) => {
                    self.s.saved_version = version;
                    self.s.path = Some(path);
                    self.s.notice = Some(format!("Saved {}", self.s.name()));
                    if !self.s.dirty() {
                        if let Some(r) = &self.recovery {
                            r.clear(&self.s.key);
                        }
                    }
                }
                Err(e) => self.error = Some(e),
            }
        }
    }

    /// The codec to hand to movie export: downloaded and turned on.
    fn codec(&self) -> Option<PathBuf> {
        self.use_openh264.then(codec::installed).flatten()
    }

    /// Fetch Cisco's OpenH264 in the background, at the user's request.
    fn start_codec_download(&mut self, ctx: &egui::Context) {
        if self.codec_download.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let t2 = tx.clone();
            let c2 = ctx.clone();
            let r = codec::download(&move |d, t| {
                let _ = t2.send(CodecMsg::Progress(d, t));
                c2.request_repaint();
            });
            let _ = tx.send(CodecMsg::Done(r));
            ctx.request_repaint();
        });
        self.use_openh264 = true;
        self.codec_download = Some(CodecDownload {
            rx,
            done: 0,
            total: None,
        });
    }

    fn poll_codec(&mut self) {
        let Some(d) = self.codec_download.as_mut() else {
            return;
        };
        while let Ok(m) = d.rx.try_recv() {
            match m {
                CodecMsg::Progress(done, total) => (d.done, d.total) = (done, total),
                CodecMsg::Done(r) => {
                    match r {
                        Ok(_) => self.s.notice = Some("OpenH264 is ready for movie export".into()),
                        Err(e) => self.error = Some(e),
                    }
                    self.codec_download = None;
                    return;
                }
            }
        }
    }

    /// Preferences > Video: Cisco's codec, with the notice and controls its
    /// licence requires (docs/CODECS.md).
    fn video_preferences(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Video");
        ui.label(egui::RichText::new(codec::NOTICE).color(theme::TEXT_PRIMARY));
        symphony_ui::widgets::checkbox(ui, &mut self.use_openh264, "Use OpenH264 for movie export");
        let dense = |ui: &mut egui::Ui, t: String| {
            ui.label(
                egui::RichText::new(t)
                    .font(theme::dense())
                    .color(theme::TEXT_SECONDARY),
            );
        };
        if let Some(d) = &self.codec_download {
            ui.horizontal(|ui| {
                feedback::spinner(ui, 14.0);
                ui.label("Downloading from Cisco");
            });
            feedback::progress_bar(
                ui,
                d.total.map(|t| d.done as f32 / t.max(1) as f32),
                ui.available_width(),
            );
        } else if let Some(p) = codec::installed() {
            dense(
                ui,
                format!(
                    "Version {} is installed at {}.",
                    codec::VERSION,
                    p.display()
                ),
            );
            if ui.button("Remove OpenH264").clicked() {
                codec::remove();
            }
        } else if codec::file_name().is_some() {
            dense(
                ui,
                "Not downloaded. Movie export downloads it from Cisco when you ask.".into(),
            );
            if ui.button("Download OpenH264 from Cisco").clicked() {
                let ctx = ui.ctx().clone();
                self.start_codec_download(&ctx);
            }
        } else {
            dense(
                ui,
                "Cisco doesn't publish OpenH264 for this kind of computer.".into(),
            );
        }
        dense(ui, "Cisco's licence covers personal and other unpaid use. Commercial work may need its own H.264 licence.".into());
        if ui.link("Show licence").clicked() {
            self.show_licences = true;
        }
    }

    /// Confirmations show briefly, then clear (D17.6).
    fn expire_notice(&mut self, now: f64) {
        match (&self.s.notice, &self.notice_seen) {
            (Some(n), Some((seen, at))) if n == seen => {
                if now - at > NOTICE_SECONDS {
                    self.s.notice = None;
                    self.notice_seen = None;
                }
            }
            (Some(n), _) => self.notice_seen = Some((n.clone(), now)),
            (None, _) => self.notice_seen = None,
        }
    }

    fn autosave(&mut self, now: f64) {
        if now - self.autosave_at < AUTOSAVE_EVERY {
            return;
        }
        self.autosave_at = now;
        let v = self.s.doc.version;
        if !self.s.dirty() || v == self.autosaved_version || self.autosaving.load(Ordering::SeqCst)
        {
            return;
        }
        let Some(dir) = recovery_dir() else { return };
        self.autosaved_version = v;
        self.autosaving.store(true, Ordering::SeqCst);
        let (doc, key, origin, flag) = (
            self.s.doc.clone(),
            self.s.key.clone(),
            self.s.path.clone(),
            self.autosaving.clone(),
        );
        std::thread::spawn(move || {
            if let Err(e) = Recovery::new(dir).write(&key, &doc, origin.as_deref()) {
                eprintln!("Autosave failed: {e}");
            }
            flag.store(false, Ordering::SeqCst);
        });
    }

    // ---- Actions -----------------------------------------------------------

    fn tick_playback(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let dt = self.last_time.map_or(0.0, |t| now - t);
        self.last_time = Some(now);
        if !self.s.playing {
            self.frame_clock = 0.0;
            return;
        }
        self.frame_clock += dt;
        let step = 1.0 / self.s.doc.settings.fps.max(1.0) as f64;
        while self.frame_clock >= step {
            self.frame_clock -= step;
            self.s.frame = if self.s.frame >= self.s.length() {
                1
            } else {
                self.s.frame + 1
            };
        }
        ctx.request_repaint();
    }

    fn run(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::NewScene => self.guard(ctx, Pending::New),
            Action::Open => self.guard(ctx, Pending::Open),
            Action::Save => self.save(false),
            Action::SaveAs => self.save(true),
            Action::ZoomIn
            | Action::ZoomOut
            | Action::RotateCw
            | Action::RotateCcw
            | Action::ResetZoom
            | Action::ResetRotation
            | Action::ResetPan
            | Action::ResetView => {
                self.camera.apply(action);
                self.drawing.apply(action);
            }
            Action::MaximiseView => {
                self.maximised = match self.maximised {
                    Some(_) => None,
                    None => self.layout_mut().find_active_focused().map(|(_, v)| *v),
                };
            }
            other => {
                let s = &mut self.s;
                match other {
                    Action::Undo => s.undo(),
                    Action::SelectAll => {
                        s.tool = Tool::Select;
                        s.select_all();
                    }
                    Action::DeselectAll => s.deselect(),
                    Action::Delete => s.delete_selection(),
                    Action::Cut => s.cut_selection(),
                    Action::Copy => s.copy_selection(),
                    Action::Paste => {
                        s.tool = Tool::Select;
                        s.paste();
                    }
                    Action::FlipHorizontal | Action::FlipVertical => {
                        let h = other == Action::FlipHorizontal;
                        if s.tool == Tool::Transform {
                            let layer = s.selected;
                            s.adjust_peg(layer, |v| {
                                if h {
                                    v.scale_x = -v.scale_x
                                } else {
                                    v.scale_y = -v.scale_y
                                }
                            });
                            s.end_gesture();
                        } else {
                            s.flip_selection(h);
                        }
                    }
                    Action::Redo => s.redo(),
                    Action::PrevFrame => s.frame = s.frame.saturating_sub(1).max(1),
                    Action::NextFrame => s.frame = (s.frame + 1).min(s.length()),
                    Action::FirstFrame => s.frame = 1,
                    Action::LastFrame => s.frame = s.length(),
                    Action::PlayPause => s.playing = !s.playing,
                    Action::IncreaseExposure => s.change_exposure(1),
                    Action::DecreaseExposure => s.change_exposure(-1),
                    Action::CreateEmptyDrawing => s.new_drawing(),
                    Action::AddLayer => s.add_layer(),
                    Action::ToggleOnionSkin => s.onion.on = !s.onion.on,
                    Action::OnionMorePrevious => s.onion_step(true, 1),
                    Action::OnionLessPrevious => s.onion_step(true, -1),
                    Action::OnionMoreNext => s.onion_step(false, 1),
                    Action::OnionLessNext => s.onion_step(false, -1),
                    Action::OnionPrevious1 => s.onion_set(true, 1),
                    Action::OnionPrevious2 => s.onion_set(true, 2),
                    Action::OnionPrevious3 => s.onion_set(true, 3),
                    Action::OnionNoPrevious => s.onion_set(true, 0),
                    Action::OnionNext1 => s.onion_set(false, 1),
                    Action::OnionNext2 => s.onion_set(false, 2),
                    Action::OnionNext3 => s.onion_set(false, 3),
                    Action::OnionNoNext => s.onion_set(false, 0),
                    _ => {
                        if let Some(t) = Tool::ALL.into_iter().find(|t| tool_action(*t) == other) {
                            if t.planned_for().is_none() {
                                s.tool = t;
                            }
                        }
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
                self.run(ctx, a);
            }
        }
    }

    // ---- Chrome ------------------------------------------------------------

    /// Menu item that shows its shortcut (D8.7).
    fn menu_action(&mut self, ui: &mut egui::Ui, action: Action) {
        self.menu_item(ui, action, action.name().to_owned());
    }

    fn menu_item(&mut self, ui: &mut egui::Ui, action: Action, label: String) {
        let btn = egui::Button::new(label).shortcut_text(keymap::label(self.profile, action));
        if ui.add(btn).clicked() {
            let ctx = ui.ctx().clone();
            ui.close();
            self.run(&ctx, action);
        }
    }

    fn menus(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            self.menu_item(ui, Action::NewScene, "New scene".into());
            self.menu_item(ui, Action::Open, "Open...".into());
            ui.separator();
            self.menu_action(ui, Action::Save);
            self.menu_item(ui, Action::SaveAs, "Save as...".into());
            ui.separator();
            ui.menu_button("Export", |ui| {
                if ui.button("Animated GIF...").clicked() {
                    self.export
                        .open(symphony_render::Format::AnimatedGif, &self.s);
                    ui.close();
                }
                if ui.button("Image Sequence (PNG)...").clicked() {
                    self.export
                        .open(symphony_render::Format::PngSequence, &self.s);
                    ui.close();
                }
                if ui.button("Movie (H.264 MP4)...").clicked() {
                    self.export.open(symphony_render::Format::Mp4Movie, &self.s);
                    ui.close();
                }
            });
            ui.separator();
            if ui.button("Quit Symphony").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        ui.menu_button("Edit", |ui| {
            let undo = self
                .s
                .history
                .undo_name()
                .map_or("Undo".into(), |n| format!("Undo {n}"));
            let redo = self
                .s
                .history
                .redo_name()
                .map_or("Redo".into(), |n| format!("Redo {n}"));
            self.menu_item(ui, Action::Undo, undo);
            self.menu_item(ui, Action::Redo, redo);
            ui.separator();
            self.menu_action(ui, Action::Cut);
            self.menu_action(ui, Action::Copy);
            self.menu_action(ui, Action::Paste);
            self.menu_action(ui, Action::Delete);
            ui.separator();
            self.menu_action(ui, Action::SelectAll);
            self.menu_action(ui, Action::DeselectAll);
            ui.separator();
            self.menu_action(ui, Action::FlipHorizontal);
            self.menu_action(ui, Action::FlipVertical);
            ui.separator();
            if ui.button("Preferences").clicked() {
                self.show_preferences = true;
                ui.close();
            }
        });
        ui.menu_button("Timing", |ui| {
            self.menu_action(ui, Action::IncreaseExposure);
            self.menu_action(ui, Action::DecreaseExposure);
            ui.separator();
            self.menu_action(ui, Action::CreateEmptyDrawing);
            self.menu_action(ui, Action::AddLayer);
        });
        ui.menu_button("Play", |ui| {
            self.menu_action(ui, Action::PlayPause);
            self.menu_action(ui, Action::PrevFrame);
            self.menu_action(ui, Action::NextFrame);
            self.menu_action(ui, Action::FirstFrame);
            self.menu_action(ui, Action::LastFrame);
        });
        ui.menu_button("View", |ui| {
            self.menu_action(ui, Action::ToggleOnionSkin);
            ui.separator();
            self.menu_action(ui, Action::ZoomIn);
            self.menu_action(ui, Action::ZoomOut);
            self.menu_action(ui, Action::RotateCw);
            self.menu_action(ui, Action::RotateCcw);
            ui.separator();
            self.menu_action(ui, Action::ResetZoom);
            self.menu_action(ui, Action::ResetRotation);
            self.menu_action(ui, Action::ResetPan);
            self.menu_action(ui, Action::ResetView);
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
            if ui.button("Licences").clicked() {
                self.show_licences = true;
                ui.close();
            }
            if ui.button("Icon sheet").clicked() {
                self.show_icon_sheet = true;
                ui.close();
            }
        });
    }

    /// D3.1: menus left, scene name centred, workspace right. On macOS this
    /// bar is the title bar, so its empty space drags the window.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let drag = ui.interact(
            rect,
            egui::Id::unique("symphony-top-bar-drag"),
            egui::Sense::click_and_drag(),
        );
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
        }
        let mut title = self.s.name();
        if self.s.dirty() {
            title.push_str("  (edited)");
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            &title,
            theme::body(),
            theme::TEXT_SECONDARY,
        );
        ui.horizontal_centered(|ui| {
            // Room for the macOS window buttons (D12.2).
            if cfg!(target_os = "macos") {
                ui.add_space(72.0);
            }
            egui::MenuBar::new().ui(ui, |ui| self.menus(ui));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut ws = self.workspace;
                egui::ComboBox::from_id_salt("workspace")
                    .width(170.0)
                    .selected_text(ws.title())
                    .show_ui(ui, |ui| {
                        for w in Workspace::ALL {
                            ui.selectable_value(&mut ws, w, w.title());
                        }
                        ui.separator();
                        if ui.button("Reset this workspace").clicked() {
                            *self.layout_mut() = self.workspace.preset();
                            self.maximised = None;
                        }
                    });
                if ws != self.workspace {
                    self.workspace = ws;
                    self.maximised = None;
                }
                ui.colored_label(theme::TEXT_SECONDARY, "Workspace");
            });
        });
    }

    /// The Tools toolbar card, in groups (D6.2).
    fn tools_strip(&mut self, ui: &mut egui::Ui) {
        const GROUPS: [&[Tool]; 4] = [
            &[Tool::Select, Tool::Transform],
            &[Tool::Brush, Tool::Pencil, Tool::Eraser],
            &[Tool::Paint, Tool::Dropper],
            &[Tool::Hand, Tool::Zoom],
        ];
        egui::Frame::new()
            .fill(theme::BG_PANEL)
            .corner_radius(theme::RADIUS_CARD)
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing.y = 2.0;
                for (g, group) in GROUPS.iter().enumerate() {
                    if g > 0 {
                        ui.add_space(theme::TOOL_GROUP_GAP - 2.0);
                    }
                    for &t in group.iter() {
                        let mut tip = keymap::tooltip(self.profile, tool_action(t));
                        if let Some(m) = t.planned_for() {
                            tip.push_str(&format!("\nComing in {m}"));
                        }
                        let enabled = t.planned_for().is_none();
                        if icons::tool_button(ui, tool_icon(t), self.s.tool == t, enabled, &tip)
                            .clicked()
                        {
                            self.s.tool = t;
                        }
                    }
                }
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, device: Option<&DeviceInfo>) {
        let dense = |ui: &mut egui::Ui, text: String, colour: egui::Color32| {
            ui.label(egui::RichText::new(text).font(theme::dense()).color(colour));
        };
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let s = &self.s;
            dense(
                ui,
                format!("Frame {} of {}", s.frame, s.length()),
                theme::TEXT_SECONDARY,
            );
            let drawing = s
                .drawing_name(s.selected, s.frame)
                .map_or("empty cell".into(), |d| format!("drawing {d}"));
            dense(
                ui,
                format!("{}, {drawing}", s.layer_name(s.selected)),
                theme::TEXT_SECONDARY,
            );
            dense(
                ui,
                format!("Tool {}", tool_action(s.tool).name()),
                theme::TEXT_SECONDARY,
            );
            if s.animate {
                // D8.5: a visible mode, in its token colour (amber = time).
                dense(ui, "Animate mode on".into(), theme::ACCENT_TIME);
            }
            if s.onion.on {
                dense(ui, "Onion skin on".into(), theme::TEXT_PRIMARY);
            }
            if let Some(n) = &s.notice {
                dense(ui, n.clone(), theme::TEXT_PRIMARY);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                dense(
                    ui,
                    format!("{} profile", self.profile.title()),
                    theme::TEXT_SECONDARY,
                );
                let (pen, dot) = match device {
                    Some(d) if d.in_proximity => ("Pen in range", theme::STATE_OK),
                    Some(_) => ("Pen ready", theme::TEXT_SECONDARY),
                    None => ("Mouse only", theme::TEXT_DISABLED),
                };
                dense(ui, pen.into(), theme::TEXT_SECONDARY);
                let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 3.5, dot);

                // Activity area (D17.5): background work stays visible.
                let mut jobs: Vec<(String, Option<f32>)> = Vec::new();
                if let Some((done, total)) = self.export.progress() {
                    jobs.push((
                        format!("Exporting frame {done} of {total}"),
                        Some(done as f32 / total.max(1) as f32),
                    ));
                }
                if let Some(d) = &self.codec_download {
                    jobs.push((
                        "Downloading OpenH264".into(),
                        d.total.map(|t| d.done as f32 / t.max(1) as f32),
                    ));
                }
                if self.saving_jobs > 0 {
                    jobs.push(("Saving".into(), None));
                }
                if self.autosaving.load(Ordering::SeqCst) {
                    jobs.push(("Autosaving".into(), None));
                }
                for (label, frac) in jobs {
                    ui.add_space(8.0);
                    if let Some(f) = frac {
                        feedback::progress_bar(ui, Some(f), 64.0);
                    }
                    dense(ui, label, theme::TEXT_PRIMARY);
                    feedback::spinner(ui, 12.0);
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(o) = &self.opening {
            let name = symphony_io::project_name(&o.path);
            let elapsed = self.last_time.unwrap_or(0.0) - o.started;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique("symphony-opening")).show(ctx, |ui| {
                ui.set_width(320.0);
                ui.horizontal(|ui| {
                    feedback::spinner(ui, 16.0);
                    ui.label(format!("Opening {name}"));
                });
                ui.add_space(8.0);
                feedback::progress_bar(ui, None, ui.available_width());
                if let Some(e) = feedback::elapsed_label(elapsed) {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(e)
                                .font(theme::dense())
                                .color(theme::TEXT_SECONDARY),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            cancel = ui.button("Cancel").clicked();
                        });
                    });
                }
            });
            if cancel {
                // The read finishes in the background and is ignored.
                self.opening = None;
            }
        }

        if let Some(p) = self.pending {
            let name = self.s.name();
            let mut choice = None;
            egui::Modal::new(egui::Id::unique("symphony-unsaved")).show(ctx, |ui| {
                ui.heading(format!("Save changes to {name}?"));
                ui.label("If you don't save, the changes since your last save are lost.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("Don't save").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(2);
                    }
                });
            });
            match choice {
                Some(0) => {
                    self.pending = None;
                    if self.save_now() {
                        self.proceed(ctx, p);
                    }
                }
                Some(1) => {
                    self.pending = None;
                    if let Some(r) = &self.recovery {
                        r.clear(&self.s.key);
                    }
                    self.s.saved_version = self.s.doc.version;
                    self.proceed(ctx, p);
                }
                Some(_) => self.pending = None,
                None => {}
            }
        }

        if !self.recovered.is_empty() {
            let n = self.recovered.len();
            let mut choice = None;
            egui::Modal::new(egui::Id::unique("symphony-recover")).show(ctx, |ui| {
                ui.heading("Recover unsaved work?");
                let what = if n == 1 { "a scene".to_owned() } else { format!("{n} scenes") };
                ui.label(format!("Symphony didn't close properly last time. It autosaved {what} you hadn't saved."));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Recover").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Discard").clicked() {
                        choice = Some(false);
                    }
                });
            });
            if let Some(recover) = choice {
                if recover {
                    let (key, origin, doc) = self.recovered.remove(0);
                    let mut next = Session::from_doc(doc, origin);
                    next.key = key;
                    // Recovered work isn't saved yet, so it shows as edited.
                    next.saved_version = next.doc.version.wrapping_sub(1);
                    self.s = next;
                    self.autosaved_version = self.s.doc.version;
                }
                if let Some(r) = &self.recovery {
                    for (k, _, _) in self.recovered.drain(..) {
                        r.clear(&k);
                    }
                }
            }
        }

        if let Some(e) = self.error.clone() {
            egui::Modal::new(egui::Id::unique("symphony-error")).show(ctx, |ui| {
                ui.heading("That didn't work");
                ui.label(e);
                ui.add_space(8.0);
                if ui.button("OK").clicked() {
                    self.error = None;
                }
            });
        }
    }
}

struct Viewer<'a> {
    s: &'a mut Session,
    camera: &'a mut camera::CanvasState,
    drawing: &'a mut camera::CanvasState,
    node_view: &'a mut node::NodeViewState,
    timing: &'a mut timing::TimingState,
    samples: &'a [PenSample],
    device: Option<&'a DeviceInfo>,
    profile: Profile,
}

impl Viewer<'_> {
    fn view(&mut self, ui: &mut egui::Ui, v: View) {
        match v {
            View::Camera => camera::ui(ui, self.s, self.camera, self.samples, false, self.profile),
            View::Drawing => camera::ui(ui, self.s, self.drawing, self.samples, true, self.profile),
            View::Timeline => timing::timeline(ui, self.s, self.timing, self.profile),
            View::Xsheet => timing::xsheet(ui, self.s, self.timing, self.profile),
            View::Node => node::ui(ui, self.s, self.node_view),
            View::ToolProperties => panels::tool_properties(ui, self.s, self.device),
            View::Colour => panels::colour(ui, self.s),
            View::LayerProperties => panels::layer_properties(ui, self.s),
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
        if let Some(s) = self.shot.as_mut() {
            s.tick(&ctx);
        }
        // Design review only: show the shared loading screen (DESIGN D17.7).
        if self.shot.is_some() && std::env::var_os("SYMPHONY_SHOT_LOADING").is_some() {
            feedback::loading_screen(
                ui,
                "Building the latest Symphony",
                Some("Compiling symphony-core (148 of about 246)"),
                Some(0.6),
                12.4,
            );
            return;
        }

        // Closing with unsaved work asks first.
        if ctx.input(|i| i.viewport().close_requested()) && self.s.dirty() && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        }

        let samples = match &self.pen {
            Some(p) => p.drain(),
            None => camera::pointer_samples(&ctx),
        };
        let device = self.pen.as_ref().map(|p| p.device());
        let now = ctx.input(|i| i.time);
        self.poll_saves();
        self.poll_open();
        self.poll_codec();
        self.expire_notice(now);
        self.autosave(now);
        self.tick_playback(&ctx);
        if self.pending.is_none() && self.recovered.is_empty() && self.error.is_none() {
            self.shortcuts(&ctx);
        }

        let title = if self.s.dirty() {
            format!("{} (edited) - Symphony", self.s.name())
        } else {
            format!("{} - Symphony", self.s.name())
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }

        let base = egui::Frame::new().fill(theme::BG_BASE);
        egui::Panel::top("top-bar")
            .frame(base.inner_margin(egui::Margin::symmetric(10, 0)))
            .exact_size(theme::TOP_BAR)
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status")
            .frame(base.inner_margin(egui::Margin::symmetric(12, 0)))
            .exact_size(theme::STATUS_BAR)
            .show(ui, |ui| self.status_bar(ui, device.as_ref()));
        let g = theme::GUTTER as i8;
        egui::Panel::left("tools")
            .frame(base.inner_margin(egui::Margin {
                left: g,
                right: 0,
                top: g,
                bottom: g,
            }))
            .exact_size(theme::TOOLS_STRIP + theme::GUTTER)
            .resizable(false)
            .show(ui, |ui| self.tools_strip(ui));

        let mut open = self.show_preferences;
        egui::Window::new("Preferences")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(&ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(560.0)
                    .show(ui, |ui| {
                        panels::preferences(
                            ui,
                            &mut self.profile,
                            &mut self.use_no_ai,
                            &mut self.s.brush_size_cursor,
                        );
                        self.video_preferences(ui);
                    })
            });
        self.show_preferences = open;
        let mut open = self.show_icon_sheet;
        egui::Window::new("Icon sheet")
            .open(&mut open)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
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
            s: &mut self.s,
            camera: &mut self.camera,
            drawing: &mut self.drawing,
            node_view: &mut self.node_view,
            timing: &mut self.timing,
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
                        .show_close_buttons(false)
                        .show_leaf_close_all_buttons(false)
                        .show_leaf_collapse_buttons(false)
                        .show_inside(ui, &mut viewer);
                }
            });

        let codec_path = self.codec();
        let downloading = self.codec_download.is_some();
        if self
            .export
            .ui(&ctx, &self.s, codec_path, downloading)
            .download_codec
        {
            self.start_codec_download(&ctx);
        }
        let mut open = self.show_licences;
        egui::Window::new("Licences")
            .open(&mut open)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_size([560.0, 480.0])
            .show(&ctx, |ui| {
                ui.label("Symphony is free software under the GNU General Public License, version 3 or later.");
                ui.separator();
                ui.heading("OpenH264");
                ui.label(codec::NOTICE);
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    ui.label(egui::RichText::new(codec::LICENSE).font(theme::dense()));
                });
            });
        self.show_licences = open;
        self.dialogs(&ctx);

        let pending = self.pen.as_ref().is_some_and(|p| p.has_pending());
        if pending || self.camera.busy() || self.drawing.busy() {
            ctx.request_repaint();
        }
        if self.export.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        // Keep autosave ticking even when idle.
        ctx.request_repaint_after(std::time::Duration::from_secs(5));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let saved = Saved {
            workspace: self.workspace,
            layouts: self.layouts.clone(),
            profile: self.profile,
            use_no_ai: self.use_no_ai,
            pressure_gamma: Some(self.s.pressure_gamma),
            brush_size_cursor: self.s.brush_size_cursor,
            use_openh264: self.use_openh264,
        };
        eframe::set_value(storage, eframe::APP_KEY, &saved);
    }

    fn on_exit(&mut self) {
        // A clean exit with nothing unsaved needs no recovery file (E7.5).
        if !self.s.dirty() {
            if let Some(r) = &self.recovery {
                r.clear(&self.s.key);
            }
        }
    }
}

/// On macOS the top bar becomes the title bar: content runs under it and
/// the window buttons float at its left (D3.1, D12.2).
fn viewport() -> egui::ViewportBuilder {
    // Design review can force a window size: SYMPHONY_SHOT_SIZE=1280x800.
    let size = std::env::var("SYMPHONY_SHOT_SIZE")
        .ok()
        .and_then(|v| {
            v.split_once('x')
                .and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))
        })
        .unwrap_or([1440.0, 900.0]);
    let v = egui::ViewportBuilder::default()
        .with_title("Symphony")
        .with_inner_size(size)
        .with_min_inner_size([960.0, 600.0]);
    if cfg!(target_os = "macos") {
        v.with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
    } else {
        v
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: viewport(),
        // A forced review size must win over the remembered window size.
        persist_window: std::env::var_os("SYMPHONY_SHOT_SIZE").is_none(),
        ..Default::default()
    };
    eframe::run_native(
        "Symphony",
        options,
        Box::new(|cc| Ok(Box::new(SymphonyApp::new(cc)))),
    )
}
