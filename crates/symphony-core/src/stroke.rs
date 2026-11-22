//! Vector strokes and drawings. Geometry work on them (smoothing,
//! tessellation, hit-testing) lives in `symphony-vector` (E1.1).

use serde::{Deserialize, Serialize};

use crate::id::Id;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    /// 0..=1 after the pressure curve.
    pub pressure: f32,
}

/// How a stroke is shaped (DESIGN/RESEARCH: Harmony's pencil vs brush).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrokeKind {
    /// Filled contour whose width follows pressure.
    Brush,
    /// Centreline with a thickness profile, editable later.
    Pencil,
    /// Invisible line that only bounds paint zones (Harmony's Stroke and
    /// Close Gap tools). Shown only with Show Strokes.
    Invisible,
}

/// Which art layer of a drawing the stroke is on (Harmony's art layers).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ArtLayer {
    Overlay,
    #[default]
    LineArt,
    ColourArt,
    Underlay,
}

impl ArtLayer {
    /// Back to front.
    pub const STACK: [ArtLayer; 4] = [
        ArtLayer::Underlay,
        ArtLayer::ColourArt,
        ArtLayer::LineArt,
        ArtLayer::Overlay,
    ];
}

/// How a line ends (Harmony's pencil Start and End tips).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tip {
    #[default]
    Round,
    Flat,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tips {
    /// The end drawn first.
    pub start: Tip,
    pub end: Tip,
}

/// Harmony's Textured Vector Brush: the stroke stays a vector line, and
/// its look comes from stamping a bitmap tip along it, multiplied by a
/// paper grain. It's redrawn from the line, so editing or transforming the
/// line re-samples the texture instead of stretching pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokeTexture {
    /// Tip image in `Scene::textures` (none = a hard round tip).
    #[serde(default)]
    pub tip: Option<u64>,
    /// Paper grain in `Scene::textures`, tiled in drawing space.
    #[serde(default)]
    pub grain: Option<u64>,
    /// Distance between stamps as a fraction of the tip size.
    pub spacing: f32,
    /// Most opacity the stroke can reach (non-cumulative), 0..=1.
    pub opacity: f32,
    /// Tip rotation in degrees, counter-clockwise.
    #[serde(default)]
    pub angle: f32,
    /// Turn the tip with the stroke's direction (Harmony's Heading).
    #[serde(default)]
    pub follow: bool,
    /// Grain pixels per drawing unit.
    #[serde(default = "one")]
    pub grain_scale: f32,
    /// How much each stamp lays down, 0..=1. Below 1, overlapping stamps
    /// build up (Krita, Harmony and Procreate flow).
    #[serde(default = "one")]
    pub flow: f32,
    /// Edge of the round tip (no tip image): 1 is hard, lower is softer.
    #[serde(default = "one")]
    pub hardness: f32,
    /// Stamp opacity at pen pressures 0, 1/3, 2/3 and 1: how pressure
    /// fades the stroke. All 1 means pressure doesn't change opacity.
    #[serde(default = "full")]
    pub opacity_by_pressure: [f32; 4],
}

fn full() -> [f32; 4] {
    [1.0; 4]
}

impl StrokeTexture {
    /// Stamp opacity for a pen pressure.
    pub fn opacity_at(&self, pressure: f32) -> f32 {
        let x = pressure.clamp(0.0, 1.0) * 3.0;
        let i = (x.floor() as usize).min(2);
        let f = x - i as f32;
        let c = &self.opacity_by_pressure;
        (c[i] * (1.0 - f) + c[i + 1] * f).clamp(0.0, 1.0)
    }
}

fn one() -> f32 {
    1.0
}

/// A greyscale image used by textured strokes: 255 paints fully.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    #[serde(with = "crate::b64")]
    pub pixels: Vec<u8>,
}

impl Texture {
    /// Sizes match and nothing is absurd (hostile files, E7.6).
    pub fn is_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= 4096
            && self.height <= 4096
            && self.pixels.len() == (self.width * self.height) as usize
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub id: Id,
    pub kind: StrokeKind,
    #[serde(default)]
    pub art: ArtLayer,
    /// Palette swatch, never a raw colour (E2.3).
    pub swatch: Id,
    pub min_width: f32,
    pub max_width: f32,
    pub points: Vec<Point>,
    #[serde(default)]
    pub tips: Tips,
    /// Set for textured vector brush strokes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<StrokeTexture>,
    /// Once a brush stroke is erased, its outline (outer contour first,
    /// then holes, even-odd): drawn, hit and painted against instead of
    /// the centreline, so the eraser's shape shows exactly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shape: Vec<Vec<[f32; 2]>>,
}

impl Stroke {
    pub fn width_at(&self, pressure: f32) -> f32 {
        self.min_width + (self.max_width - self.min_width) * pressure.clamp(0.0, 1.0)
    }
}

/// One drawing in a column, like Harmony's numbered drawings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drawing {
    pub id: Id,
    /// The column (element) this drawing belongs to.
    pub column: Id,
    pub name: String,
    pub strokes: Vec<Stroke>,
    /// Painted colour zones, drawn under the strokes of their art layer.
    #[serde(default)]
    pub fills: Vec<Fill>,
}

/// A painted zone (Harmony's Paint tool): the closed area found from the
/// strokes, filled with a palette swatch by ID (E2.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    pub id: Id,
    pub swatch: Id,
    #[serde(default)]
    pub art: ArtLayer,
    /// Closed outlines in drawing space. The first is the outside, any
    /// others are holes (even-odd).
    pub contours: Vec<Vec<[f32; 2]>>,
}

impl Fill {
    /// Is the point inside the painted area (even-odd over all contours)?
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let mut inside = false;
        for c in &self.contours {
            let n = c.len();
            for i in 0..n {
                let (a, b) = (c[i], c[(i + n - 1) % n]);
                if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0]
                {
                    inside = !inside;
                }
            }
        }
        inside
    }
}
