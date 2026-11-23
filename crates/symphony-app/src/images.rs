//! Imported images in the app: the GPU textures the Camera and Drawing
//! views draw them with, and File > Import > Images.
//!
//! Decoding a large image takes longer than a frame, so decoding and
//! reading files run on worker threads (E5.2) and show in the status bar
//! (D17.5). Textures are kept by content key and alpha encoding, so undo,
//! redo and reopening a scene never decode the same file twice.
//!
//! The views draw a texture of at most [`MAX_TEXTURE`] pixels a side, as
//! Harmony's Camera view shows bitmaps at a reduced size for speed. Export
//! and Render Preview use `symphony-render`, at full size.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui;
use symphony_core::bitmap::{Alpha, ImageFile, ImageRef};

use crate::session::Session;

/// Longest texture side. egui's wgpu renderer makes no mipmaps, so a
/// texture much larger than its size on screen would shimmer; 2048 keeps
/// a full HD background at full size.
pub const MAX_TEXTURE: u32 = 2048;

type Key = (u64, Alpha);

enum Done {
    Imported {
        name: String,
        files: Vec<(u64, ImageFile)>,
        errors: Vec<String>,
    },
    Decoded {
        key: Key,
        image: Option<egui::ColorImage>,
    },
}

pub struct ImageState {
    textures: HashMap<Key, egui::TextureHandle>,
    decoding: HashSet<Key>,
    /// Images that don't decode, so they aren't tried every frame.
    failed: HashSet<Key>,
    importing: usize,
    tx: Sender<Done>,
    rx: Receiver<Done>,
}

impl Default for ImageState {
    fn default() -> Self {
        let (tx, rx) = channel();
        ImageState {
            textures: HashMap::new(),
            decoding: HashSet::new(),
            failed: HashSet::new(),
            importing: 0,
            tx,
            rx,
        }
    }
}

impl ImageState {
    /// The texture of an image, once it is decoded.
    pub fn texture(&self, r: ImageRef) -> Option<&egui::TextureHandle> {
        self.textures.get(&(r.key, r.alpha))
    }

    /// Words for the status bar while work runs (D17.5).
    pub fn activity(&self) -> Option<String> {
        match (self.importing, self.decoding.len()) {
            (0, 0) => None,
            (0, _) => Some("Reading images".into()),
            (1, _) => Some("Importing images".into()),
            (n, _) => Some(format!("Importing {n} sets of images")),
        }
    }

    /// Ask for files to import (File > Import > Images). Several files
    /// become one layer that shows them in name order, one a frame.
    pub fn import_dialog(&mut self, ctx: &egui::Context) {
        let Some(paths) = rfd::FileDialog::new()
            .set_title("Import Images")
            .add_filter("Image", &["png", "jpg", "jpeg", "exr"])
            .pick_files()
        else {
            return;
        };
        self.import(paths, ctx);
    }

