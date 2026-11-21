//! Drag brush files from other apps onto Symphony to import them as Brush
//! presets. Reading runs on a background thread (DESIGN §17): brush sets
//! can be large.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use symphony_brushes::Brush;
use symphony_core::stroke::Tips;

use crate::presets::Preset;
use crate::session::BrushSettings;

/// Where imported tip and grain textures are kept.
pub fn library_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("Symphony").join("brushes"))
}

pub struct Outcome {
    pub presets: Vec<Preset>,
    /// Plain-language notes: what came from where, and what didn't import.
    pub messages: Vec<String>,
}

pub struct BrushImport {
    rx: Receiver<Outcome>,
    pub files: usize,
}

impl BrushImport {
    pub fn start(paths: Vec<PathBuf>, ctx: &eframe::egui::Context) -> Self {
        let (tx, rx) = channel();
        let files = paths.len();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let out = run(&paths, library_dir().as_deref());
            let _ = tx.send(out);
            ctx.request_repaint();
        });
        BrushImport { rx, files }
    }

    pub fn poll(&self) -> Option<Outcome> {
        self.rx.try_recv().ok()
    }

    /// Screenshot mode: import on this thread so the shot shows the result.
    pub fn finished(paths: Vec<PathBuf>) -> Self {
        let (tx, rx) = channel();
        let files = paths.len();
        // Never touch the real brush library from a screenshot run.
        let scratch = std::env::temp_dir().join("symphony-shot-brushes");
        let _ = tx.send(run(&paths, Some(&scratch)));
        BrushImport { rx, files }
    }
}

/// Symphony's tools take sizes up to 60 points; bigger stamps are capped.
const MAX_WIDTH: f32 = 60.0;

fn settings(b: &Brush) -> BrushSettings {
    let max = b.diameter.clamp(0.5, MAX_WIDTH);
    BrushSettings {
        min_width: (max * b.min_fraction).min(max),
        max_width: max,
        smoothing: b.smoothing.clamp(0.0, 0.95),
        tips: Tips::default(),
        stencil: None,
    }
}

/// Save a texture under a name made from its pixels, so the same tip
/// imported twice is stored once.
fn save(img: &image::GrayImage, dir: &Path) -> Option<String> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (img.width(), img.height()).hash(&mut h);
    img.as_raw().hash(&mut h);
    let name = format!("{:016x}.png", h.finish());
    let path = dir.join(&name);
    if !path.exists() {
        std::fs::create_dir_all(dir).ok()?;
        img.save(&path).ok()?;
    }
    Some(name)
}

fn run(paths: &[PathBuf], library: Option<&Path>) -> Outcome {
    let mut out = Outcome {
        presets: Vec::new(),
        messages: Vec::new(),
    };
    for path in paths {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match symphony_brushes::import(path) {
            Ok(got) => {
                let mut added = 0;
                let mut erasers = 0;
                let mut source = None;
                for b in &got.brushes {
                    if b.eraser {
                        erasers += 1;
                        continue;
                    }
                    source = Some(b.source.title());
                    out.presets.push(Preset {
                        name: b.name.clone(),
                        settings: settings(b),
                        source: Some(b.source.title().to_owned()),
                        tip: b.tip.as_ref().zip(library).and_then(|(t, d)| save(t, d)),
                        grain: b.grain.as_ref().zip(library).and_then(|(g, d)| save(g, d)),
                    });
                    added += 1;
                }
                if added > 0 {
                    out.messages.push(format!(
                        "{added} brush{} from {file}{}",
                        if added == 1 { "" } else { "es" },
                        source.map_or(String::new(), |s| format!(" ({s})"))
                    ));
                }
                if erasers > 0 {
                    out.messages.push(format!(
                        "{erasers} eraser preset{} skipped: eraser presets aren't supported yet",
                        if erasers == 1 { "" } else { "s" }
                    ));
                }
                out.messages
                    .extend(got.problems.iter().map(|p| format!("{file}: {p}")));
            }
            Err(e) => out.messages.push(format!("{file}: {e}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_fit_symphony_tools() {
        let mut b = symphony_brushes::import_bytes(
            "myb",
            "Pen",
            br#"{"settings":{"radius_logarithmic":{"base_value":6.0,"inputs":{}}}}"#,
        )
        .unwrap()
        .brushes
        .remove(0);
        b.min_fraction = 0.25;
        let s = settings(&b);
        assert_eq!(s.max_width, MAX_WIDTH);
        assert_eq!(s.min_width, 15.0);
    }

    #[test]
    fn bad_files_report_instead_of_failing() {
        let dir = std::env::temp_dir().join(format!("sym-bi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("broken.kpp");
        std::fs::write(&bad, b"nope").unwrap();
        let out = run(&[bad, dir.join("missing.myb")], None);
        assert!(out.presets.is_empty());
        assert_eq!(out.messages.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }
}
