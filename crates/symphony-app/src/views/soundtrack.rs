//! File > Export > Soundtrack, after Harmony 27's Export Soundtracks
//! dialog: output path, export range, merged or one file per track, and the
//! sound settings (rate, 8 or 16 bits, mono or stereo). The work runs on a
//! background thread from a snapshot of the scene, with progress and Cancel
//! (ENGINEERING E5.2, E5.3). The mixing is in `symphony_audio::soundtrack`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use eframe::egui;
use symphony_audio::soundtrack::{self, SampleSize, SoundtrackJob, RATES};
use symphony_ui::{theme, widgets};

use crate::session::Session;

const TITLE: &str = "Export Soundtrack";

enum Msg {
    Progress(u32, u32),
    Done(Result<Vec<PathBuf>, String>),
}

struct Running {
    cancel: Arc<AtomicBool>,
    rx: Receiver<Msg>,
    done: u32,
    total: u32,
}

pub struct SoundtrackDialog {
    open: bool,
    path: Option<PathBuf>,
    all_frames: bool,
    first: u32,
    last: u32,
    merge: bool,
    rate: u32,
    size: SampleSize,
    channels: u16,
    running: Option<Running>,
    result: Option<Result<Vec<PathBuf>, String>>,
}

impl Default for SoundtrackDialog {
    fn default() -> Self {
        SoundtrackDialog {
            open: false,
            path: None,
            all_frames: true,
            first: 1,
            last: 1,
            merge: true,
            rate: 48_000,
            size: SampleSize::Sixteen,
            channels: 2,
            running: None,
            result: None,
        }
    }
}

impl SoundtrackDialog {
    pub fn open(&mut self, s: &Session) {
        if self.running.is_some() {
            return;
        }
        self.open = true;
        self.last = s.length();
        self.first = self.first.clamp(1, self.last);
        self.result = None;
        self.path = None;
    }

    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    /// Tracks done so far, for the status bar (DESIGN D17.5).
    pub fn progress(&self) -> Option<(u32, u32)> {
        self.running.as_ref().map(|r| (r.done, r.total))
    }

    fn browse(&mut self, s: &Session) {
        if let Some(mut p) = rfd::FileDialog::new()
            .set_title(TITLE)
            .set_file_name(format!("{}.wav", s.name()))
            .add_filter("wav", &["wav"])
            .save_file()
        {
            if p.extension().is_none() {
                p.set_extension("wav");
            }
            self.path = Some(p);
        }
    }

