//! Developer mode: `symphony-launcher --dev <repo>` builds the latest
//! Symphony from a source checkout, with a designed loading screen the
//! whole time (DESIGN §17), then starts it. Used by Symphony Dev.app.
//!
//! Progress is counted from cargo's JSON messages: each finished crate is
//! one step, measured against how many the last build had, so the bar can
//! fill instead of just spinning (D17.3).

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;
use symphony_ui::{feedback, theme};

/// How long the "Starting Symphony" step stays up after launch, covering
/// macOS checking the freshly built program before its window appears.
const START_GRACE: Duration = Duration::from_millis(2500);

enum Ev {
    Crate(String),
    Error(String),
    Finished(bool),
}

enum Phase {
    Building,
    Starting(Instant),
    Failed(String),
}

pub struct DevBuild {
    repo: PathBuf,
    started: Instant,
    rx: Receiver<Ev>,
    done: usize,
    total: Option<usize>,
    current: String,
    errors: Vec<String>,
    phase: Phase,
    child: Option<Child>,
}

fn count_file(repo: &std::path::Path) -> PathBuf {
    repo.join("target").join(".symphony-dev-units")
}

impl DevBuild {
    pub fn new(cc: &eframe::CreationContext<'_>, repo: PathBuf) -> Self {
        theme::apply(&cc.egui_ctx);
        let total = std::fs::read_to_string(count_file(&repo))
            .ok()
            .and_then(|s| s.trim().parse().ok());
        let (tx, rx) = channel();
        let (dir, ctx) = (repo.clone(), cc.egui_ctx.clone());
        std::thread::spawn(move || {
            let child = Command::new("cargo")
                .args([
                    "build",
                    "--release",
                    "-p",
                    "symphony-app",
                    "--message-format=json-render-diagnostics",
                ])
                .current_dir(&dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            let mut child = match child {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Ev::Error(format!(
                        "Couldn't run cargo ({e}). Is Rust installed?"
                    )));
                    let _ = tx.send(Ev::Finished(false));
                    return;
                }
            };
            if let Some(out) = child.stdout.take() {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    match v["reason"].as_str() {
                        Some("compiler-artifact") => {
                            let name = v["target"]["name"].as_str().unwrap_or("").to_owned();
                            let _ = tx.send(Ev::Crate(name));
                        }
                        Some("compiler-message") if v["message"]["level"] == "error" => {
                            let text = v["message"]["rendered"].as_str().unwrap_or("").to_owned();
                            let _ = tx.send(Ev::Error(text));
                        }
                        _ => {}
                    }
                    ctx.request_repaint();
                }
            }
            let ok = child.wait().map(|s| s.success()).unwrap_or(false);
            let _ = tx.send(Ev::Finished(ok));
            ctx.request_repaint();
        });
        DevBuild {
            repo,
            started: Instant::now(),
            rx,
            done: 0,
            total,
            current: String::new(),
            errors: Vec::new(),
            phase: Phase::Building,
            child: None,
        }
    }

    fn launch(&mut self) {
        let exe = self
            .repo
            .join("target")
            .join("release")
            .join(if cfg!(windows) {
                "symphony.exe"
            } else {
                "symphony"
            });
        match Command::new(&exe).current_dir(&self.repo).spawn() {
            Ok(c) => {
                self.child = Some(c);
                self.phase = Phase::Starting(Instant::now());
            }
            Err(e) => {
                self.phase = Phase::Failed(format!("Built Symphony, but couldn't start it ({e})."))
            }
        }
    }

    fn receive(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Ev::Crate(name) => {
                    self.done += 1;
                    self.current = name;
                }
                Ev::Error(e) => self.errors.push(e),
                Ev::Finished(true) => {
                    let _ = std::fs::write(count_file(&self.repo), self.done.to_string());
                    self.launch();
                }
                Ev::Finished(false) => {
                    let msg = if self.errors.is_empty() {
                        "The build failed. Run cargo build in the project for details.".to_owned()
                    } else {
                        self.errors.join("\n")
                    };
                    self.phase = Phase::Failed(msg);
                }
            }
        }
    }
}

impl eframe::App for DevBuild {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receive();
        let elapsed = self.started.elapsed().as_secs_f64();
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG_BASE))
            .show(ui, |ui| match &self.phase {
                Phase::Building => {
                    let fraction = self
                        .total
                        .filter(|t| *t > 0)
                        .map(|t| (self.done as f32 / t as f32).min(0.98));
                    let detail = match (self.current.is_empty(), self.total) {
                        (true, _) => "Checking for changes".to_owned(),
                        (false, Some(t)) => {
                            format!("Compiling {} ({} of about {t})", self.current, self.done)
                        }
                        (false, None) => format!("Compiling {} ({} done)", self.current, self.done),
                    };
                    feedback::loading_screen(
                        ui,
                        "Building the latest Symphony",
                        Some(&detail),
                        fraction,
                        elapsed,
                    );
                }
                Phase::Starting(at) => {
                    let early_exit = self
                        .child
                        .as_mut()
                        .and_then(|c| c.try_wait().ok().flatten());
                    if let Some(status) = early_exit.filter(|s| !s.success()) {
                        self.phase = Phase::Failed(format!(
                            "Symphony quit right after starting ({status})."
                        ));
                    } else if at.elapsed() >= START_GRACE {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    } else {
                        feedback::loading_screen(ui, "Starting Symphony", None, Some(1.0), elapsed);
                        ctx.request_repaint_after(Duration::from_millis(100));
                    }
                }
                Phase::Failed(msg) => {
                    let msg = msg.clone();
                    egui::Frame::new()
                        .inner_margin(egui::Margin::same(20))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("Symphony didn't build")
                                    .font(egui::FontId::proportional(theme::SIZE_HEADING))
                                    .color(theme::TEXT_PRIMARY),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "The latest code has an error. Here's what the compiler said:",
                                )
                                .color(theme::TEXT_SECONDARY),
                            );
                            ui.add_space(8.0);
                            egui::ScrollArea::vertical()
                                .max_height(ui.available_height() - 40.0)
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(msg)
                                            .font(theme::numbers())
                                            .color(theme::TEXT_PRIMARY),
                                    );
                                });
                            ui.add_space(8.0);
                            if ui.button("Close").clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                }
            });
    }
}

/// Run developer mode for the repo at `repo`.
pub fn run(repo: PathBuf) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Symphony Dev")
            .with_inner_size([440.0, 240.0])
            .with_resizable(true)
            .with_min_inner_size([360.0, 200.0]),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Symphony Dev",
        options,
        Box::new(|cc| Ok(Box::new(DevBuild::new(cc, repo)))),
    )
}
