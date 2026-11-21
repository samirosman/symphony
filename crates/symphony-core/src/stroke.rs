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
}
