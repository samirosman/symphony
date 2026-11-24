//! Symphony Launcher: the one thing testers install (ENGINEERING E13.4).
//! It checks GitHub Releases, installs and updates Symphony in the
//! background with checksum checks, keeps the previous version for
//! rollback, and launches the version you choose. It works offline with
//! whatever is already installed.

mod dev;
mod install;
mod release;

pub use install::editor_lock;

use std::process::Child;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use eframe::egui;
use semver::Version;
use symphony_ui::motion::{self, Motion};
use symphony_ui::{feedback, theme, widgets};

use install::{Paths, Progress, State};
use release::{Channel, Release};

enum Msg {
    Releases(Result<Vec<Release>, String>),
    Progress(Progress),
    Installed(Result<Version, String>),
}

enum Job {
    Idle,
    Checking,
    Installing(Option<Progress>),
}

struct Launcher {
    update_on_open: bool,
    paths: Paths,
    state: State,
    releases: Vec<Release>,
    job: Job,
    message: Option<String>,
    error: Option<String>,
    running: Vec<(Version, Child)>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// The logo texture, made once.
    mark: Option<egui::TextureHandle>,
    /// When the window first drew, for the fade-in.
    born: Option<f64>,
    shot: Option<std::path::PathBuf>,
    shot_frames: u32,
    shot_asked: bool,
}

impl Launcher {
    fn new(cc: &eframe::CreationContext<'_>, paths: Paths, update_on_open: bool) -> Self {
        theme::apply(&cc.egui_ctx);
        let (tx, rx) = channel();
        let state = paths.load_state();
        let mut l = Launcher {
            update_on_open,
            paths,
            state,
            releases: Vec::new(),
            job: Job::Idle,
            message: None,
            error: None,
            running: Vec::new(),
            tx,
            rx,
            mark: None,
            born: None,
            shot: std::env::var_os("SYMPHONY_LAUNCHER_SHOT").map(Into::into),
            shot_frames: 0,
            shot_asked: false,
        };
        l.check(&cc.egui_ctx);
        l
    }

    fn check(&mut self, ctx: &egui::Context) {
        self.job = Job::Checking;
        self.error = None;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Releases(release::fetch_releases()));
            ctx.request_repaint();
        });
    }

    fn start_install(&mut self, ctx: &egui::Context, rel: Release) {
        self.job = Job::Installing(None);
        self.error = None;
        let (tx, ctx, paths) = (self.tx.clone(), ctx.clone(), self.paths.clone());
        std::thread::spawn(move || {
            let report = |p| {
                let _ = tx.send(Msg::Progress(p));
                ctx.request_repaint();
            };
            let result = install::install(&paths, &rel, &report);
            let _ = tx.send(Msg::Installed(result));
            ctx.request_repaint();
        });
    }

    fn receive(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Releases(Ok(r)) => {
                    self.releases = r;
                    self.job = Job::Idle;
                    // A first install while only previews exist: use
                    // Preview straight away rather than showing an empty
                    // Stable channel. Once Stable has a release, new users
                    // stay on it.
                    if self.state.channel == Channel::Stable
                        && self.paths.installed().is_empty()
                        && release::newest(&self.releases, Channel::Stable).is_none()
                        && release::newest(&self.releases, Channel::Preview).is_some()
                    {
                        self.state.channel = Channel::Preview;
                        self.save();
                    }
                }
                Msg::Releases(Err(e)) => {
                    self.error = Some(e);
                    self.job = Job::Idle;
                }
                Msg::Progress(p) => self.job = Job::Installing(Some(p)),
                Msg::Installed(Ok(v)) => {
                    self.job = Job::Idle;
                    self.message = Some(format!("Symphony {} is installed.", pretty(&v)));
                    // Keep the new version and the one before it, for rollback.
                    let keep: Vec<Version> = self.paths.installed().into_iter().take(2).collect();
                    let running: Vec<Version> =
                        self.running.iter().map(|(v, _)| v.clone()).collect();
                    install::prune(&self.paths, &keep, &running);
                    self.state.selected = None;
                    self.save();
                }
                Msg::Installed(Err(e)) => {
                    self.job = Job::Idle;
                    self.error = Some(e);
                }
            }
        }
        self.running
            .retain_mut(|(_, c)| matches!(c.try_wait(), Ok(None)));
    }

    fn save(&mut self) {
        if let Err(e) = self.paths.save_state(&self.state) {
            self.error = Some(format!("Couldn't save launcher settings ({e})."));
        }
    }

    fn selected(&self, installed: &[Version]) -> Option<Version> {
        let chosen = self
            .state
            .selected
            .as_deref()
            .and_then(|s| Version::parse(s).ok());
        chosen
            .filter(|v| installed.contains(v))
            .or_else(|| installed.first().cloned())
    }

    fn busy(&self) -> bool {
        !matches!(self.job, Job::Idle)
    }
}

