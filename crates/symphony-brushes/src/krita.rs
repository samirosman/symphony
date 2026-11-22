//! Krita: a `.kpp` preset is a PNG (its preview) carrying the preset as
//! XML in a `preset` text chunk. Tips and patterns can be embedded in it as
//! base64, or sit beside it in a `.bundle` (a zip). Reference: Krita's
//! open-source resource code and user manual.

use std::collections::HashMap;
use std::io::Read;

use base64::Engine;

use crate::{
    mask, parse_curve, read_entry, smooth_curve, Brush, Error, Imported, Source, Texturing,
};

/// Look up a resource (tip or pattern) by file name.
pub type Lookup<'a> = dyn Fn(&str) -> Option<Vec<u8>> + 'a;

/// The preset XML from the PNG's text chunks.
fn preset_xml(png: &[u8]) -> Result<String, Error> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(Error::Damaged("not a Krita preset".into()));
    }
    let mut i = 8usize;
    while i + 12 <= png.len() {
        let n = u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]) as usize;
        let kind = &png[i + 4..i + 8];
        let Some(body) = png.get(i + 8..(i + 8).saturating_add(n)) else {
            break;
        };
        let mut parts = body.splitn(2, |b| *b == 0);
        let key = parts.next().unwrap_or_default();
        let rest = parts.next().unwrap_or_default();
        if key == b"preset" {
            match kind {
                b"tEXt" => return Ok(String::from_utf8_lossy(rest).into_owned()),
                b"zTXt" if !rest.is_empty() => {
                    let mut out = String::new();
                    flate2::read::ZlibDecoder::new(&rest[1..])
                        .take(32 * 1024 * 1024)
                        .read_to_string(&mut out)
                        .map_err(|_| Error::Damaged("unreadable preset text".into()))?;
                    return Ok(out);
                }
                // iTXt: flag, method, language\0, translated key\0, text.
                b"iTXt" if rest.len() >= 2 => {
                    let compressed = rest[0] == 1;
                    let mut fields = rest[2..].splitn(3, |b| *b == 0);
                    let (_, _, text) = (fields.next(), fields.next(), fields.next());
                    let text = text.unwrap_or_default();
                    if !compressed {
                        return Ok(String::from_utf8_lossy(text).into_owned());
                    }
                    let mut out = String::new();
                    flate2::read::ZlibDecoder::new(text)
                        .take(64 * 1024 * 1024)
                        .read_to_string(&mut out)
                        .map_err(|_| Error::Damaged("unreadable preset text".into()))?;
                    return Ok(out);
                }
                _ => {}
            }
        }
        i = i.saturating_add(12 + n);
    }
    Err(Error::Damaged("no preset inside".into()))
}

