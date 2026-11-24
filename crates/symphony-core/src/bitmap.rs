//! Imported bitmap images, kept as the original file (Harmony's Keep as
//! Original Bitmap import). A drawing can show one image under its
//! strokes ([`crate::stroke::PlacedImage`]); the image follows the layer's
//! peg like any drawing.
//!
//! Core holds only the file's name and size in `Scene::media`, never
//! pixels: decoding needs an image library, which core doesn't depend on
//! (E1.2). Renderers decode the file and cache the pixels.

use serde::{Deserialize, Serialize};

use crate::doc::Scene;
use crate::history::Edit;
use crate::stroke::PlacedImage;

/// Largest image side accepted on import (E7.6).
pub const MAX_SIDE: u32 = 16_384;
/// Most pixels in one image: 64 megapixels, 256 MB once decoded.
pub const MAX_PIXELS: u64 = 64 * 1024 * 1024;
/// Largest image file accepted.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// How semi-transparent pixels were stored by the program that made the
/// file. Renderers undo the premultiplication before blending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Alpha {
    /// Colour is independent of alpha (PNG, TGA).
    #[default]
    Straight,
    /// Colour fades to black as alpha falls (typical for TIFF, the
    /// OpenEXR convention).
    PremultipliedBlack,
    /// Colour fades to white as alpha falls (typical for a flat PSD).
    PremultipliedWhite,
}

/// The file is OpenEXR (half or full float, one RGB layer), by its
/// extension. Its values are linear light; renderers convert them to sRGB.
pub fn is_exr(file: &str) -> bool {
    std::path::Path::new(file)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exr"))
}

/// The alpha encoding files of this kind use by convention, for the
/// import default. OpenEXR stores colour premultiplied by alpha
/// (openexr.com, Technical Introduction).
pub fn usual_alpha(file: &str) -> Alpha {
    if is_exr(file) {
        Alpha::PremultipliedBlack
    } else {
        Alpha::Straight
    }
}

/// The size is within the limits (hostile files, E7.6).
pub fn size_ok(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_SIDE
        && height <= MAX_SIDE
        && u64::from(width) * u64::from(height) <= MAX_PIXELS
}

/// Where an image of `size` pixels sits when it fits inside a stage of
/// `stage` pixels, centred: `[left, top, width, height]` in scene pixels.
pub fn fit(size: [u32; 2], stage: [u32; 2]) -> [f32; 4] {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    let k = (stage[0].max(1) as f32 / w).min(stage[1].max(1) as f32 / h);
    centred(w * k, h * k)
}

/// Where an image of `size` pixels sits at one image pixel per scene
/// pixel, centred.
pub fn one_to_one(size: [u32; 2]) -> [f32; 4] {
    centred(size[0] as f32, size[1] as f32)
}

fn centred(w: f32, h: f32) -> [f32; 4] {
    [-w / 2.0, -h / 2.0, w, h]
}

/// The edits that add a layer named `name` at `index` showing `images`,
/// one drawing each, for one undo step. The files must already be in
/// `Scene::media`. One image is exposed from `start` to the end of the
/// scene; several are a sequence, one a frame from `start`.
pub fn layer_edits(
    scene: &mut Scene,
    name: &str,
    index: usize,
    images: &[PlacedImage],
    start: u32,
) -> Vec<Edit> {
    let start = start.max(1);
    let snap = scene.make_layer(name, index);
    let column = snap.column.id;
    let mut edits = vec![Edit::AddLayer(Box::new(snap))];
    let mut cells = Vec::new();
    for (i, &img) in images.iter().enumerate() {
        let mut d = scene.make_drawing(column);
        d.name = (i + 1).to_string();
        d.image = Some(img);
        cells.push(Some(d.id));
        edits.push(Edit::AddDrawing { drawing: d });
    }
    if let [one] = cells[..] {
        let end = scene.settings.length.max(start);
        cells = vec![one; (end - start + 1) as usize];
    }
    if !cells.is_empty() {
        edits.push(Edit::SetCells {
            column,
            from: start,
            before: Vec::new(),
            after: cells,
        });
    }
    edits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_refuse_empty_and_huge_images() {
        assert!(size_ok(1920, 1080));
        assert!(!size_ok(0, 10));
        assert!(!size_ok(MAX_SIDE + 1, 10));
        assert!(!size_ok(MAX_SIDE, MAX_SIDE));
    }

    #[test]
    fn fit_keeps_the_whole_image_on_stage() {
        // A portrait 1000x2000 image fits a 1920x1080 stage by height.
        assert_eq!(
            fit([1000, 2000], [1920, 1080]),
            [-270.0, -540.0, 540.0, 1080.0]
        );
        assert_eq!(one_to_one([960, 540]), [-480.0, -270.0, 960.0, 540.0]);
    }

    #[test]
    fn one_image_is_exposed_to_the_end_and_undoes_in_one_step() {
        use crate::doc::Settings;
        use crate::history::History;
        let mut s = Scene::new(Settings::default());
        let img = PlacedImage {
            media: 9,
            rect: fit([100, 100], [1920, 1080]),
            alpha: Alpha::Straight,
        };
        let edits = layer_edits(&mut s, "BG", 0, &[img], 5);
        let mut h = History::default();
        h.apply(&mut s, "Import image", edits);
        assert_eq!(s.exposed(0, 4), None);
        let d = s.exposed(0, 5).unwrap();
        assert_eq!(s.exposed(0, s.settings.length), Some(d));
        assert_eq!(s.drawings[&d].image.unwrap().media, 9);
        h.undo(&mut s);
        assert!(s.layers.is_empty() && s.drawings.is_empty());
    }

    #[test]
    fn exr_is_premultiplied_by_default() {
        assert!(is_exr("00ab.EXR"));
        assert_eq!(usual_alpha("00ab.exr"), Alpha::PremultipliedBlack);
        assert_eq!(usual_alpha("00ab.png"), Alpha::Straight);
    }
}