    pub fn import(&mut self, mut paths: Vec<PathBuf>, ctx: &egui::Context) {
        if paths.is_empty() {
            return;
        }
        paths.sort();
        self.importing += 1;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let name = layer_name(&paths[0]);
            let mut files = Vec::new();
            let mut errors = Vec::new();
            for path in &paths {
                match symphony_io::media::import_image(path) {
                    Ok(f) => files.push(f),
                    Err(e) => errors.push(e.to_string()),
                }
            }
            let _ = tx.send(Done::Imported {
                name,
                files,
                errors,
            });
            ctx.request_repaint();
        });
    }

    /// Once a UI frame: take finished work, and decode the images the
    /// scene shows that have no texture yet.
    pub fn update(&mut self, s: &mut Session, ctx: &egui::Context) {
        while let Ok(done) = self.rx.try_recv() {
            match done {
                Done::Imported {
                    name,
                    files,
                    errors,
                } => {
                    self.importing -= 1;
                    if !files.is_empty() {
                        s.import_images(&name, files);
                    }
                    if let Some(e) = errors.first() {
                        s.notify(match errors.len() {
                            1 => e.clone(),
                            n => format!("{e} ({} more files weren't imported.)", n - 1),
                        });
                    }
                }
                Done::Decoded { key, image } => {
                    self.decoding.remove(&key);
                    match image {
                        Some(image) => {
                            let name = format!("image-{:016x}", key.0);
                            let tex = ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
                            self.textures.insert(key, tex);
                        }
                        None => {
                            self.failed.insert(key);
                            s.notify("An image in this scene couldn't be read.");
                        }
                    }
                }
            }
        }
        self.decode_missing(s, ctx);
        // Keep only what this scene has.
        let has = |k: &Key| s.doc.images.contains_key(&k.0);
        self.textures.retain(|k, _| has(k));
        self.failed.retain(|k| has(k));
    }

    fn decode_missing(&mut self, s: &Session, ctx: &egui::Context) {
        let wanted: HashSet<Key> = s
            .doc
            .drawings
            .values()
            .filter_map(|d| d.image)
            .map(|r| (r.key, r.alpha))
            .collect();
        for key in wanted {
            if self.textures.contains_key(&key)
                || self.decoding.contains(&key)
                || self.failed.contains(&key)
            {
                continue;
            }
            let Some(file) = s.doc.images.get(&key.0) else {
                continue;
            };
            self.decoding.insert(key);
            let (tx, ctx, file) = (self.tx.clone(), ctx.clone(), file.clone());
            std::thread::spawn(move || {
                let image = texture_image(&file, key.1);
                let _ = tx.send(Done::Decoded { key, image });
                ctx.request_repaint();
            });
        }
    }
}

/// Decode to premultiplied RGBA, reduced to fit [`MAX_TEXTURE`].
fn texture_image(file: &ImageFile, alpha: Alpha) -> Option<egui::ColorImage> {
    let pix = symphony_render::bitmap::decode(file, alpha)?;
    let (w, h) = (pix.width(), pix.height());
    let rgba = image::RgbaImage::from_raw(w, h, pix.take())?;
    let (tw, th) = texture_size(w, h);
    // Filtering premultiplied pixels keeps clear pixels from darkening
    // the edges.
    let rgba = if (tw, th) == (w, h) {
        rgba
    } else {
        image::imageops::resize(&rgba, tw, th, image::imageops::FilterType::Triangle)
    };
    Some(egui::ColorImage::from_rgba_premultiplied(
        [tw as usize, th as usize],
        rgba.as_raw(),
    ))
}

/// The size of the texture for an image of `w` x `h`: the same, or
/// reduced with its aspect kept so the longer side is [`MAX_TEXTURE`].
fn texture_size(w: u32, h: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= MAX_TEXTURE {
        return (w, h);
    }
    let scale = f64::from(MAX_TEXTURE) / f64::from(long);
    let side = |n: u32| ((f64::from(n) * scale).round() as u32).max(1);
    (side(w), side(h))
}

/// The new layer's name: the first file's name without its extension.
fn layer_name(path: &Path) -> String {
    path.file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Image".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_images_keep_their_size() {
        assert_eq!(texture_size(1920, 1080), (1920, 1080));
        assert_eq!(texture_size(2048, 1), (2048, 1));
    }

    #[test]
    fn large_images_shrink_to_the_longest_side() {
        assert_eq!(texture_size(4096, 2160), (2048, 1080));
        assert_eq!(texture_size(1000, 8000), (256, 2048));
        // A thin image keeps at least one pixel.
        assert_eq!(texture_size(16384, 1), (2048, 1));
    }

    #[test]
    fn a_large_image_becomes_a_reduced_premultiplied_texture() {
        // Half-clear red: premultiplied, the red halves too.
        let img = image::RgbaImage::from_pixel(4096, 16, image::Rgba([255, 0, 0, 128]));
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let (_, file) = symphony_io::media::image_from_bytes(bytes).unwrap();
        let tex = texture_image(&file, Alpha::Straight).unwrap();
        assert_eq!(tex.size, [2048, 8]);
        let p = tex.pixels[8 * 2048 / 2 + 1024];
        assert_eq!((p.r(), p.g(), p.b(), p.a()), (128, 0, 0, 128));
    }
}