pub fn preset(png: &[u8], lookup: &Lookup) -> Result<Brush, Error> {
    let xml = preset_xml(png)?;
    let doc = parse_with_doctype(&xml).map_err(|_| Error::Damaged("unreadable preset".into()))?;
    let root = doc.root_element();
    let name = root.attribute("name").unwrap_or("Krita brush");
    // Krita names often start with a sort prefix like "b)_".
    let name = crate::strip_sort_prefix(&name.replace('_', " "));
    let mut b = Brush::new(name, Source::Krita);
    let params: HashMap<&str, &str> = root
        .children()
        .filter(|n| n.has_tag_name("param"))
        .filter_map(|n| Some((n.attribute("name")?, n.text().unwrap_or(""))))
        .collect();
    // Resources embedded in the preset itself (newer Krita).
    let mut embedded: HashMap<String, Vec<u8>> = HashMap::new();
    for r in root.descendants().filter(|n| n.has_tag_name("resource")) {
        if let (Some(file), Some(text)) = (r.attribute("filename"), r.text()) {
            let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            if let Ok(data) = base64::engine::general_purpose::STANDARD.decode(clean) {
                embedded.insert(file.to_owned(), data);
            }
        }
    }
    let find = |file: &str| embedded.get(file).cloned().or_else(|| lookup(file));
    let num = |k: &str| params.get(k).and_then(|v| v.trim().parse::<f32>().ok());

    let flag = |k: &str| params.get(k).map(|v| v.trim() == "true");
    // A Color Smudge preset with its Color Rate off only smears paint that's
    // already there (a blending stump). Symphony can't smear yet, and
    // drawing it as ink would look nothing like it.
    let paintop = root.attribute("paintopid").unwrap_or("");
    if paintop == "colorsmudge" && flag("PressureColorRate") != Some(true) {
        return Err(Error::NotYet(
            "blending brushes (they smear paint instead of laying it down)",
        ));
    }
    // A curve option's pressure curve, unless "use curve" is off. Krita's
    // curves are smooth splines through their points.
    let curve_of = |prefix: &str| -> Option<Vec<[f32; 2]>> {
        if flag(&format!("{prefix}UseCurve")) == Some(false) {
            return None;
        }
        pressure_curve(
            params
                .get(format!("{prefix}Sensor").as_str())
                .copied()
                .unwrap_or(""),
        )
        .map(|c| smooth_curve(&c))
    };
    b.opacity = num("OpacityValue").unwrap_or(1.0);
    b.flow = num("FlowValue").unwrap_or(1.0);
    // Opacity and flow follow the pen when their sensor has a pressure
    // curve (alone, or among several sensors).
    if let Some(curve) = curve_of("Opacity") {
        b.opacity_curve = curve;
    }
    if let Some(curve) = curve_of("Flow") {
        b.flow_curve = curve;
    }
    b.eraser = params.get("EraserMode").is_some_and(|v| v.trim() == "true")
        || params
            .get("CompositeOp")
            .is_some_and(|v| v.trim() == "erase");
    if params
        .get("PressureSize")
        .is_some_and(|v| v.trim() == "true")
    {
        let sensor = params.get("SizeSensor").copied().unwrap_or("");
        if let Some(curve) = parse_with_doctype(sensor).ok().and_then(|d| {
            d.descendants()
                .find(|n| n.has_tag_name("curve"))
                .and_then(|n| n.text().map(str::to_owned))
        }) {
            b.size_curve = smooth_curve(&parse_curve(&curve));
        }
        if b.size_curve.is_empty() {
            b.size_curve = vec![[0.0, 0.0], [1.0, 1.0]];
        }
    } else {
        b.min_fraction = 1.0;
    }

    let def = params.get("brush_definition").copied().unwrap_or("");
    if let Ok(d) = parse_with_doctype(def) {
        let brush = d.root_element();
        let attr = |k: &str| brush.attribute(k).and_then(|v| v.parse::<f32>().ok());
        b.spacing = attr("spacing").unwrap_or(0.1);
        // Auto spacing: stamps coefficient x sqrt(size) apart.
        if brush.attribute("useAutoSpacing") == Some("1") {
            b.auto_spacing = attr("autoSpacingCoeff").unwrap_or(1.0);
        }
        // Krita stores the tip angle in radians.
        b.angle = attr("angle").unwrap_or(0.0).to_degrees();
        let scale = attr("scale").unwrap_or(1.0);
        match brush.attribute("type") {
            Some("auto_brush") => {
                if let Some(m) = brush.children().find(|n| n.has_tag_name("MaskGenerator")) {
                    b.diameter = m
                        .attribute("diameter")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(10.0);
                    // Krita's fade: 1 is a hard edge, lower fades in from
                    // the edge. Gaussian tips are soft.
                    let fade = m
                        .attribute("hfade")
                        .and_then(|v| v.parse::<f32>().ok())
                        .unwrap_or(1.0);
                    b.hardness = if m.attribute("id") == Some("gauss") {
                        fade.min(0.3)
                    } else {
                        fade
                    };
                }
            }
            _ => {
                if let Some(file) = brush.attribute("filename") {
                    if let Some(img) = find(file).and_then(|data| mask(&data, true)) {
                        b.diameter = img.width().max(img.height()) as f32 * scale;
                        b.tip = Some(img);
                    } else if let Some((w, h)) = find(file)
                        .and_then(|data| crate::gimp::tip(&data, "").ok())
                        .and_then(|g| g.tip.map(|t| (t.width(), t.height())))
                    {
                        b.diameter = w.max(h) as f32 * scale;
                    }
                }
            }
        }
    }
    if params
        .get("Texture/Pattern/Enabled")
        .is_some_and(|v| v.trim() == "true")
    {
        let file = params
            .get("Texture/Pattern/PatternFileName")
            .copied()
            .unwrap_or("");
        let adjust = PatternAdjust {
            scale: num("Texture/Pattern/Scale").unwrap_or(1.0),
            brightness: num("Texture/Pattern/Brightness").unwrap_or(0.0),
            contrast: num("Texture/Pattern/Contrast").unwrap_or(1.0),
            neutral: num("Texture/Pattern/NeutralPoint").unwrap_or(0.5),
            invert: flag("Texture/Pattern/Invert") == Some(true),
            cutoff_policy: num("Texture/Pattern/CutoffPolicy").unwrap_or(0.0) as i32,
            cutoff: [
                num("Texture/Pattern/CutoffLeft").unwrap_or(0.0),
                num("Texture/Pattern/CutoffRight").unwrap_or(255.0),
            ],
        };
        b.grain = find(file).and_then(|data| pattern(&data, &adjust));
        // Krita 5.0's texturing modes; lightness and gradient fall back to
        // subtract, as Krita does where they can't apply.
        b.texturing = match num("Texture/Pattern/TexturingMode").unwrap_or(0.0) as i32 {
            1..=3 => Texturing::Subtract,
            4 => Texturing::Darken,
            12 | 14 => Texturing::Height,
            13 | 15 => Texturing::LinearHeight,
            _ => Texturing::Multiply,
        };
        b.grain_strength = num("Texture/Strength/Value").unwrap_or(1.0);
        if flag("PressureTexture/Strength/") == Some(true) {
            if let Some(curve) = curve_of("Texture/Strength/") {
                b.grain_strength_curve = curve;
            }
        }
        // Krita samples the pattern at (canvas - offset).
        b.grain_offset = [
            -num("Texture/Pattern/OffsetX").unwrap_or(0.0),
            -num("Texture/Pattern/OffsetY").unwrap_or(0.0),
        ];
    }
    Ok(b)
}

