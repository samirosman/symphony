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

const IMAGES: [&str; 11] = [
    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif", "tga", "svg", "exr",
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

pub struct Outcome {
    pub groups: Vec<ImageGroup>,
    pub problems: Vec<String>,
}

pub struct ImageImport {
    rx: Receiver<Outcome>,
    pub files: usize,
}

impl ImageImport {
    pub fn start(paths: Vec<PathBuf>, ctx: &eframe::egui::Context) -> Self {
        let (tx, rx) = channel();
        let files = paths.len();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(stage_images(&paths));
            ctx.request_repaint();
        });
        ImageImport { rx, files }
    }

    /// Screenshot runs import on this thread.
    pub fn finished(paths: Vec<PathBuf>) -> Self {
        let (tx, rx) = channel();
        let files = paths.len();
        let _ = tx.send(stage_images(&paths));
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

fn stage_images(paths: &[PathBuf]) -> Outcome {
    match cache_dir() {
        Some(cache) => stage_images_into(paths, &cache),
        None => Outcome {
            groups: Vec::new(),
            problems: vec!["There's nowhere to keep imported files on this computer.".into()],
        },
    }
}

/// Read `paths`, copy them into `cache` under their content key, and group
/// numbered ones into sequences.
pub fn stage_images_into(paths: &[PathBuf], cache: &Path) -> Outcome {
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
        let too_big =
            std::fs::metadata(path).is_ok_and(|m| m.len() > symphony_core::bitmap::MAX_FILE_BYTES);
        if too_big {
            problems.push(format!("{shown}: the file is larger than 256 MB"));
            continue;
        }
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
            Some((w, h)) if symphony_core::bitmap::size_ok(w, h) => [w, h],
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
        problems,
    }
}

/// Numbered files with the same name before the number form one sequence
/// (in number order); everything else is a group of its own.
fn group(staged: Vec<(PathBuf, Staged)>) -> Vec<ImageGroup> {
    let mut groups: Vec<(Option<String>, ImageGroup, Vec<u64>)> = Vec::new();
    for (path, s) in staged {
        let seq = numbered(&s.name).filter(|_| path.parent().is_some());
        match seq {
            Some((head, n)) => {
                if let Some(g) = groups.iter_mut().find(|g| g.0.as_deref() == Some(&head)) {
                    g.1.frames.push(s);
                    g.2.push(n);
                } else {
                    let name = head.trim_end_matches(['_', '-', '.', ' ']).to_owned();
                    let name = if name.is_empty() {
                        s.name.clone()
                    } else {
                        name
                    };
                    groups.push((
                        Some(head),
                        ImageGroup {
                            name,
                            frames: vec![s],
                        },
                        vec![n],
                    ));
                }
            }
            None => groups.push((
                None,
                ImageGroup {
                    name: s.name.clone(),
                    frames: vec![s],
                },
                vec![0],
            )),
        }
    }
    groups
        .into_iter()
        .map(|(_, mut g, numbers)| {
            let mut order: Vec<(u64, Staged)> = numbers.into_iter().zip(g.frames).collect();
            order.sort_by_key(|(n, _)| *n);
            g.frames = order.into_iter().map(|(_, s)| s).collect();
            g
        })
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

/// A sound file, checked by decoding it and copied into the media cache.
pub struct StagedSound {
    pub key: u64,
    pub name: String,
    /// The scene's record of the file: its name in the media folders and
    /// what its header says.
    pub media: symphony_core::doc::Media,
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
        // Checked against the limits, and decoded once to know it plays
        // (E7.6). Playback decodes it again from the media folder.
        let (key, file, _) = match symphony_audio::import(path) {
            Ok(done) => done,
            Err(e) => {
                out.problems.push(e);
                continue;
            }
        };
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "Sound".into());
        let media = file.media(key, &name);
        let dest = cache.join(&media.file);
        if !dest.is_file() {
            if let Err(e) = std::fs::write(&dest, &*file.bytes) {
                out.problems
                    .push(format!("{shown}: couldn't copy it ({e})"));
                continue;
            }
        }
        out.sounds.push(StagedSound { key, name, media });
    }
    out
}

/// Import `img` as a PNG named `name` through the media cache, as the
/// Import action does, and return the new layer's index. For tests.
#[cfg(test)]
pub fn import_test_image(
    s: &mut crate::session::Session,
    name: &str,
    img: &image::RgbaImage,
) -> usize {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("symphony-test-image-{}-{n}", std::process::id()));
    let cache = dir.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let path = dir.join(format!("{name}.png"));
    img.save(&path).unwrap();
    s.doc.media_dirs = vec![cache.clone()];
    let out = stage_images_into(&[path], &cache);
    assert!(out.problems.is_empty(), "{:?}", out.problems);
    assert_eq!(s.import_images(out.groups), 1);
    (0..s.layer_count())
        .find(|&i| s.layer_name(i) == name)
        .expect("the new layer")
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
