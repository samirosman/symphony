//! One import for everything: File > Import… (and the top bar's Import
//! button) and files dropped on the window go through here, which works
//! out what each file is and sends it to the right importer.
//!
//! - Images become a layer holding the picture; numbered images
//!   ("walk_001.png", "walk_002.png"...) become one layer, a drawing per
//!   frame.
//! - Brushes and brush presets go to the brush importer.
//! - Harmony scenes and Symphony projects open as scenes.
//!
//! Copying and reading files happens on a background thread (DESIGN §17).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

/// What a file is, from its extension (and, for folders, its contents).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Sound,
    Video,
    Brush,
    Preset,
    Project,
    HarmonyScene,
    Unknown,
}

const IMAGES: [&str; 10] = [
    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif", "tga", "svg",
];
const SOUNDS: [&str; 8] = ["wav", "mp3", "ogg", "flac", "m4a", "aac", "aif", "aiff"];
const VIDEOS: [&str; 3] = ["mp4", "mov", "m4v"];

pub fn classify(path: &Path) -> Kind {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if path.is_dir() {
        return if path.join("project.json").is_file() {
            Kind::Project
        } else {
            Kind::Unknown
        };
    }
    match ext.as_str() {
        e if IMAGES.contains(&e) => Kind::Image,
        e if SOUNDS.contains(&e) => Kind::Sound,
        e if VIDEOS.contains(&e) => Kind::Video,
        "xstage" => Kind::HarmonyScene,
        e if e == crate::presets::EXTENSION => Kind::Preset,
        _ if symphony_brushes::is_brush_file(path) => Kind::Brush,
        _ => Kind::Unknown,
    }
}

/// Every extension Import… offers in its file picker.
pub fn extensions() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = IMAGES.to_vec();
    all.extend(SOUNDS);
    all.extend(VIDEOS);
    all.extend(["xstage", crate::presets::EXTENSION]);
    all.extend(symphony_brushes::EXTENSIONS.iter().copied());
    all
}

/// Where imported files wait before a project is saved (and where
/// unsaved or recovered scenes find them).
pub fn cache_dir() -> Option<PathBuf> {
    if std::env::var_os("SYMPHONY_SCREENSHOT").is_some() {
        return Some(std::env::temp_dir().join("symphony-shot-media"));
    }
    dirs::data_dir().map(|d| d.join("Symphony").join("media"))
}

/// A stable content key (FNV-1a), also the media file's name.
pub fn content_key(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// One image, read and copied into the media cache.
#[derive(Clone, Debug)]
pub struct Staged {
    pub key: u64,
    pub file: String,
    pub name: String,
    pub size: [u32; 2],
}

/// Images to place: one per layer, or a numbered sequence on one layer.
#[derive(Clone, Debug)]
pub struct ImageGroup {
    pub name: String,
    pub frames: Vec<Staged>,
}

/// Vector drawings to place (SVG files, vectorized pictures): one layer
/// per group, a drawing per frame.
#[derive(Clone, Debug)]
pub struct VectorGroup {
    pub name: String,
    pub frames: Vec<crate::vector_import::VectorArt>,
}

pub struct Outcome {
    pub groups: Vec<ImageGroup>,
    pub vectors: Vec<VectorGroup>,
    pub problems: Vec<String>,
}

/// How pictures come in (Harmony 27's import modes): kept as they are
/// (a locked picture), or vectorized into a drawing every tool can edit.
/// SVG files always come in as vector drawings.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImportOptions {
    pub as_vectors: bool,
    pub vectorize: crate::vector_import::Vectorize,
}

pub struct ImageImport {
    rx: Receiver<Outcome>,
    pub files: usize,
}