/// Errors: a red marker plus plain text (D4.1 red only as a marker, D10.3).
fn error_line(ui: &mut egui::Ui, text: &str) {
    ui.horizontal_wrapped(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(r.center(), 4.0, theme::STATE_DANGER);
        ui.label(text);
    });
}

fn open_folder(path: &std::path::Path) {
    let _ = std::fs::create_dir_all(path);
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(cmd).arg(path).spawn();
}

impl eframe::App for Launcher {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive();
        if !self.running.is_empty() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        let installed = self.paths.installed();
        let selected = self.selected(&installed);
        let newest = release::newest(&self.releases, self.state.channel).cloned();
        let newest_v = newest.as_ref().and_then(Release::version);
        let update = newest_v
            .as_ref()
            .filter(|nv| installed.first().is_none_or(|iv| *nv > iv))
            .cloned();

        if matches!(self.job, Job::Idle) && std::mem::take(&mut self.update_on_open) {
            if let (Some(_), Some(rel)) = (&update, &newest) {
                self.start_install(&ctx, rel.clone());
            }
        }

        self.screenshot(&ctx);

        // The backdrop: the app's Studio wallpaper.
        let area = ui.max_rect();
        studio_backdrop(ui.painter(), area);
        if cfg!(target_os = "macos") {
            // No title bar (like the app): the top strip drags the window.
            let top = egui::Rect::from_min_size(area.min, egui::vec2(area.width(), 36.0));
            let drag = ui.interact(
                top,
                egui::Id::unique("launcher-titlebar"),
                egui::Sense::drag(),
            );
            if drag.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        }

        // Logo, title, card and footer fade in a beat apart, like the
        // app's start screen.
        let now = ctx.input(|i| i.time);
        let born = *self.born.get_or_insert(now);
        let step = |delay: f64| -> f32 {
            if !motion::enabled(&ctx) {
                return 1.0;
            }
            let t = ((now - born - delay) / Motion::Appear.seconds()).clamp(0.0, 1.0) as f32;
            if t < 1.0 {
                ctx.request_repaint();
            }
            Motion::Appear.ease(t)
        };
        let (logo_in, card_in, foot_in) = (step(0.0), step(0.1), step(0.2));

