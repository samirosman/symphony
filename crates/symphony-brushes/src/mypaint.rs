//! MyPaint `.myb`: JSON with a `settings` map of `base_value` plus input
//! mappings. Reference: libmypaint's documented brush settings.

use crate::{Brush, Error, Source};

pub fn preset(bytes: &[u8], stem: &str) -> Result<Brush, Error> {
    let v: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| Error::Damaged("not a MyPaint brush".into()))?;
    let settings = v
        .get("settings")
        .and_then(|s| s.as_object())
        .ok_or_else(|| Error::Damaged("no brush settings".into()))?;
    let base = |k: &str| {
        settings
            .get(k)
            .and_then(|s| s.get("base_value"))
            .and_then(|b| b.as_f64())
            .map(|f| f as f32)
    };
    // Krita prefixes its MyPaint presets like "c)_Pencil_(mypaint)".
    let name = crate::strip_sort_prefix(&stem.replace("_(mypaint)", "").replace('_', " "));
    let mut b = Brush::new(name, Source::MyPaint);
    // radius_logarithmic: the radius is e^value pixels.
    let radius = base("radius_logarithmic")
        .unwrap_or(2.0)
        .clamp(-2.0, 8.0)
        .exp();
    b.diameter = radius * 2.0;
    b.opacity = base("opaque").unwrap_or(1.0);
    b.hardness = base("hardness").unwrap_or(1.0);
    // opaque_multiply with a pressure input: opacity follows the pen.
    let pressure_points = |key: &str| -> Option<Vec<[f32; 2]>> {
        let pts = settings
            .get(key)?
            .get("inputs")?
            .get("pressure")?
            .as_array()?;
        let c: Vec<[f32; 2]> = pts
            .iter()
            .filter_map(|p| p.as_array())
            .filter_map(|p| Some([p.first()?.as_f64()? as f32, p.get(1)?.as_f64()? as f32]))
            .collect();
        (c.len() >= 2).then_some(c)
    };
    if let Some(c) = pressure_points("opaque_multiply") {
        b.opacity_curve = c;
    }
    b.eraser = base("eraser").unwrap_or(0.0) > 0.5;
    b.smoothing = (base("slow_tracking").unwrap_or(0.0) / 10.0).clamp(0.0, 1.0);
    b.spacing = 1.0 / base("dabs_per_actual_radius").unwrap_or(2.0).max(0.1) / 2.0;
    // Pressure on the radius: an offset (in log radius) at full pressure.
    let pressure_radius = settings
        .get("radius_logarithmic")
        .and_then(|s| s.get("inputs"))
        .and_then(|i| i.get("pressure"))
        .and_then(|p| p.as_array())
        .and_then(|pts| {
            pts.iter()
                .filter_map(|p| p.as_array())
                .filter_map(|p| Some((p.first()?.as_f64()?, p.get(1)?.as_f64()?)))
                .map(|(_, y)| y as f32)
                .reduce(f32::max)
        });
    b.min_fraction = match pressure_radius {
        // Light pressure shrinks the radius by that many log units.
        Some(dy) if dy > 0.0 => (-dy).exp().clamp(0.0, 1.0),
        _ => 1.0,
    };
    Ok(b)
}

/// MyPaint exports ZIP packs with a root order.conf listing brush paths.
/// https://github.com/mypaint/mypaint/blob/master/gui/brushmanager.py
pub fn pack(bytes: &[u8]) -> Result<crate::Imported, Error> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| Error::Damaged("not a readable brush ZIP archive".into()))?;
    let manifest = {
        let entry = zip
            .by_name("order.conf")
            .map_err(|_| Error::Unknown("zip".into()))?;
        let size = entry.size();
        crate::read_entry(entry, size)
            .ok_or_else(|| Error::Damaged("could not read the MyPaint brush list".into()))?
    };
    let manifest = String::from_utf8(manifest)
        .unwrap_or_else(|error| error.into_bytes().into_iter().map(char::from).collect());
    let mut out = crate::Imported {
        brushes: Vec::new(),
        problems: Vec::new(),
    };
    let mut seen = std::collections::HashSet::new();
    let mut remaining = crate::MAX_FILE;
    for line in manifest.split(['\r', '\n']) {
        let name = line.trim().trim_start_matches('\u{feff}');
        if name.is_empty()
            || name.starts_with('#')
            || name.starts_with("Group:")
            || !seen.insert(name)
        {
            continue;
        }
        let path = format!("{name}.myb");
        let stem = name.rsplit('/').next().unwrap_or(name);
        let result = match zip.by_name(&path) {
            Ok(entry) => {
                let size = entry.size();
                let Some(budget) = remaining.checked_sub(size) else {
                    out.problems
                        .push("The MyPaint brush pack exceeds the unpacked size limit.".into());
                    break;
                };
                remaining = budget;
                crate::read_entry(entry, size)
                    .ok_or_else(|| Error::Damaged("could not read this pack entry".into()))
                    .and_then(|bytes| preset(&bytes, stem))
            }
            Err(_) => Err(Error::Damaged("the pack entry is missing".into())),
        };
        match result {
            Ok(brush) => out.brushes.push(brush),
            Err(error) => out.problems.push(format!("{name}: {error}")),
        }
        if seen.len() >= crate::MAX_BRUSHES {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    #[test]
    fn imports_listed_presets_without_duplicates_and_reports_missing_entries() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in [
            (
                "order.conf",
                "Group: Inks\ninks/Soft Pen\nMissing\nGroup: Favourites\ninks/Soft Pen\n",
            ),
            (
                "inks/Soft Pen.myb",
                r#"{"settings":{"hardness":{"base_value":0.25}}}"#,
            ),
        ] {
            writer.start_file(name, options).unwrap();
            writer.write_all(data.as_bytes()).unwrap();
        }
        let archive = writer.finish().unwrap().into_inner();
        let result = crate::import_bytes("zip", "Inks", &archive).unwrap();
        assert_eq!(result.brushes.len(), 1);
        assert_eq!(result.brushes[0].name, "Soft Pen");
        assert_eq!(result.brushes[0].hardness, 0.25);
        assert_eq!(result.problems.len(), 1);
        assert!(result.problems[0].starts_with("Missing:"));
    }
}
