//! Brush presets from other drawing apps, read into one neutral
//! description ([`Brush`]) that Symphony turns into its own presets.
//!
//! Artists bring their brushes with them. Drag a brush file onto Symphony
//! and it is imported. Supported today:
//! - Krita: `.kpp` presets and `.bundle` resource bundles
//! - MyPaint: `.myb`
//! - Procreate: `.brush` and `.brushset`
//! - Clip Studio Paint: `.sut`
//! - GIMP: `.gbr` tips
//!
//! Every format is read from files users made or bought themselves, using
//! each app's public documentation or open-source code as the reference
//! (CLEAN-ROOM.md). Readers never panic: bad, hostile or huge files give an
//! error, and the other brushes in the same drop still import (E7.6).

use std::io::Read;
use std::path::Path;

use image::GrayImage;

mod clip;
mod gimp;
mod krita;
mod mypaint;
mod procreate;

/// Which app a brush came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Krita,
    MyPaint,
    Procreate,
    ClipStudio,
    Gimp,
}

impl Source {
    pub fn title(self) -> &'static str {
        match self {
            Source::Krita => "Krita",
            Source::MyPaint => "MyPaint",
            Source::Procreate => "Procreate",
            Source::ClipStudio => "Clip Studio Paint",
            Source::Gimp => "GIMP",
        }
    }
}

/// One brush, in units every app can be mapped to.
#[derive(Clone, Debug)]
pub struct Brush {
    pub name: String,
    pub source: Source,
    /// Diameter at full pressure, in pixels of the source app. Apps scale
    /// sizes differently, so this is a best match, not an exact one.
    pub diameter: f32,
    /// Size at the lightest pressure, as a fraction of `diameter`. 1.0
    /// means pressure doesn't change the size.
    pub min_fraction: f32,
    /// Pressure to size, as (pressure, fraction) points from 0 to 1. Empty
    /// means linear between `min_fraction` and 1.
    pub size_curve: Vec<[f32; 2]>,
    /// 0..=1.
    pub opacity: f32,
    /// Distance between tip stamps, as a fraction of the diameter.
    pub spacing: f32,
    /// Stroke stabilisation, 0..=1.
    pub smoothing: f32,
    /// Is it an eraser preset?
    pub eraser: bool,
    /// Tip shape (white = paint), at most [`MAX_IMAGE`] pixels a side.
    pub tip: Option<GrayImage>,
    /// Paper or grain texture, same limits.
    pub grain: Option<GrayImage>,
}

impl Brush {
    fn new(name: impl Into<String>, source: Source) -> Self {
        Brush {
            name: name.into(),
            source,
            diameter: 10.0,
            min_fraction: 0.0,
            size_curve: Vec::new(),
            opacity: 1.0,
            spacing: 0.1,
            smoothing: 0.0,
            eraser: false,
            tip: None,
            grain: None,
        }
    }

    /// Clamp every number into a sane range (hostile files).
    fn sanitise(mut self) -> Self {
        let fix =
            |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.name = self
            .name
            .chars()
            .filter(|c| !c.is_control())
            .take(80)
            .collect();
        if self.name.trim().is_empty() {
            self.name = "Imported brush".into();
        }
        self.diameter = fix(self.diameter, 0.1, 5000.0, 10.0);
        self.min_fraction = fix(self.min_fraction, 0.0, 1.0, 0.0);
        self.opacity = fix(self.opacity, 0.0, 1.0, 1.0);
        self.spacing = fix(self.spacing, 0.001, 10.0, 0.1);
        self.smoothing = fix(self.smoothing, 0.0, 1.0, 0.0);
        self.size_curve
            .retain(|p| p[0].is_finite() && p[1].is_finite());
        self.size_curve.truncate(64);
        for p in &mut self.size_curve {
            p[0] = p[0].clamp(0.0, 1.0);
            p[1] = p[1].clamp(0.0, 1.0);
        }
        self.size_curve.sort_by(|a, b| a[0].total_cmp(&b[0]));
        if let Some(first) = self.size_curve.first() {
            self.min_fraction = first[1];
        }
        self
    }