        // One centred column, centred vertically by last frame's height.
        let width = COLUMN.min(area.width() - 32.0);
        let height_id = egui::Id::unique("launcher-height");
        let last_h: f32 = ctx.data(|d| d.get_temp(height_id)).unwrap_or(460.0);
        let top = (area.top() + ((area.height() - last_h) / 2.0).max(40.0)).round();
        let column = egui::Rect::from_min_size(
            egui::pos2((area.center().x - width / 2.0).round(), top),
            egui::vec2(width, area.bottom() - top),
        );
        let mut col = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(column)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        let mark = self
            .mark
            .get_or_insert_with(|| {
                let img = image::load_from_memory(LOGO)
                    .expect("embedded logo")
                    .into_rgba8();
                ctx.load_texture(
                    "Symphony mark",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [img.width() as usize, img.height() as usize],
                        img.as_raw(),
                    ),
                    egui::TextureOptions::LINEAR,
                )
            })
            .id();
        col.scope(|ui| {
            ui.multiply_opacity(logo_in);
            ui.add(
                egui::Image::new(egui::load::SizedTexture::new(mark, egui::Vec2::splat(72.0)))
                    .alt_text("Symphony logo"),
            );
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Symphony").font(theme::wordmark()));
            ui.label(
                egui::RichText::new("Free 2D animation, familiar to Harmony animators.")
                    .color(theme::TEXT_SECONDARY),
            );
        });
        col.add_space(24.0);

        // The card: what to do now, and one big button to do it.
        col.scope(|ui| {
            ui.multiply_opacity(card_in);
            let card = egui::Frame::new()
                .fill(theme::GLASS_PANEL)
                .corner_radius(theme::GLASS_RADIUS)
                .inner_margin(egui::Margin::same(24))
                .show(ui, |ui| {
                    ui.set_width(width - 48.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        self.card(
                            ui,
                            &ctx,
                            &installed,
                            selected.as_ref(),
                            update.as_ref(),
                            newest.as_ref(),
                        );
                    });
                });
            theme::glass_rim(ui.painter(), card.response.rect);
        });
        col.add_space(16.0);

        // The quiet row: channel, check again, install folder.
        col.scope(|ui| {
            ui.multiply_opacity(foot_in);
            // Centred under the card, using last frame's width.
            let foot_id = egui::Id::unique("launcher-foot");
            let foot_w: f32 = ui.ctx().data(|d| d.get_temp(foot_id)).unwrap_or(0.0);
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - foot_w) / 2.0).max(0.0));
                let start = ui.cursor().left();
                ui.spacing_mut().item_spacing.x = 14.0;
                let small = |s: &str| {
                    egui::RichText::new(s)
                        .font(theme::dense())
                        .color(theme::TEXT_SECONDARY)
                };
                ui.label(small("Updates:"));
                let mut ch = self.state.channel;
                for (c, name) in [(Channel::Stable, "Stable"), (Channel::Preview, "Preview")] {
                    let text = if ch == c {
                        egui::RichText::new(name)
                            .font(theme::dense())
                            .color(theme::TEXT_PRIMARY)
                    } else {
                        small(name)
                    };
                    if ui.add(egui::Button::new(text).frame(ch == c)).clicked() {
                        ch = c;
                    }
                }
                if ch != self.state.channel {
                    self.state.channel = ch;
                    self.save();
                }
                ui.label(small("·"));
                if ui
                    .add_enabled(
                        !self.busy(),
                        egui::Button::new(small("Check again")).frame(false),
                    )
                    .clicked()
                {
                    self.message = None;
                    self.check(&ctx);
                }
                if ui
                    .add(egui::Button::new(small("Install folder")).frame(false))
                    .clicked()
                {
                    open_folder(&self.paths.root);
                }
                let w = ui.min_rect().right() - start;
                if (w - foot_w).abs() > 0.5 {
                    ui.ctx().data_mut(|d| d.insert_temp(foot_id, w));
                    ui.ctx().request_repaint();
                }
            });
        });
        let used = col.min_rect().height();
        ctx.data_mut(|d| d.insert_temp(height_id, used));
    }
}

