//! Brush presets from other drawing apps, read into one neutral
//! description ([`Brush`]) that Symphony turns into its own presets.
//!
//! Artists bring their brushes with them. Drag a brush file onto Symphony
//! and it is imported. Supported today:
//! - Krita: `.kpp` presets and `.bundle` resource bundles
//! - MyPaint: `.myb` and brush pack `.zip`
//! - Procreate: `.brush` and `.brushset`
//! - Clip Studio Paint: `.sut`
//! - GIMP: `.gbr` tips and `.gih` image pipes
//! - Photoshop: sampled `.abr` brush tips
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
mod photoshop;
mod procreate;

/// Which app a brush came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Krita,
    MyPaint,
    Procreate,
    ClipStudio,
    Gimp,
    Photoshop,
}

impl Source {
    pub fn title(self) -> &'static str {
        match self {
            Source::Krita => "Krita",
            Source::MyPaint => "MyPaint",
            Source::Procreate => "Procreate",
            Source::ClipStudio => "Clip Studio Paint",
            Source::Gimp => "GIMP",
            Source::Photoshop => "Photoshop",
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
    /// 0..=1: the most the stroke can reach.
    pub opacity: f32,
    /// How much each stamp lays down, 0..=1; below 1, overlaps build up.
    pub flow: f32,
    /// Round-tip edge, 1 hard .. 0 soft (no effect on image tips).
    pub hardness: f32,
    /// Pressure to stamp opacity, as (pressure, opacity) points. Empty
    /// means pressure doesn't change opacity.
    pub opacity_curve: Vec<[f32; 2]>,
    /// Distance between tip stamps, as a fraction of the diameter.
    pub spacing: f32,
    /// Stroke stabilisation, 0..=1.
    pub smoothing: f32,
    /// Tip rotation in degrees, counter-clockwise.
    pub angle: f32,
    /// Rotate the tip with the path tangent.
    pub follow: bool,
    /// Is it an eraser preset?
    pub eraser: bool,
    /// Tip shape (white = paint), at most [`MAX_IMAGE`] pixels a side.
    pub tip: Option<GrayImage>,
    /// Paper or grain texture, same limits. For Krita, already adjusted
    /// (brightness, contrast, invert, neutral point, cutoff, scale).
    pub grain: Option<GrayImage>,
    /// Pressure to flow, as (pressure, factor) points multiplying `flow`.
    /// Empty means pressure doesn't change flow.
    pub flow_curve: Vec<[f32; 2]>,
    /// Krita's auto spacing coefficient (stamps `coefficient * sqrt(size)`
    /// apart), 0 when off.
    pub auto_spacing: f32,
    /// How the grain shapes each stamp.
    pub texturing: Texturing,
    /// Grain strength, 0..=1, and its pressure curve (empty: constant).
    pub grain_strength: f32,
    pub grain_strength_curve: Vec<[f32; 2]>,
    /// Grain offset in grain pixels.
    pub grain_offset: [f32; 2],
}

/// How a grain shapes each stamp (Krita's texturing modes; other apps
/// multiply).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Texturing {
    #[default]
    Multiply,
    Subtract,
    Darken,
    Height,
    LinearHeight,
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
            flow: 1.0,
            hardness: 1.0,
            opacity_curve: Vec::new(),
            spacing: 0.1,
            smoothing: 0.0,
            angle: 0.0,
            follow: false,
            eraser: false,
            tip: None,
            grain: None,
            flow_curve: Vec::new(),
            auto_spacing: 0.0,
            texturing: Texturing::Multiply,
            grain_strength: 1.0,
            grain_strength_curve: Vec::new(),
            grain_offset: [0.0, 0.0],
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
        self.flow = fix(self.flow, 0.01, 1.0, 1.0);
        self.hardness = fix(self.hardness, 0.0, 1.0, 1.0);
        self.opacity_curve
            .retain(|p| p[0].is_finite() && p[1].is_finite());
        self.opacity_curve.truncate(64);
        for p in &mut self.opacity_curve {
            p[0] = p[0].clamp(0.0, 1.0);
            p[1] = p[1].clamp(0.0, 1.0);
        }
        self.opacity_curve.sort_by(|a, b| a[0].total_cmp(&b[0]));
        self.spacing = fix(self.spacing, 0.001, 10.0, 0.1);
        self.auto_spacing = fix(self.auto_spacing, 0.0, 10.0, 0.0);
        self.grain_strength = fix(self.grain_strength, 0.0, 1.0, 1.0);
        self.grain_offset = self.grain_offset.map(|v| fix(v, -1.0e5, 1.0e5, 0.0));
        for c in [&mut self.flow_curve, &mut self.grain_strength_curve] {
            c.retain(|p| p[0].is_finite() && p[1].is_finite());
            c.truncate(64);
            for p in c.iter_mut() {
                p[0] = p[0].clamp(0.0, 1.0);
                p[1] = p[1].clamp(0.0, 1.0);
            }
            c.sort_by(|a, b| a[0].total_cmp(&b[0]));
        }
        self.smoothing = fix(self.smoothing, 0.0, 1.0, 0.0);
        self.angle = fix(self.angle, -360.0, 360.0, 0.0);
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

    /// Flow factor at `pressure` (1 without a curve).
    pub fn flow_at(&self, pressure: f32) -> f32 {
        curve_at(&self.flow_curve, pressure).unwrap_or(1.0)
    }

    /// Grain strength at `pressure`.
    pub fn grain_strength_at(&self, pressure: f32) -> f32 {
        self.grain_strength * curve_at(&self.grain_strength_curve, pressure).unwrap_or(1.0)
    }

