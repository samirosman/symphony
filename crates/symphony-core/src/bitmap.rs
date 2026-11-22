//! Imported bitmap images, kept as the original file (Harmony's Keep as
//! Original Bitmap import). A drawing can show one image under its
//! strokes; the image follows the layer's peg like any drawing.
//!
//! Core holds the encoded file bytes and their size, never decoded pixels:
//! decoding needs an image library, which core doesn't depend on (E1.2).
//! `symphony-io` reads and checks files, and copies them into the
//! project's `media/` folder (E7.1); renderers decode and cache them.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::doc::{Scene, Settings};
use crate::history::Edit;

/// Largest image side accepted on import and open (E7.6).
pub const MAX_SIDE: u32 = 16_384;
/// Most pixels in one image: 64 megapixels, 256 MB once decoded.
pub const MAX_PIXELS: u64 = 64 * 1024 * 1024;
/// Largest image file accepted.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
    /// OpenEXR, half or full float, one RGB layer. Its values are linear
    /// light; renderers convert them to sRGB.
    Exr,
}

impl ImageFormat {
    /// File extension used in `media/`.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Exr => "exr",
        }
    }

    /// The alpha encoding files of this format use by convention, for the
    /// import default. OpenEXR stores colour premultiplied by alpha
    /// (openexr.com, Technical Introduction).
    pub fn usual_alpha(self) -> Alpha {
        match self {
            ImageFormat::Png | ImageFormat::Jpeg => Alpha::Straight,
            ImageFormat::Exr => Alpha::PremultipliedBlack,
        }
    }
}

/// How the image is sized in the camera frame on import. The image is
/// centred on the stage either way; its aspect ratio never changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alignment {
    /// The whole image fits inside the stage.
    #[default]
    Fit,
    /// The image covers the stage and bleeds past its longer side, for
    /// panning backgrounds.
    Pan,
    /// One image pixel is one scene pixel.
    ProjectResolution,
}

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

/// One image file used by the scene, stored by content key in
/// `Scene::images`. The bytes are the original file, unchanged.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageFile {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    /// Loaded from `media/` on open; not part of `scene.json`.
    #[serde(skip)]
    pub bytes: Arc<Vec<u8>>,
}

impl ImageFile {
    /// The name of the file in the project's `media/` folder.
    pub fn file_name(&self, key: u64) -> String {
        format!("{key:016x}.{}", self.format.extension())
    }

    /// The size is within the limits (hostile files, E7.6).
    pub fn size_ok(&self) -> bool {
        size_ok(self.width, self.height)
    }
}

pub fn size_ok(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_SIDE
        && height <= MAX_SIDE
        && u64::from(width) * u64::from(height) <= MAX_PIXELS
}

/// A drawing's image: which file, and how it is placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    /// Key in `Scene::images`.
    pub key: u64,
    #[serde(default)]
    pub align: Alignment,
    #[serde(default)]
    pub alpha: Alpha,
}

impl ImageRef {
    /// Where the image sits in drawing space: `[left, top, right, bottom]`
    /// in scene pixels, origin at the stage centre, y down. The rule is
    /// applied against the scene's current resolution.
    pub fn rect(&self, image: &ImageFile, settings: &Settings) -> [f32; 4] {
        let (w, h) = (image.width.max(1) as f32, image.height.max(1) as f32);
        let (sw, sh) = (settings.width.max(1) as f32, settings.height.max(1) as f32);
        let k = match self.align {
            Alignment::Fit => (sw / w).min(sh / h),
            Alignment::Pan => (sw / w).max(sh / h),
            Alignment::ProjectResolution => 1.0,
        };
        let (hw, hh) = (w * k / 2.0, h * k / 2.0);
        [-hw, -hh, hw, hh]
    }
}

