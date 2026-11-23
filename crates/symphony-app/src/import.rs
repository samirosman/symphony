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

const IMAGES: [&str; 9] = [
    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif", "tga",
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
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                problems.push(format!("{shown}: couldn't read it ({e})"));
                continue;
            }
        };
        let size = match image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.into_dimensions().ok())
        {
            Some((w, h)) if w > 0 && h > 0 && w <= 16_384 && h <= 16_384 => [w, h],
            _ => {
                problems.push(format!("{shown}: not an image Symphony can read"));
                continue;
            }
        };
        let key = content_key(&bytes);
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_else(|| "png".into());
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
