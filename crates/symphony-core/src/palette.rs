//! Palettes. Strokes point at swatches by ID, so editing a swatch
//! recolours every stroke that uses it (E2.3).

use serde::{Deserialize, Serialize};

use crate::id::Id;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Swatch {
    pub id: Id,
    pub name: String,
    /// sRGB (E10.4).
    pub rgb: [u8; 3],
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Palette {
    pub name: String,
    pub swatches: Vec<Swatch>,
}

impl Palette {
    pub fn get(&self, id: Id) -> Option<&Swatch> {
        self.swatches.iter().find(|s| s.id == id)
    }

    pub fn get_mut(&mut self, id: Id) -> Option<&mut Swatch> {
        self.swatches.iter_mut().find(|s| s.id == id)
    }
}
