//! Krita: a `.kpp` preset is a PNG (its preview) carrying the preset as
//! XML in a `preset` text chunk. Tips and patterns can be embedded in it as
//! base64, or sit beside it in a `.bundle` (a zip). Reference: Krita's
//! open-source resource code and user manual.

use std::collections::HashMap;
use std::io::Read;

use base64::Engine;

use crate::{mask, parse_curve, read_entry, Brush, Error, Imported, Source};

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
    let doc =
        roxmltree::Document::parse(&xml).map_err(|_| Error::Damaged("unreadable preset".into()))?;
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

    b.opacity = num("OpacityValue").unwrap_or(1.0);
    b.eraser = params.get("EraserMode").is_some_and(|v| v.trim() == "true")
        || params
            .get("CompositeOp")
            .is_some_and(|v| v.trim() == "erase");
    if params
        .get("PressureSize")
        .is_some_and(|v| v.trim() == "true")
    {
        let sensor = params.get("SizeSensor").copied().unwrap_or("");
        if let Some(curve) = roxmltree::Document::parse(sensor).ok().and_then(|d| {
            d.descendants()
                .find(|n| n.has_tag_name("curve"))
                .and_then(|n| n.text().map(str::to_owned))
        }) {
            b.size_curve = parse_curve(&curve);
        }
        if b.size_curve.is_empty() {
            b.size_curve = vec![[0.0, 0.0], [1.0, 1.0]];
        }
    } else {
        b.min_fraction = 1.0;
    }

    let def = params.get("brush_definition").copied().unwrap_or("");
    if let Ok(d) = roxmltree::Document::parse(def) {
        let brush = d.root_element();
        let attr = |k: &str| brush.attribute(k).and_then(|v| v.parse::<f32>().ok());
        b.spacing = attr("spacing").unwrap_or(0.1);
        let scale = attr("scale").unwrap_or(1.0);
        match brush.attribute("type") {
            Some("auto_brush") => {
                if let Some(m) = brush.children().find(|n| n.has_tag_name("MaskGenerator")) {
                    b.diameter = m
                        .attribute("diameter")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(10.0);
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
        b.grain = find(file).and_then(|data| mask(&data, false));
    }
    Ok(b)
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