/// Content key for image bytes: 64-bit FNV-1a. Stable across Rust
/// versions and platforms, unlike `DefaultHasher`, because it names the
/// file in `media/`.
pub fn content_key(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The edits that add a layer named `name` at `index` showing imported
/// images, one drawing per image, for one undo step. The files must
/// already be in `Scene::images` under `keys`.
///
/// One image is exposed from `start` to the end of the scene. Several
/// images are a sequence: one per frame from `start`, in the given order.
/// (Harmony's docs don't state the frame or the length, so this is our
/// choice.)
pub fn layer_edits(
    scene: &mut Scene,
    name: &str,
    index: usize,
    keys: &[u64],
    start: u32,
    align: Alignment,
    alpha: Alpha,
) -> Vec<Edit> {
    let start = start.max(1);
    let snap = scene.make_layer(name, index);
    let column = snap.column.id;
    let mut edits = vec![Edit::AddLayer(Box::new(snap))];
    let mut cells = Vec::new();
    for (i, &key) in keys.iter().enumerate() {
        let mut d = scene.make_drawing(column);
        d.name = (i + 1).to_string();
        d.image = Some(ImageRef { key, align, alpha });
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

    fn file(w: u32, h: u32) -> ImageFile {
        ImageFile {
            format: ImageFormat::Png,
            width: w,
            height: h,
            bytes: Arc::default(),
        }
    }

    fn rect(align: Alignment, w: u32, h: u32) -> [f32; 4] {
        let r = ImageRef {
            key: 0,
            align,
            alpha: Alpha::Straight,
        };
        r.rect(&file(w, h), &Settings::default())
    }

    #[test]
    fn fit_keeps_the_whole_image_on_stage() {
        // 1920x1080 stage; a portrait 1000x2000 image fits by height.
        assert_eq!(
            rect(Alignment::Fit, 1000, 2000),
            [-270.0, -540.0, 270.0, 540.0]
        );
        // A wide 3840x1080 image fits by width.
        assert_eq!(
            rect(Alignment::Fit, 3840, 1080),
            [-960.0, -270.0, 960.0, 270.0]
        );
    }

    #[test]
    fn pan_covers_the_stage_and_bleeds() {
        // The wide image covers the stage height and bleeds left and right.
        assert_eq!(
            rect(Alignment::Pan, 3840, 1080),
            [-1920.0, -540.0, 1920.0, 540.0]
        );
        assert_eq!(
            rect(Alignment::Pan, 1000, 2000),
            [-960.0, -1920.0, 960.0, 1920.0]
        );
    }

    #[test]
    fn project_resolution_is_one_pixel_per_pixel() {
        assert_eq!(
            rect(Alignment::ProjectResolution, 960, 540),
            [-480.0, -270.0, 480.0, 270.0]
        );
    }

    #[test]
    fn limits_refuse_empty_and_huge_images() {
        assert!(size_ok(1920, 1080));
        assert!(!size_ok(0, 10));
        assert!(!size_ok(MAX_SIDE + 1, 10));
        assert!(!size_ok(MAX_SIDE, MAX_SIDE));
    }

    #[test]
    fn exr_is_stored_as_exr_and_premultiplied() {
        assert_eq!(ImageFormat::Exr.extension(), "exr");
        assert_eq!(ImageFormat::Exr.usual_alpha(), Alpha::PremultipliedBlack);
        assert_eq!(ImageFormat::Png.usual_alpha(), Alpha::Straight);
    }

    #[test]
    fn content_key_is_fnv1a() {
        // Reference values of 64-bit FNV-1a.
        assert_eq!(content_key(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(content_key(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn one_image_is_exposed_to_the_end_and_undoes_in_one_step() {
        use crate::history::History;
        let mut s = Scene::new(Settings::default());
        s.images.insert(9, file(100, 100));
        let edits = layer_edits(&mut s, "BG", 0, &[9], 5, Alignment::Fit, Alpha::Straight);
        let mut h = History::default();
        h.apply(&mut s, "Import image", edits);
        let col = s.layers[0].column;
        assert_eq!(s.xsheet.column(col).unwrap().name, "BG");
        assert_eq!(s.exposed(0, 4), None);
        let d = s.exposed(0, 5).unwrap();
        assert_eq!(s.exposed(0, 48), Some(d));
        assert_eq!(s.exposed(0, 49), None);
        assert_eq!(s.drawings[&d].image.unwrap().key, 9);
        h.undo(&mut s);
        assert!(s.layers.is_empty());
        assert!(s.drawings.is_empty());
    }

    #[test]
    fn several_images_are_a_sequence() {
        let mut s = Scene::new(Settings::default());
        for k in [1, 2, 3] {
            s.images.insert(k, file(10, 10));
        }
        let edits = layer_edits(
            &mut s,
            "a",
            0,
            &[1, 2, 3],
            1,
            Alignment::Pan,
            Alpha::Straight,
        );
        crate::history::History::default().apply(&mut s, "Import images", edits);
        let keys: Vec<u64> = (1..=4)
            .map(|f| {
                s.exposed(0, f)
                    .map(|d| s.drawings[&d].image.unwrap().key)
                    .unwrap_or(0)
            })
            .collect();
        assert_eq!(keys, [1, 2, 3, 0]);
    }
}