impl Launcher {
    /// Inside the card: a headline, a line under it, the main button, then
    /// what's new or other versions.
    fn card(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        installed: &[Version],
        selected: Option<&Version>,
        update: Option<&Version>,
        newest: Option<&Release>,
    ) {
        let headline = |ui: &mut egui::Ui, s: &str| {
            ui.label(
                egui::RichText::new(s)
                    .size(theme::SIZE_HEADING + 3.0)
                    .color(theme::TEXT_PRIMARY),
            );
        };
        let line = |ui: &mut egui::Ui, s: &str| {
            ui.label(egui::RichText::new(s).color(theme::TEXT_SECONDARY));
        };
        let full = ui.available_width();

        // Installing: progress is the whole card.
        if let Job::Installing(p) = &self.job {
            let (frac, text) = match p {
                Some(Progress::Downloading { done, total }) if *total > 0 => (
                    Some(*done as f32 / *total as f32),
                    format!(
                        "Downloading {:.0} of {:.0} MB",
                        *done as f64 / 1e6,
                        *total as f64 / 1e6
                    ),
                ),
                Some(Progress::Verifying) => (None, "Checking the download".to_owned()),
                Some(Progress::Installing) => (None, "Installing".to_owned()),
                _ => (None, "Starting the download".to_owned()),
            };
            let v = update.map(|v| v.to_string()).unwrap_or_default();
            headline(ui, &format!("Installing Symphony {}", pretty_str(&v)));
            ui.add_space(12.0);
            feedback::progress_bar(ui, frac, full);
            ui.add_space(8.0);
            line(ui, &text);
            return;
        }

        match (selected, update, newest) {
            // Nothing installed yet.
            (None, Some(v), Some(rel)) => {
                headline(ui, "Ready to install");
                line(
                    ui,
                    &format!(
                        "Symphony {}, about {} MB.",
                        pretty(v),
                        rel.archive().map_or(0, |a| a.size / 1_000_000).max(1)
                    ),
                );
                ui.add_space(16.0);
                if big_button(ui, &format!("Install Symphony {}", pretty(v)), true, full).clicked()
                {
                    self.start_install(ctx, rel.clone());
                }
                self.whats_new(ui, rel, v);
            }
            (None, _, _) => {
                if matches!(self.job, Job::Checking) {
                    ui.horizontal(|ui| {
                        feedback::spinner(ui, 16.0);
                        line(ui, "Looking for the newest Symphony");
                    });
                } else if self.error.is_none() {
                    headline(ui, "Nothing to install yet");
                    line(
                        ui,
                        "There's no release on this update channel yet. Try Preview below.",
                    );
                }
            }
            // Installed: launch, and offer the update if there is one.
            (Some(v), _, _) => {
                let is_running = self.running.iter().any(|(r, _)| r == v);
                headline(ui, &format!("Symphony {}", pretty(v)));
                match update {
                    Some(nv) => line(ui, &format!("Version {} is available.", pretty(nv))),
                    None if matches!(self.job, Job::Checking) => line(ui, "Checking for updates…"),
                    None if self.error.is_none() && newest.is_none() => {
                        line(ui, "There's no release on this update channel yet.")
                    }
                    None if self.error.is_none() => line(ui, "You have the newest version."),
                    None => {}
                }
                ui.add_space(16.0);
                let label = if is_running {
                    "Symphony is running".to_owned()
                } else {
                    "Launch Symphony".to_owned()
                };
                if big_button(ui, &label, !is_running, full).clicked() {
                    match install::launch(&self.paths, v) {
                        Ok(child) => self.running.push((v.clone(), child)),
                        Err(e) => self.error = Some(e),
                    }
                }
                if let (Some(nv), Some(rel)) = (update, newest) {
                    ui.add_space(8.0);
                    let b = egui::Button::new(format!("Update to {}", pretty(nv)));
                    if ui.add_sized([full, theme::ROW + 4.0], b).clicked() {
                        self.start_install(ctx, rel.clone());
                    }
                    self.whats_new(ui, rel, nv);
                }
                if installed.len() > 1 {
                    ui.add_space(12.0);
                    egui::CollapsingHeader::new(
                        egui::RichText::new("Other versions").color(theme::TEXT_SECONDARY),
                    )
                    .id_salt("versions")
                    .show(ui, |ui| {
                        for iv in installed {
                            let mut label = iv.to_string();
                            if Some(iv) == installed.first() {
                                label.push_str(" (newest)");
                            }
                            if widgets::choice(ui, Some(iv) == selected, &label).clicked() {
                                self.state.selected = Some(iv.to_string());
                                self.save();
                            }
                        }
                    });
                }
            }
        }
        if let Some(m) = &self.message {
            ui.add_space(8.0);
            line(ui, m);
        }
        if let Some(e) = self.error.clone() {
            ui.add_space(8.0);
            error_line(ui, &e);
        }
    }