    fn start(&mut self, s: &Session, ctx: &egui::Context) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let (first, last) = if self.all_frames {
            (1, s.length())
        } else {
            (self.first, self.last.max(self.first))
        };
        let job = SoundtrackJob {
            path,
            first,
            last,
            rate: self.rate,
            size: self.size,
            channels: self.channels,
            merge: self.merge,
        };
        let total = s.doc.sounds.iter().filter(|t| t.enabled).count() as u32;
        let scene = s.doc.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (flag, ctx) = (cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let report = |d, t| {
                let _ = tx.send(Msg::Progress(d, t));
                ctx.request_repaint();
            };
            let r = soundtrack::export(&scene, &job, &flag, &report);
            let _ = tx.send(Msg::Done(r));
            ctx.request_repaint();
        });
        self.running = Some(Running {
            cancel,
            rx,
            done: 0,
            total,
        });
        self.result = None;
    }

    pub fn ui(&mut self, ctx: &egui::Context, s: &Session) {
        if let Some(r) = self.running.as_mut() {
            while let Ok(m) = r.rx.try_recv() {
                match m {
                    Msg::Progress(d, t) => (r.done, r.total) = (d, t),
                    Msg::Done(res) => {
                        self.result = Some(res);
                        self.running = None;
                        break;
                    }
                }
            }
        }
        if !self.open {
            return;
        }
        let mut open = true;
        egui::Window::new(TITLE)
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.set_width(380.0);
                let busy = self.running.is_some();
                ui.add_enabled_ui(!busy, |ui| self.settings(ui, s));
                ui.add_space(10.0);
                if let Some(r) = &self.running {
                    let frac = r.done as f32 / r.total.max(1) as f32;
                    ui.add(egui::ProgressBar::new(frac).text(format!(
                        "Exporting track {} of {}",
                        (r.done + 1).min(r.total),
                        r.total
                    )));
                    if ui.button("Cancel").clicked() {
                        r.cancel.store(true, Ordering::SeqCst);
                    }
                    return;
                }
                match &self.result {
                    Some(Ok(files)) if files.len() == 1 => {
                        ui.label(format!("Exported to {}", files[0].display()));
                    }
                    Some(Ok(files)) => {
                        let dir = files[0].parent().map(|d| d.display().to_string());
                        ui.label(format!(
                            "Exported {} files to {}",
                            files.len(),
                            dir.unwrap_or_default()
                        ));
                    }
                    Some(Err(e)) => {
                        ui.label(e.as_str());
                    }
                    None => {}
                }
                let has_sound = s.doc.sounds.iter().any(|t| t.enabled);
                let ready = self.path.is_some() && has_sound;
                let why = if !has_sound {
                    "The scene has no sound tracks that play. Muted tracks are left out."
                } else {
                    "Choose where to save first"
                };
                if widgets::primary_button(ui, "Export", ready)
                    .on_disabled_hover_text(why)
                    .clicked()
                {
                    self.start(s, ctx);
                }
            });
        if !open && self.running.is_none() {
            self.open = false;
        }
    }

    fn settings(&mut self, ui: &mut egui::Ui, s: &Session) {
        let note = |ui: &mut egui::Ui, text: &str| {
            ui.label(
                egui::RichText::new(text)
                    .font(theme::dense())
                    .color(theme::TEXT_SECONDARY),
            );
        };
        ui.label("Output");
        ui.horizontal(|ui| {
            let shown = self
                .path
                .as_ref()
                .map_or("Not chosen".to_owned(), |p| p.display().to_string());
            note(ui, &shown);
            if ui.button("Browse...").clicked() {
                self.browse(s);
            }
        });
        ui.add_space(6.0);
        ui.label("Export range");
        let len = s.length();
        widgets::radio_value(ui, &mut self.all_frames, true, &format!("All (1 to {len})"));
        ui.horizontal(|ui| {
            widgets::radio_value(ui, &mut self.all_frames, false, "Frames");
            ui.add(egui::DragValue::new(&mut self.first).range(1..=len));
            ui.label("to");
            ui.add(egui::DragValue::new(&mut self.last).range(1..=len));
        });
        ui.add_space(6.0);
        ui.label("Options");
        widgets::radio_value(ui, &mut self.merge, true, "Merge all soundtracks");
        widgets::radio_value(
            ui,
            &mut self.merge,
            false,
            "Process soundtracks individually",
        );
        if !self.merge {
            note(
                ui,
                "One file per track, named after the output: name_Track.wav",
            );
        }
        note(ui, "Muted tracks are left out.");
        ui.add_space(6.0);
        ui.label("Sound settings");
        egui::Grid::new("soundtrack-settings")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label("Rate");
                egui::ComboBox::from_id_salt("soundtrack-rate")
                    .selected_text(rate_label(self.rate))
                    .show_ui(ui, |ui| {
                        for r in RATES {
                            ui.selectable_value(&mut self.rate, r, rate_label(r));
                        }
                    });
                ui.end_row();
                ui.label("Size");
                ui.horizontal(|ui| {
                    widgets::radio_value(ui, &mut self.size, SampleSize::Eight, "8 bits");
                    widgets::radio_value(ui, &mut self.size, SampleSize::Sixteen, "16 bits");
                });
                ui.end_row();
                ui.label("Use");
                ui.horizontal(|ui| {
                    widgets::radio_value(ui, &mut self.channels, 1, "Mono");
                    widgets::radio_value(ui, &mut self.channels, 2, "Stereo");
                });
                ui.end_row();
            });
    }
}

fn rate_label(rate: u32) -> String {
    format!("{rate} Hz")
}