    /// Size fraction at `pressure`, from the curve or linear.
    pub fn size_at(&self, pressure: f32) -> f32 {
        let p = pressure.clamp(0.0, 1.0);
        let c = &self.size_curve;
        if c.is_empty() {
            return self.min_fraction + (1.0 - self.min_fraction) * p;
        }
        if p <= c[0][0] {
            return c[0][1];
        }
        for w in c.windows(2) {
            if p <= w[1][0] {
                let t = (p - w[0][0]) / (w[1][0] - w[0][0]).max(1e-6);
                return w[0][1] + (w[1][1] - w[0][1]) * t;
            }
        }
        c[c.len() - 1][1]
    }
}

/// The result of importing one file: brushes, plus notes about anything
/// that couldn't be read, in plain words for the user.
#[derive(Debug, Default)]
pub struct Imported {
    pub brushes: Vec<Brush>,
    pub problems: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a brush format we know.
    Unknown(String),
    /// A brush format we recognise but can't read yet.
    NotYet(&'static str),
    TooLarge,
    Damaged(String),
    Io(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unknown(ext) if ext.is_empty() => {
                write!(f, "This isn't a brush file Symphony knows.")
            }
            Error::Unknown(ext) => write!(f, "Symphony can't import .{ext} files as brushes."),
            Error::NotYet(what) => write!(f, "Symphony can't read {what} yet."),
            Error::TooLarge => write!(f, "This file is too large to be a brush file."),
            Error::Damaged(why) => write!(f, "This brush file looks damaged ({why})."),
            Error::Io(why) => write!(f, "Couldn't read the file: {why}"),
        }
    }
}

/// Biggest brush file we open. Brush sets with many textures can be big.
pub const MAX_FILE: u64 = 512 * 1024 * 1024;
/// Biggest single entry we decompress from an archive.
const MAX_ENTRY: u64 = 64 * 1024 * 1024;
/// Tips and grains are scaled down to fit this many pixels a side.
pub const MAX_IMAGE: u32 = 512;
/// Most brushes taken from one file.
const MAX_BRUSHES: usize = 2000;

/// Extensions that can be dropped onto Symphony as brushes.
pub const EXTENSIONS: &[&str] = &[
    "kpp", "bundle", "myb", "brush", "brushset", "sut", "gbr", "abr", "tbr",
];

/// Is this a file we'd try to import as brushes?
pub fn is_brush_file(path: &Path) -> bool {
    ext(path).is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
}

fn ext(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// Read every brush in the file at `path`.
pub fn import(path: &Path) -> Result<Imported, Error> {
    let size = std::fs::metadata(path)
        .map_err(|e| Error::Io(e.to_string()))?
        .len();
    if size > MAX_FILE {
        return Err(Error::TooLarge);
    }
    let bytes = std::fs::read(path).map_err(|e| Error::Io(e.to_string()))?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported brush");
    import_bytes(&ext(path).unwrap_or_default(), stem, &bytes)
}

/// Read brushes from file contents. `ext` is the lower-case extension and
/// `stem` the file name, used when a format doesn't store a name.
pub fn import_bytes(ext: &str, stem: &str, bytes: &[u8]) -> Result<Imported, Error> {
    let mut out = match ext {
        "kpp" => krita::preset(bytes, &|_| None).map(|b| Imported {
            brushes: vec![b],
            problems: Vec::new(),
        }),
        "bundle" => krita::bundle(bytes),
        "myb" => mypaint::preset(bytes, stem).map(|b| Imported {
            brushes: vec![b],
            problems: Vec::new(),
        }),
        "brush" | "brushset" => procreate::archive(bytes),
        "sut" => clip::preset(bytes, stem).map(|b| Imported {
            brushes: vec![b],
            problems: Vec::new(),
        }),
        "gbr" => gimp::tip(bytes, stem).map(|b| Imported {
            brushes: vec![b],
            problems: Vec::new(),
        }),
        "abr" => Err(Error::NotYet("Photoshop brushes (.abr)")),
        "tbr" | "xml" => Err(Error::NotYet("Harmony brush presets")),
        other => Err(Error::Unknown(other.to_owned())),
    }?;
    out.brushes.truncate(MAX_BRUSHES);
    out.brushes = out.brushes.into_iter().map(Brush::sanitise).collect();
    if out.brushes.is_empty() && out.problems.is_empty() {
        out.problems.push("The file has no brushes in it.".into());
    }
    Ok(out)
}

/// Decode an image and keep its brightness as a mask: white paints.
/// `alpha_is_shape`: the shape is in the alpha channel (Krita, GIMP);
/// otherwise in brightness (Procreate).
fn mask(bytes: &[u8], alpha_is_shape: bool) -> Option<GrayImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let img = if img.width().max(img.height()) > MAX_IMAGE {
        img.thumbnail(MAX_IMAGE, MAX_IMAGE)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let has_alpha = rgba.pixels().any(|p| p[3] < 255);
    let luma =
        |p: &image::Rgba<u8>| (p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19) / 256;
    // Light artwork on transparency (a white stamp) is shaped by alpha
    // alone; dark artwork by darkness times alpha.
    let (mut sum, mut n) = (0u64, 0u64);
    for p in rgba.pixels().filter(|p| p[3] > 128) {
        sum += luma(p) as u64;
        n += 1;
    }
    let light_art = n > 0 && sum / n > 200;
    Some(GrayImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y);
        let (l, a) = (luma(p), p[3] as u32);
        image::Luma([match (alpha_is_shape, has_alpha) {
            // Procreate: white paints.
            (false, _) => l * a / 255,
            (true, true) if light_art => a,
            (true, true) => (255 - l) * a / 255,
            // Krita and GIMP greyscale tips: black paints.
            (true, false) => 255 - l,
        } as u8])
    }))
}