impl ImageImport {
    pub fn start(paths: Vec<PathBuf>, opts: ImportOptions, ctx: &eframe::egui::Context) -> Self {
        let (tx, rx) = channel();
        let files = paths.len();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(stage_images(&paths, opts));
            ctx.request_repaint();
        });
        ImageImport { rx, files }
    }

    /// Screenshot runs import on this thread.
    pub fn finished(paths: Vec<PathBuf>, opts: ImportOptions) -> Self {
        let (tx, rx) = channel();
        let files = paths.len();
        let _ = tx.send(stage_images(&paths, opts));
        ImageImport { rx, files }
    }

    pub fn poll(&self) -> Option<Outcome> {
        self.rx.try_recv().ok()
    }
}

/// Split "walk_012" into ("walk_", 12).
fn numbered(stem: &str) -> Option<(String, u64)> {
    let digits = stem.chars().rev().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits == stem.len() && stem.len() > 6 {
        return None;
    }
    let (head, tail) = stem.split_at(stem.len() - digits);
    Some((head.to_owned(), tail.parse().ok()?))
}

fn stage_images(paths: &[PathBuf], opts: ImportOptions) -> Outcome {
    match cache_dir() {
        Some(cache) => stage_into(paths, &cache, opts),
        None => Outcome {
            groups: Vec::new(),
            vectors: Vec::new(),
            problems: vec!["There's nowhere to keep imported files on this computer.".into()],
        },
    }
}

/// Read `paths` as pictures, copy them into `cache` under their content
/// key, and group numbered ones into sequences.
#[cfg(test)]
pub fn stage_images_into(paths: &[PathBuf], cache: &Path) -> Outcome {
    stage_into(paths, cache, ImportOptions::default())
}

/// Read `paths` as `opts` says: SVG files (and, with `as_vectors`, every
/// picture) become vector drawings; the rest are copied into `cache` as
/// pictures. Numbered files group into sequences either way.
pub fn stage_into(paths: &[PathBuf], cache: &Path, opts: ImportOptions) -> Outcome {
    use rayon::prelude::*;
    let is_svg = |p: &PathBuf| {
        p.extension()
            .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("svg"))
    };
    let (vector_paths, picture_paths): (Vec<PathBuf>, Vec<PathBuf>) = paths
        .iter()
        .cloned()
        .partition(|p| is_svg(p) || opts.as_vectors);
    // Each file traces on its own (and each trace uses the cores too).
    let traced: Vec<(PathBuf, Result<crate::vector_import::VectorArt, String>)> = vector_paths
        .par_iter()
        .map(|p| (p.clone(), read_vectors(p, opts)))
        .collect();
    let mut problems = Vec::new();
    let mut fallback = Vec::new();
    let mut arts = Vec::new();
    for (path, r) in traced {
        let shown = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        match r {
            Ok(art) => arts.push((path, art)),
            // An SVG made only of pictures comes in as a picture.
            Err(e) if is_svg(&path) && e.contains("pictures") => fallback.push(path),
            Err(e) => problems.push(format!("{shown}: {e}")),
        }
    }
    let vectors = group_numbered(arts.into_iter().map(|(p, a)| {
        let name = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        (p, name, a)
    }))
    .into_iter()
    .map(|(name, frames)| VectorGroup { name, frames })
    .collect();
    let mut out = stage_pictures(&[picture_paths, fallback].concat(), cache);
    out.vectors = vectors;
    problems.append(&mut out.problems);
    out.problems = problems;
    out
}

/// One file as a vector drawing.
fn read_vectors(
    path: &Path,
    opts: ImportOptions,
) -> Result<crate::vector_import::VectorArt, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("couldn't read it ({e})"))?;
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "svg" {
        return crate::vector_import::from_svg(&bytes);
    }
    let img = decode(&bytes, &ext).ok_or("not an image Symphony can read")?;
    let art = crate::vector_import::vectorize(&img.into_rgba8(), opts.vectorize);
    if art.pieces.is_empty() {
        return Err("nothing to trace in it (try a lower threshold, or Colour)".into());
    }
    Ok(art)
}

/// Decode a picture, trusting its extension first (TGA has no signature).
fn decode(bytes: &[u8], ext: &str) -> Option<image::DynamicImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes));
    match image::ImageFormat::from_extension(ext) {
        Some(f) => reader.set_format(f),
        None => reader = reader.with_guessed_format().ok()?,
    }
    reader.decode().ok()
}