    /// Stamp opacity at `pressure`, from the opacity curve (1 without one).
    pub fn opacity_at(&self, pressure: f32) -> f32 {
        curve_at(&self.opacity_curve, pressure).unwrap_or(1.0)
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
    "kpp", "bundle", "myb", "brush", "brushset", "sut", "gbr", "gih", "abr", "tbr", "zip",
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
        "abr" => photoshop::pack(bytes, stem),
        "gih" => gimp::pipe(bytes, stem),
        "zip" => mypaint::pack(bytes),
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

fn sampled_tip(name: String, source: Source, spacing: f32, mut tip: GrayImage) -> Brush {
    let (w, h) = tip.dimensions();
    let diameter = w.max(h);
    if diameter > MAX_IMAGE {
        tip = image::imageops::thumbnail(
            &tip,
            (w * MAX_IMAGE / diameter).max(1),
            (h * MAX_IMAGE / diameter).max(1),
        );
    }
    let mut brush = Brush::new(name, source);
    brush.diameter = diameter as f32;
    brush.spacing = spacing;
    brush.min_fraction = 1.0;
    brush.tip = Some(tip);
    brush
}

/// Decode an image and keep its brightness as a mask: white paints.
/// `alpha_is_shape`: the shape is in the alpha channel (Krita, GIMP);
/// otherwise in brightness (Procreate).
fn mask(bytes: &[u8], alpha_is_shape: bool) -> Option<GrayImage> {
    mask_with_size(bytes, alpha_is_shape).map(|(mask, _)| mask)
}

fn mask_with_size(bytes: &[u8], alpha_is_shape: bool) -> Option<(GrayImage, u32)> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let diameter = img.width().max(img.height());
    let img = if diameter > MAX_IMAGE {
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
    let mask = GrayImage::from_fn(rgba.width(), rgba.height(), |x, y| {
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
    });
    Some((mask, diameter))
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

/// Krita's curves are natural cubic splines through their points
/// (KisCubicCurve): x is held to the first..last point, y to 0..=1. This
/// samples one finely so the rest of the importer can treat it as
/// piecewise linear.
fn smooth_curve(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut p: Vec<[f64; 2]> = points
        .iter()
        .filter(|q| q[0].is_finite() && q[1].is_finite())
        .map(|q| [f64::from(q[0]), f64::from(q[1])])
        .collect();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]));
    p.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-9);
    if p.len() < 3 {
        return p.iter().map(|q| [q[0] as f32, q[1] as f32]).collect();
    }
    let n = p.len() - 1;
    let h: Vec<f64> = (0..n).map(|i| p[i + 1][0] - p[i][0]).collect();
    // Second derivatives, zero at both ends (natural), from the
    // tridiagonal system, as Krita's KisCubicSpline builds it.
    let mut c = vec![0.0f64; n + 1];
    if n > 1 {
        let m = n - 1;
        let mut diag: Vec<f64> = (0..m).map(|i| 2.0 * (h[i] + h[i + 1])).collect();
        let mut rhs: Vec<f64> = (0..m)
            .map(|i| {
                6.0 * ((p[i + 2][1] - p[i + 1][1]) / h[i + 1] - (p[i + 1][1] - p[i][1]) / h[i])
            })
            .collect();
        for i in 1..m {
            let w = h[i] / diag[i - 1];
            diag[i] -= w * h[i];
            rhs[i] -= w * rhs[i - 1];
        }
        let mut sol = vec![0.0f64; m];
        sol[m - 1] = rhs[m - 1] / diag[m - 1];
        for i in (0..m - 1).rev() {
            sol[i] = (rhs[i] - h[i + 1] * sol[i + 1]) / diag[i];
        }
        c[1..=m].copy_from_slice(&sol);
    }
    let value = |x: f64| -> f64 {
        let x = x.clamp(p[0][0], p[n][0]);
        let i = (0..n).find(|&i| x <= p[i + 1][0]).unwrap_or(n - 1);
        let (hi, dx) = (h[i], x - p[i][0]);
        let d = (c[i + 1] - c[i]) / hi;
        let b = -0.5 * c[i] * hi - d * hi * hi / 6.0 + (p[i + 1][1] - p[i][1]) / hi;
        (p[i][1] + b * dx + 0.5 * c[i] * dx * dx + d * dx * dx * dx / 6.0).clamp(0.0, 1.0)
    };
    (0..=32)
        .map(|k| {
            let x = f64::from(k) / 32.0;
            [x as f32, value(x) as f32]
        })
        .collect()
}

/// A piecewise-linear curve's value at `x`, or `None` if it's empty.
fn curve_at(c: &[[f32; 2]], x: f32) -> Option<f32> {
    let x = x.clamp(0.0, 1.0);
    let first = c.first()?;
    if x <= first[0] {
        return Some(first[1]);
    }
    for w in c.windows(2) {
        if x <= w[1][0] {
            let t = (x - w[0][0]) / (w[1][0] - w[0][0]).max(1e-6);
            return Some(w[0][1] + (w[1][1] - w[0][1]) * t);
        }
    }
    Some(c[c.len() - 1][1])
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
    fn krita_curves_are_natural_splines_through_their_points() {
        let pts = [[0.0, 0.1], [0.5, 0.8], [1.0, 0.9]];
        let c = smooth_curve(&pts);
        let at = |x: f32| curve_at(&c, x).unwrap();
        for p in pts {
            assert!((at(p[0]) - p[1]).abs() < 1e-3, "{p:?}");
        }
        // Natural spline through three points: the middle of the first
        // span bulges above the straight line (0.45).
        assert!(at(0.25) > 0.5, "{}", at(0.25));
        // Held to 0..=1 even where the spline overshoots.
        assert!(c.iter().all(|p| (0.0..=1.0).contains(&p[1])));
    }

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
            import_bytes("abr", "x", &[0, 99, 0, 1]),
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
