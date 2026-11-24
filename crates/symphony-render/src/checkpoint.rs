//! Exports that lose nothing (ENGINEERING E5.2). Every finished frame is
//! kept on disk the moment it's rendered, so a crash, a full disk or a
//! Cancel never throws away work: exporting the same scene again picks up
//! where it stopped.
//!
//! - A PNG sequence's frames are its output files. A small manifest next
//!   to them (`.name.png.export`) says which scene they were made from.
//! - A movie or GIF keeps its frames as PNGs in `name.mp4.partial/` while
//!   it's being made, and is written to `name.mp4.part` first. Only a
//!   finished file is renamed to `name.mp4`; then the frames are removed.
//!
//! Every file is written under a temporary name and renamed when complete,
//! so a crash never leaves a half-written frame that looks finished. A
//! frame is only reused if it reads back whole at the right size; anything
//! else (a power cut mid-write) is rendered again.

use std::hash::Hasher;
use std::io::Write;
use std::path::{Path, PathBuf};

use image::RgbaImage;
use symphony_core::doc::Scene;

/// What the frames on disk depend on: the scene, the frame size and the
/// renderer (a new Symphony version may draw differently).
pub fn fingerprint(scene: &Scene, w: u32, h: u32) -> String {
    struct HashWriter(std::collections::hash_map::DefaultHasher);
    impl Write for HashWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut h64 = HashWriter(Default::default());
    // Streamed straight into the hash: no copy of the scene is made.
    let ok = serde_json::to_writer(&mut h64, scene).is_ok();
    format!(
        "symphony {} | {w}x{h} | scene {:016x}{}",
        env!("CARGO_PKG_VERSION"),
        h64.0.finish(),
        if ok { "" } else { " (unhashed)" }
    )
}

/// Where an export keeps its finished frames.
pub struct Checkpoint {
    manifest: PathBuf,
    frames: Frames,
    w: u32,
    h: u32,
    /// The frames on disk were made from this same scene.
    resuming: bool,
}

enum Frames {
    /// The output files themselves (`shot_0001.png`).
    Sequence(PathBuf),
    /// A folder of its own, removed when the export is done.
    Folder(PathBuf),
}

impl Checkpoint {
    /// For a PNG sequence written as `path` (`shot.png` -> `shot_0001.png`).
    pub fn sequence(path: &Path, key: &str, w: u32, h: u32) -> Result<Self, String> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "frames".into());
        let manifest = path
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!(".{name}.export"));
        Self::open(manifest, Frames::Sequence(path.to_path_buf()), key, w, h)
    }

    /// For a movie or GIF written to `out`: frames in `out.partial/`.
    pub fn folder(out: &Path, key: &str, w: u32, h: u32) -> Result<Self, String> {
        let dir = with_suffix(out, ".partial");
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Couldn't make the folder {} ({e}).", dir.display()))?;
        Self::open(dir.join("export"), Frames::Folder(dir), key, w, h)
    }

    fn open(manifest: PathBuf, frames: Frames, key: &str, w: u32, h: u32) -> Result<Self, String> {
        let resuming = std::fs::read_to_string(&manifest).is_ok_and(|s| s.trim() == key);
        if !resuming {
            // Frames from another scene (or none): start over, marking the
            // ones about to be made as this scene's.
            if let Frames::Folder(dir) = &frames {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for e in entries.flatten() {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
            write_atomic(&manifest, key.as_bytes())
                .map_err(|e| format!("Couldn't write {} ({e}).", manifest.display()))?;
        }
        Ok(Checkpoint {
            manifest,
            frames,
            w,
            h,
            resuming,
        })
    }

    pub fn path(&self, frame: u32) -> PathBuf {
        match &self.frames {
            Frames::Sequence(path) => crate::numbered(path, frame),
            Frames::Folder(dir) => dir.join(format!("frame_{frame:05}.png")),
        }
    }

    /// The frame as kept from an earlier, unfinished export of this same
    /// scene, if it reads back whole.
    pub fn load(&self, frame: u32) -> Option<RgbaImage> {
        if !self.resuming {
            return None;
        }
        let img = image::open(self.path(frame)).ok()?.into_rgba8();
        (img.width() == self.w && img.height() == self.h).then_some(img)
    }

    /// Keep a finished frame (written whole or not at all).
    pub fn save(&self, frame: u32, img: &RgbaImage) -> Result<(), String> {
        let out = self.path(frame);
        // Frames that only stand by for a movie compress fast; a PNG
        // sequence's frames are the result, so they get the usual size.
        let compression = match self.frames {
            Frames::Sequence(_) => image::codecs::png::CompressionType::Default,
            Frames::Folder(_) => image::codecs::png::CompressionType::Fast,
        };
        let mut bytes = Vec::new();
        image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new_with_quality(
                &mut bytes,
                compression,
                image::codecs::png::FilterType::Adaptive,
            ),
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| format!("Couldn't compress frame {frame} ({e})."))?;
        write_atomic(&out, &bytes).map_err(|e| {
            format!(
                "Couldn't write {} ({e}). The frames done so far are kept: \
                 export again to continue from there.",
                out.display()
            )
        })
    }

    /// The export is complete: the bookkeeping (and a movie's frames) go.
    pub fn finish(self) {
        let _ = std::fs::remove_file(&self.manifest);
        if let Frames::Folder(dir) = &self.frames {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// `shot.mp4` + `.part` -> `shot.mp4.part`.
pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Write `bytes` to `path` under a temporary name, then rename: the file
/// at `path` is always either the old one or the whole new one.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = with_suffix(path, ".tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}