fn stage_pictures(paths: &[PathBuf], cache: &Path) -> Outcome {
    let mut problems = Vec::new();
    let mut staged: Vec<(PathBuf, Staged)> = Vec::new();
    let _ = std::fs::create_dir_all(cache);
    for path in paths {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let shown = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                problems.push(format!("{shown}: couldn't read it ({e})"));
                continue;
            }
        };
        let mut ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_else(|| "png".into());
        // SVG is drawn into a sharp picture here; the rest of Symphony
        // then treats it like any other image.
        let bytes = if ext == "svg" {
            match rasterize_svg(&bytes) {
                Ok(png) => {
                    ext = "png".into();
                    png
                }
                Err(e) => {
                    problems.push(format!("{shown}: {e}"));
                    continue;
                }
            }
        } else {
            bytes
        };
        // The extension says the format first: TGA has no signature to
        // recognise it by, so guessing from the contents alone fails.
        let reader = image::ImageReader::new(std::io::Cursor::new(&bytes));
        let reader = match image::ImageFormat::from_extension(&ext) {
            Some(f) => {
                let mut r = reader;
                r.set_format(f);
                r
            }
            None => reader
                .with_guessed_format()
                .unwrap_or_else(|_| image::ImageReader::new(std::io::Cursor::new(&bytes))),
        };
        let size = match reader.into_dimensions().ok() {
            Some((w, h)) if w > 0 && h > 0 && w <= 16_384 && h <= 16_384 => [w, h],
            _ => {
                problems.push(format!("{shown}: not an image Symphony can read"));
                continue;
            }
        };
        let key = content_key(&bytes);
        let file = format!("{key:016x}.{ext}");
        let dest = cache.join(&file);
        if !dest.is_file() {
            if let Err(e) = std::fs::write(&dest, &bytes) {
                problems.push(format!("{shown}: couldn't copy it ({e})"));
                continue;
            }
        }
        staged.push((
            path.clone(),
            Staged {
                key,
                file,
                name,
                size,
            },
        ));
    }
    Outcome {
        groups: group(staged),
        vectors: Vec::new(),
        problems,
    }
}

/// Numbered items with the same name before the number form one sequence
/// (in number order); everything else is a group of its own.
fn group_numbered<T>(
    items: impl IntoIterator<Item = (PathBuf, String, T)>,
) -> Vec<(String, Vec<T>)> {
    // (sequence name before the number, shown name, numbered items)
    type Group<T> = (Option<String>, String, Vec<(u64, T)>);
    let mut groups: Vec<Group<T>> = Vec::new();
    for (path, name, item) in items {
        match numbered(&name).filter(|_| path.parent().is_some()) {
            Some((head, n)) => {
                if let Some(g) = groups.iter_mut().find(|g| g.0.as_deref() == Some(&head)) {
                    g.2.push((n, item));
                } else {
                    let shown = head.trim_end_matches(['_', '-', '.', ' ']).to_owned();
                    let shown = if shown.is_empty() {
                        name.clone()
                    } else {
                        shown
                    };
                    groups.push((Some(head), shown, vec![(n, item)]));
                }
            }
            None => groups.push((None, name, vec![(0, item)])),
        }
    }
    groups
        .into_iter()
        .map(|(_, name, mut items)| {
            items.sort_by_key(|(n, _)| *n);
            (name, items.into_iter().map(|(_, t)| t).collect())
        })
        .collect()
}

fn group(staged: Vec<(PathBuf, Staged)>) -> Vec<ImageGroup> {
    group_numbered(staged.into_iter().map(|(p, s)| (p, s.name.clone(), s)))
        .into_iter()
        .map(|(name, frames)| ImageGroup { name, frames })
        .collect()
}

/// Longest side, in pixels, an SVG is drawn at: sharp on a 4K frame.
const SVG_EDGE: f32 = 4096.0;