    /// The release notes, folded away, as readable text (not raw Markdown).
    fn whats_new(&self, ui: &mut egui::Ui, rel: &Release, v: &Version) {
        let Some(notes) = rel.body.as_deref().filter(|b| !b.trim().is_empty()) else {
            return;
        };
        ui.add_space(12.0);
        egui::CollapsingHeader::new(
            egui::RichText::new(format!("What's new in {}", pretty(v)))
                .color(theme::TEXT_SECONDARY),
        )
        .id_salt("whats-new")
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .show(ui, |ui| {
                    release_notes(ui, notes);
                });
        });
    }

    /// SYMPHONY_LAUNCHER_SHOT=<file.png>: save a screenshot once things
    /// settle, then quit (for reviewing the design).
    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.shot.clone() else {
            return;
        };
        self.shot_frames += 1;
        ctx.request_repaint();
        let settled = !matches!(self.job, Job::Checking) && self.shot_frames > 60;
        if settled && !self.shot_asked {
            self.shot_asked = true;
            let ctx2 = ctx.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                egui::ScreenshotCallback::new(move |img: std::sync::Arc<egui::ColorImage>| {
                    let [w, h] = img.size;
                    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                    let _ = image::save_buffer(
                        &path,
                        &bytes,
                        w as u32,
                        h as u32,
                        image::ColorType::Rgba8,
                    );
                    ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
                }),
            ));
        }
    }
}

/// The column the logo, title and card share.
const COLUMN: f32 = 440.0;
const LOGO: &[u8] = include_bytes!("../../symphony-app/assets/Symphony.png");