/// Krita's pattern adjustments (KisTextureMaskInfo).
struct PatternAdjust {
    scale: f32,
    brightness: f32,
    contrast: f32,
    neutral: f32,
    invert: bool,
    cutoff_policy: i32,
    cutoff: [f32; 2],
}

/// A Krita pattern as the grain Krita uses, following Krita 5.0's
/// KisTextureMaskInfo::recalculateMask: grey from (11 R + 16 G + 5 B) / 32,
/// transparency counts as white, then brightness, contrast, invert,
/// neutral point and cutoff, after scaling the pattern.
fn pattern(bytes: &[u8], a: &PatternAdjust) -> Option<image::GrayImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let mut img = reader.decode().ok()?.to_rgba8();
    let scale = if a.scale.is_finite() {
        a.scale.clamp(0.05, 10.0)
    } else {
        1.0
    };
    if (scale - 1.0).abs() > 1e-3 {
        let w = ((img.width() as f32 * scale).round() as u32).clamp(1, crate::MAX_IMAGE);
        let h = ((img.height() as f32 * scale).round() as u32).clamp(1, crate::MAX_IMAGE);
        img = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    }
    if img.width().max(img.height()) > crate::MAX_IMAGE {
        let r = crate::MAX_IMAGE as f32 / img.width().max(img.height()) as f32;
        let (w, h) = (
            ((img.width() as f32 * r) as u32).max(1),
            ((img.height() as f32 * r) as u32).max(1),
        );
        img = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    }
    let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
    let (bright, contrast, neutral) = (
        fin(a.brightness, 0.0),
        fin(a.contrast, 1.0),
        fin(a.neutral, 0.5).clamp(0.0, 1.0),
    );
    let (lo, hi) = (
        fin(a.cutoff[0], 0.0) / 255.0,
        fin(a.cutoff[1], 255.0) / 255.0,
    );
    Some(image::GrayImage::from_fn(
        img.width(),
        img.height(),
        |x, y| {
            let p = img.get_pixel(x, y);
            let grey = (u32::from(p[0]) * 11 + u32::from(p[1]) * 16 + u32::from(p[2]) * 5) / 32;
            let alpha = f32::from(p[3]) / 255.0;
            let mut v = (grey as f32 / 255.0) * alpha + (1.0 - alpha);
            v -= bright;
            v = ((v - 0.5) * contrast) + 0.5;
            v = v.clamp(0.0, 1.0);
            if a.invert {
                v = 1.0 - v;
            }
            let mut n = if neutral == 1.0 || (neutral != 0.0 && v <= neutral) {
                v / (2.0 * neutral)
            } else {
                0.5 + (v - neutral) / (2.0 - 2.0 * neutral)
            };
            let outside = n < lo || n > hi;
            if a.cutoff_policy == 1 && outside {
                n = 0.0;
            } else if a.cutoff_policy == 2 && outside {
                n = 1.0;
            }
            image::Luma([(n.clamp(0.0, 1.0) * 255.0).round() as u8])
        },
    ))
}