/// Draw an SVG into a PNG, its longest side `SVG_EDGE`.
fn rasterize_svg(svg: &[u8]) -> Result<Vec<u8>, String> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default())
        .map_err(|_| "not an SVG Symphony can read".to_owned())?;
    let size = tree.size();
    let k = SVG_EDGE / size.width().max(size.height()).max(1.0);
    let (w, h) = (
        (size.width() * k).round().max(1.0) as u32,
        (size.height() * k).round().max(1.0) as u32,
    );
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or("the SVG has no size")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(k, k),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|e| format!("couldn't convert the SVG ({e})"))
}

/// File > Import's question for pictures (Harmony 27's Import Images
/// modes): keep each as it is, or vectorize it into a drawing every tool
/// can edit. Returns Some(true) to import, Some(false) to cancel.
pub fn options_ui(
    ctx: &eframe::egui::Context,
    paths: &[PathBuf],
    opts: &mut ImportOptions,
) -> Option<bool> {
    use crate::vector_import::Vectorize;
    use eframe::egui;
    use symphony_ui::{theme, widgets};
    let pictures = paths
        .iter()
        .filter(|p| !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")))
        .count();
    let svgs = paths.len() - pictures;
    let mut answer = None;
    let note = |ui: &mut egui::Ui, text: &str| {
        ui.label(
            egui::RichText::new(text)
                .font(theme::dense())
                .color(theme::TEXT_SECONDARY),
        );
    };
    let modal = theme::glass_modal(ctx, "symphony-import-images").show(ctx, |ui| {
        ui.set_width(400.0);
        ui.heading("Import Images");
        ui.add_space(4.0);
        note(
            ui,
            &match (pictures, svgs) {
                (1, 0) => "1 picture".to_owned(),
                (n, 0) => format!("{n} pictures"),
                (n, s) => format!(
                    "{n} picture{}, and {s} SVG file{} (SVG always comes in as a vector drawing)",
                    if n == 1 { "" } else { "s" },
                    if s == 1 { "" } else { "s" }
                ),
            },
        );
        ui.add_space(8.0);
        ui.label("Import as");
        widgets::segmented(
            ui,
            "import-as",
            &mut opts.as_vectors,
            &[(false, "Picture"), (true, "Vector drawing")],
            false,
        );
        if opts.as_vectors {
            note(
                ui,
                "Traced into lines and colour zones that every tool can edit: select, \
                 transform, erase, paint and recolour.",
            );
            ui.add_space(6.0);
            ui.label("Vectorize");
            let mut colour = matches!(opts.vectorize, Vectorize::Colour { .. });
            if widgets::segmented(
                ui,
                "vectorize",
                &mut colour,
                &[(false, "Line art"), (true, "Colour")],
                false,
            ) {
                opts.vectorize = if colour {
                    Vectorize::Colour { colours: 8 }
                } else {
                    Vectorize::LineArt { threshold: 0.5 }
                };
            }
            match &mut opts.vectorize {
                Vectorize::LineArt { threshold } => {
                    note(ui, "Dark marks become lines; the paper becomes clear.");
                    ui.horizontal(|ui| {
                        ui.label("Threshold");
                        widgets::precise_slider(
                            ui,
                            egui::Slider::new(threshold, 0.1..=0.9).fixed_decimals(2),
                        )
                        .on_hover_text("Lower keeps fainter marks; higher keeps only dark ink.");
                    });
                }
                Vectorize::Colour { colours } => {
                    note(
                        ui,
                        "Each main colour becomes colour zones (on Colour Art); the darkest \
                         becomes lines; white paper becomes clear. Its colours join the palette.",
                    );
                    ui.horizontal(|ui| {
                        ui.label("Colours");
                        widgets::precise_slider(ui, egui::Slider::new(colours, 2..=24));
                    });
                }
            }
        } else {
            note(
                ui,
                "Kept exactly as it is, under the drawing. Draw over it on the same layer.",
            );
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, "Import", true).clicked() {
                answer = Some(true);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(false);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// A sound file, copied into the media cache and decoded.
pub struct StagedSound {
    pub key: u64,
    pub file: String,
    pub name: String,
    pub seconds: f32,
}

pub struct SoundOutcome {
    pub sounds: Vec<StagedSound>,
    pub problems: Vec<String>,
}

pub struct SoundImport {
    rx: Receiver<SoundOutcome>,
    pub files: usize,
}

impl SoundImport {
    pub fn start(paths: Vec<PathBuf>, ctx: &eframe::egui::Context) -> Self {
        let (tx, rx) = channel();
        let files = paths.len();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let out = match cache_dir() {
                Some(cache) => stage_sounds_into(&paths, &cache),
                None => SoundOutcome {
                    sounds: Vec::new(),
                    problems: vec![
                        "There's nowhere to keep imported files on this computer.".into()
                    ],
                },
            };
            let _ = tx.send(out);
            ctx.request_repaint();
        });
        SoundImport { rx, files }
    }

    pub fn poll(&self) -> Option<SoundOutcome> {
        self.rx.try_recv().ok()
    }
}

/// Copy sound files into `cache` and decode them (to check they play and
/// to know their length; the decoded sound is kept for playback).
pub fn stage_sounds_into(paths: &[PathBuf], cache: &Path) -> SoundOutcome {
    let _ = std::fs::create_dir_all(cache);
    let mut out = SoundOutcome {
        sounds: Vec::new(),
        problems: Vec::new(),
    };
    for path in paths {
        let shown = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                out.problems
                    .push(format!("{shown}: couldn't read it ({e})"));
                continue;
            }
        };
        let key = content_key(&bytes);
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_else(|| "wav".into());
        let file = format!("{key:016x}.{ext}");
        let dest = cache.join(&file);
        if !dest.is_file() {
            if let Err(e) = std::fs::write(&dest, &bytes) {
                out.problems
                    .push(format!("{shown}: couldn't copy it ({e})"));
                continue;
            }
        }
        match crate::audio::decode(&dest) {
            Ok(d) => {
                let seconds = d.seconds();
                crate::audio::remember(&dest, std::sync::Arc::new(d));
                out.sounds.push(StagedSound {
                    key,
                    file,
                    name: path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    seconds,
                });
            }
            Err(e) => out.problems.push(format!("{shown}: {e}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn staged(name: &str) -> (PathBuf, Staged) {
        (
            PathBuf::from(format!("/x/{name}.png")),
            Staged {
                key: content_key(name.as_bytes()),
                file: format!("{name}.png"),
                name: name.into(),
                size: [4, 4],
            },
        )
    }

    #[test]
    fn files_are_recognised_by_kind() {
        assert_eq!(classify(Path::new("a/bg.PNG")), Kind::Image);
        assert_eq!(classify(Path::new("voice.wav")), Kind::Sound);
        assert_eq!(classify(Path::new("ref.mov")), Kind::Video);
        assert_eq!(classify(Path::new("scene.xstage")), Kind::HarmonyScene);
        assert_eq!(classify(Path::new("Pencil.kpp")), Kind::Brush);
        assert_eq!(classify(Path::new("notes.txt")), Kind::Unknown);
    }

    #[test]
    fn numbered_images_become_one_ordered_sequence() {
        let groups = group(vec![
            staged("walk_010"),
            staged("bg"),
            staged("walk_002"),
            staged("walk_001"),
        ]);
        assert_eq!(groups.len(), 2);
        let walk = groups.iter().find(|g| g.name == "walk").unwrap();
        let names: Vec<&str> = walk.frames.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["walk_001", "walk_002", "walk_010"]);
        assert!(groups.iter().any(|g| g.name == "bg" && g.frames.len() == 1));
    }

    #[test]
    fn content_keys_are_stable() {
        assert_eq!(content_key(b"symphony"), content_key(b"symphony"));
        assert_ne!(content_key(b"a"), content_key(b"b"));
    }
}
