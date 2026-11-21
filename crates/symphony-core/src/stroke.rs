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