/// The app's "Studio" wallpaper: a soft four-corner gradient.
fn studio_backdrop(painter: &egui::Painter, rect: egui::Rect) {
    let c = |[r, g, b]: [u8; 3]| egui::Color32::from_rgb(r, g, b);
    let [tl, tr, br, bl] = [[46, 56, 59], [28, 36, 43], [21, 25, 31], [35, 40, 47]].map(c);
    let mut mesh = egui::Mesh::default();
    for (p, col) in [
        (rect.left_top(), tl),
        (rect.right_top(), tr),
        (rect.right_bottom(), br),
        (rect.left_bottom(), bl),
    ] {
        mesh.colored_vertex(p, col);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// The one thing to do, big and blue.
fn big_button(ui: &mut egui::Ui, label: &str, enabled: bool, width: f32) -> egui::Response {
    let text = egui::RichText::new(label)
        .size(theme::SIZE_HEADING)
        .color(if enabled {
            theme::ON_ACCENT
        } else {
            theme::TEXT_DISABLED
        });
    let b = egui::Button::new(text)
        .corner_radius(theme::GLASS_CONTROL_RADIUS)
        .fill(if enabled {
            theme::ACCENT_SELECT_FILL
        } else {
            theme::BG_RAISED
        });
    // add_sized centres the label across the full-width button.
    ui.add_enabled_ui(enabled, |ui| ui.add_sized([width, 40.0], b))
        .inner
}

/// 0.1.0-preview.3 -> "0.1.0 preview 3": how people say it.
fn pretty_str(v: &str) -> String {
    Version::parse(v).map_or_else(|_| v.to_owned(), |v| pretty(&v))
}

fn pretty(v: &Version) -> String {
    let base = format!("{}.{}.{}", v.major, v.minor, v.patch);
    if v.pre.is_empty() {
        return base;
    }
    format!("{base} {}", v.pre.as_str().replace('.', " "))
}

/// Release notes are Markdown: show headings, bullets and bold as text.
fn release_notes(ui: &mut egui::Ui, md: &str) {
    let plain = |s: &str| -> String {
        // [text](url) -> text, and drop ** and backticks.
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find('[') {
            out.push_str(&rest[..i]);
            let after = &rest[i + 1..];
            match (after.find("]("), after.find(')')) {
                (Some(j), Some(k)) if k > j => {
                    out.push_str(&after[..j]);
                    rest = &after[k + 1..];
                }
                _ => {
                    out.push('[');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out.replace("**", "").replace('`', "")
    };
    let mut para = String::new();
    let flush = |ui: &mut egui::Ui, para: &mut String| {
        if !para.trim().is_empty() {
            ui.label(egui::RichText::new(plain(para.trim())).color(theme::TEXT_SECONDARY));
        }
        para.clear();
    };
    for raw in md.lines() {
        let l = raw.trim_end();
        if l.starts_with("# ") {
            continue; // The title repeats the version.
        } else if let Some(h) = l.strip_prefix("## ") {
            flush(ui, &mut para);
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(plain(h))
                    .color(theme::TEXT_PRIMARY)
                    .strong(),
            );
        } else if let Some(item) = l.trim_start().strip_prefix("- ") {
            flush(ui, &mut para);
            para.push_str("•  ");
            para.push_str(item);
        } else if l.trim().is_empty() {
            flush(ui, &mut para);
        } else {
            para.push(' ');
            para.push_str(l.trim());
        }
    }
    flush(ui, &mut para);
}

pub fn run() -> eframe::Result {
    // Developer mode, used by Symphony Dev.app: build from source, then launch.
    let args: Vec<String> = std::env::args().collect();
    // Smoke tests without the window: --check [stable|preview], --install [stable|preview].
    if let Some(cmd @ ("--check" | "--install")) = args.get(1).map(String::as_str) {
        std::process::exit(cli(cmd, args.get(2).map(String::as_str)));
    }
    if args.get(1).map(String::as_str) == Some("--dev") {
        let repo = args
            .get(2)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        return dev::run(repo);
    }
    run_release_manager(false)
}

pub fn run_release_manager(update_on_open: bool) -> eframe::Result {
    // SYMPHONY_LAUNCHER_ROOT: use another install folder (reviews, tests).
    let paths = std::env::var_os("SYMPHONY_LAUNCHER_ROOT")
        .map(|root| Paths { root: root.into() })
        .or_else(Paths::default_location);
    let Some(paths) = paths else {
        eprintln!("Couldn't find a folder to install Symphony into.");
        std::process::exit(1);
    };
    let icon = image::load_from_memory(LOGO).ok().map(|i| {
        let i = i.into_rgba8();
        egui::IconData {
            width: i.width(),
            height: i.height(),
            rgba: i.into_raw(),
        }
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Symphony Launcher")
        .with_inner_size([520.0, 600.0])
        .with_min_inner_size([440.0, 520.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    if cfg!(target_os = "macos") {
        // Like the app: content runs under the window buttons.
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Symphony Launcher",
        options,
        Box::new(|cc| Ok(Box::new(Launcher::new(cc, paths, update_on_open)))),
    )
}

/// Command-line checks used to test releases end to end.
fn cli(cmd: &str, channel: Option<&str>) -> i32 {
    let channel = if channel == Some("preview") {
        Channel::Preview
    } else {
        Channel::Stable
    };
    let Some(paths) = Paths::default_location() else {
        eprintln!("No data folder.");
        return 1;
    };
    let releases = match release::fetch_releases() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let Some(rel) = release::newest(&releases, channel) else {
        println!(
            "No {} release with a build for this computer.",
            channel.title()
        );
        return 2;
    };
    println!(
        "Newest {}: {} ({})",
        channel.title(),
        rel.tag_name,
        rel.archive().map_or("no build", |a| a.name.as_str())
    );
    if cmd == "--install" {
        match install::install(&paths, rel, &|p| {
            if let Progress::Downloading { done, total } = p {
                eprint!(
                    "\rDownloading {:.1} of {:.1} MB",
                    done as f64 / 1e6,
                    total as f64 / 1e6
                );
            }
        }) {
            Ok(v) => println!("\nInstalled {v} at {}", paths.version_dir(&v).display()),
            Err(e) => {
                eprintln!("\n{e}");
                return 1;
            }
        }
    }
    0
}