/// Read one archive entry, refusing entries that inflate too far.
fn read_entry<R: Read>(entry: R, size: u64) -> Option<Vec<u8>> {
    if size > MAX_ENTRY {
        return None;
    }
    let mut buf = Vec::with_capacity(size as usize);
    entry.take(MAX_ENTRY + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 <= MAX_ENTRY).then_some(buf)
}

/// Krita sorts presets with prefixes like "b) ": drop them.
fn strip_sort_prefix(name: &str) -> String {
    match name.split_once(") ") {
        Some((p, rest))
            if !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            rest.trim().to_owned()
        }
        _ => name.trim().to_owned(),
    }
}

/// Parse "x,y;x,y;" curves (Krita).
fn parse_curve(text: &str) -> Vec<[f32; 2]> {
    text.split(';')
        .filter_map(|pair| {
            let (x, y) = pair.split_once(',')?;
            Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_never_panics() {
        let samples: [&[u8]; 6] = [
            b"",
            b"\x89PNG\r\n\x1a\n",
            b"PK\x03\x04garbage",
            b"{\"settings\": 5}",
            b"SQLite format 3\0",
            &[0xff; 300],
        ];
        for ext in EXTENSIONS.iter().chain(&["xml", "png", ""]) {
            for s in samples {
                let _ = import_bytes(ext, "x", s);
            }
        }
    }

    #[test]
    fn sanitise_tames_wild_values() {
        let mut b = Brush::new("\u{7}", Source::Krita);
        b.diameter = f32::NAN;
        b.opacity = 9.0;
        b.size_curve = vec![[0.5, 2.0], [f32::INFINITY, 0.0], [0.0, -1.0]];
        let b = b.sanitise();
        assert_eq!(b.name, "Imported brush");
        assert_eq!(b.diameter, 10.0);
        assert_eq!(b.opacity, 1.0);
        assert_eq!(b.size_curve, vec![[0.0, 0.0], [0.5, 1.0]]);
        assert_eq!(b.size_at(0.25), 0.5);
    }

    #[test]
    fn unknown_and_unsupported_formats_say_so() {
        assert!(matches!(
            import_bytes("abr", "x", b""),
            Err(Error::NotYet(_))
        ));
        assert!(matches!(
            import_bytes("tbr", "x", b""),
            Err(Error::NotYet(_))
        ));
        assert!(matches!(
            import_bytes("png", "x", b""),
            Err(Error::Unknown(_))
        ));
    }
}