/// Krita's settings start with `<!DOCTYPE params>`, which the XML reader
/// refuses by default; a bare DOCTYPE with no internal subset is harmless.
fn parse_with_doctype(text: &str) -> Result<roxmltree::Document<'_>, roxmltree::Error> {
    roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
}

/// The pressure curve of a Krita sensor setting, if it has one: either a
/// lone pressure sensor or a pressure child in a sensor list.
fn pressure_curve(sensor: &str) -> Option<Vec<[f32; 2]>> {
    let d = parse_with_doctype(sensor).ok()?;
    let node = d.descendants().find(|n| {
        n.attribute("id") == Some("pressure") && n.children().any(|c| c.has_tag_name("curve"))
    })?;
    let text = node.children().find(|c| c.has_tag_name("curve"))?.text()?;
    let c = parse_curve(text);
    (c.len() >= 2).then_some(c)
}

/// A `.bundle`: a zip with `paintoppresets/`, `brushes/` and `patterns/`.
pub fn bundle(bytes: &[u8]) -> Result<Imported, Error> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| Error::Damaged("not a Krita bundle".into()))?;
    let mut resources: HashMap<String, Vec<u8>> = HashMap::new();
    let mut presets: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total = 0u64;
    for i in 0..zip.len().min(10_000) {
        let Ok(entry) = zip.by_index(i) else { continue };
        let Ok(path) = entry.name().map(|n| n.into_owned()) else {
            continue;
        };
        let size = entry.size();
        total = total.saturating_add(size);
        if total > crate::MAX_FILE {
            return Err(Error::TooLarge);
        }
        let file = path.rsplit('/').next().unwrap_or("").to_owned();
        let wanted = path.starts_with("paintoppresets/")
            || path.starts_with("brushes/")
            || path.starts_with("patterns/");
        if !wanted || file.is_empty() {
            continue;
        }
        let Some(data) = read_entry(entry, size) else {
            continue;
        };
        if path.starts_with("paintoppresets/") && (file.ends_with(".kpp") || file.ends_with(".myb"))
        {
            presets.push((file, data));
        } else {
            resources.insert(file, data);
        }
    }
    let lookup = |f: &str| resources.get(f).cloned();
    let mut out = Imported::default();
    presets.sort_by(|a, b| a.0.cmp(&b.0));
    for (file, data) in presets {
        let read = if file.ends_with(".myb") {
            let stem = file.trim_end_matches(".myb");
            crate::mypaint::preset(&data, stem)
        } else {
            preset(&data, &lookup)
        };
        match read {
            Ok(b) => out.brushes.push(b),
            Err(e) => out.problems.push(format!("{file}: {e}")),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal .kpp: a PNG signature and a tEXt "preset" chunk (the
    /// reader doesn't need image data or CRCs).
    fn kpp(xml: &str) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut body = b"preset\0".to_vec();
        body.extend_from_slice(xml.as_bytes());
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(b"tEXt");
        out.extend_from_slice(&body);
        out.extend_from_slice(&[0, 0, 0, 0]);
        out
    }

    #[test]
    fn reads_flow_opacity_by_pressure_and_soft_tips() {
        let xml = r#"<Preset name="b)_Soft_Pen" paintopid="paintbrush">
<param name="FlowValue" type="string"><![CDATA[0.33]]></param>
<param name="OpacityValue" type="string"><![CDATA[1]]></param>
<param name="OpacitySensor" type="string"><![CDATA[<!DOCTYPE params><params id="sensorslist"><ChildSensor id="fuzzy"><curve>0,0;1,1;</curve></ChildSensor><ChildSensor id="pressure"><curve>0,0.2;1,1;</curve></ChildSensor></params>]]></param>
<param name="PressureSize" type="string"><![CDATA[true]]></param>
<param name="SizeSensor" type="string"><![CDATA[<!DOCTYPE params><params id="pressure"><curve>0,0.5;1,1;</curve></params>]]></param>
<param name="brush_definition" type="string"><![CDATA[<Brush type="auto_brush" spacing="0.1"><MaskGenerator diameter="24" hfade="0.4" id="default" type="circle"/></Brush>]]></param>
</Preset>"#;
        let b = preset(&kpp(xml), &|_| None).unwrap().sanitise();
        assert_eq!(b.name, "Soft Pen");
        assert!((b.flow - 0.33).abs() < 1e-4);
        assert!(
            (b.opacity_at(0.0) - 0.2).abs() < 1e-4,
            "pressure curve, not the fuzzy one"
        );
        assert!((b.hardness - 0.4).abs() < 1e-4);
        assert_eq!(b.diameter, 24.0);
        assert!((b.min_fraction - 0.5).abs() < 1e-4);
    }

    /// A 2x1 PNG pattern: one grey pixel, one fully transparent one.
    fn pattern_png() -> Vec<u8> {
        let img = image::RgbaImage::from_raw(2, 1, vec![128, 128, 128, 255, 0, 0, 0, 0]).unwrap();
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn reads_krita_texture_settings_like_krita() {
        let xml = r#"<Preset name="c)_Pencil" paintopid="paintbrush">
<param name="FlowValue" type="string"><![CDATA[0.5]]></param>
<param name="FlowUseCurve" type="string"><![CDATA[true]]></param>
<param name="FlowSensor" type="string"><![CDATA[<!DOCTYPE params><params id="pressure"><curve>0,0.2;1,1;</curve></params>]]></param>
<param name="Texture/Pattern/Enabled" type="string"><![CDATA[true]]></param>
<param name="Texture/Pattern/PatternFileName" type="string"><![CDATA[grain.png]]></param>
<param name="Texture/Pattern/TexturingMode" type="string"><![CDATA[1]]></param>
<param name="Texture/Pattern/Contrast" type="string"><![CDATA[2]]></param>
<param name="Texture/Pattern/OffsetX" type="string"><![CDATA[10]]></param>
<param name="Texture/Strength/Value" type="string"><![CDATA[0.8]]></param>
<param name="PressureTexture/Strength/" type="string"><![CDATA[true]]></param>
<param name="Texture/Strength/Sensor" type="string"><![CDATA[<!DOCTYPE params><params id="pressure"><curve>0,0.5;1,1;</curve></params>]]></param>
<param name="brush_definition" type="string"><![CDATA[<Brush type="auto_brush" spacing="0.1" useAutoSpacing="1" autoSpacingCoeff="0.3"><MaskGenerator diameter="20" id="default" type="circle"/></Brush>]]></param>
</Preset>"#;
        let png = pattern_png();
        let b = preset(&kpp(xml), &|f| (f == "grain.png").then(|| png.clone()))
            .unwrap()
            .sanitise();
        assert_eq!(b.texturing, Texturing::Subtract);
        assert!((b.auto_spacing - 0.3).abs() < 1e-5);
        assert!((b.flow_at(0.0) - 0.2).abs() < 1e-3 && (b.flow_at(1.0) - 1.0).abs() < 1e-3);
        assert!((b.grain_strength_at(0.0) - 0.4).abs() < 1e-3, "0.8 x curve");
        assert!((b.grain_strength_at(1.0) - 0.8).abs() < 1e-3);
        assert_eq!(b.grain_offset, [-10.0, 0.0]);
        // Krita: grey (128*32/32)/255 = 0.502, contrast 2 around 0.5 ->
        // 0.504; transparency counts as white.
        let g = b.grain.unwrap();
        assert_eq!(g.get_pixel(0, 0)[0], 129);
        assert_eq!(g.get_pixel(1, 0)[0], 255);
    }

    #[test]
    fn blending_stumps_are_skipped_with_a_reason() {
        let xml = r#"<Preset name="Stump" paintopid="colorsmudge">
<param name="PressureColorRate" type="string"><![CDATA[false]]></param>
</Preset>"#;
        let e = preset(&kpp(xml), &|_| None).unwrap_err();
        assert!(e.to_string().contains("blending"), "{e}");
        // A colour smudge brush that does lay paint down still imports.
        let xml = r#"<Preset name="Waxy" paintopid="colorsmudge">
<param name="PressureColorRate" type="string"><![CDATA[true]]></param>
</Preset>"#;
        assert!(preset(&kpp(xml), &|_| None).is_ok());
    }
}
