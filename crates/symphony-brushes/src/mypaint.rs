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
    let pressure_points = |key: &str| -> Option<Vec<[f32; 2]>> {
        let points = settings
            .get(key)?
            .get("inputs")?
            .get("pressure")?
            .as_array()?;
        let mut curve: Vec<_> = points
            .iter()
            .filter_map(|point| {
                let [x, y, ..] = point.as_array()?.as_slice() else {
                    return None;
                };
                let (x, y) = (x.as_f64()? as f32, y.as_f64()? as f32);
                (x.is_finite() && y.is_finite()).then_some([x, y])
            })
            .take(256)
            .collect();
        curve.sort_by(|a, b| a[0].total_cmp(&b[0]));
        (!curve.is_empty()).then_some(curve)
    };
    let radius_curve = pressure_points("radius_logarithmic").unwrap_or_default();
    let opacity_curve = pressure_points("opaque").unwrap_or_default();
    let multiplier_curve = pressure_points("opaque_multiply").unwrap_or_default();
    let offset = |curve: &[[f32; 2]], pressure| crate::curve_at(curve, pressure).unwrap_or(0.0);
    let nominal_radius = base("radius_logarithmic").unwrap_or(2.0);
    let nominal_opacity = base("opaque").unwrap_or(1.0);
    let nominal_multiplier = base("opaque_multiply").unwrap_or(if multiplier_curve.is_empty() {
        1.0
    } else {
        0.0
    });

    // MyPaint pressure inputs add offsets. Radius is logarithmic, and
    // opacity combines two independent settings multiplicatively.
    let mut radius = Vec::new();
    let mut opacity = Vec::new();
    for i in 0..=32 {
        let p = i as f32 / 32.0;
        radius.push([
            p,
            (nominal_radius + offset(&radius_curve, p))
                .clamp(-2.0, 8.0)
                .exp(),
        ]);
        let alpha = (nominal_opacity + offset(&opacity_curve, p)).max(0.0);
        let multiplier = nominal_multiplier + offset(&multiplier_curve, p);
        opacity.push([p, (alpha * multiplier).clamp(0.0, 1.0)]);
    }
    let max_radius = radius.iter().map(|p| p[1]).fold(0.0, f32::max);
    b.diameter = max_radius * 2.0;
    b.min_fraction = 1.0;
    if !radius_curve.is_empty() && max_radius > 0.0 {
        b.size_curve = radius
            .into_iter()
            .map(|[p, r]| [p, r / max_radius])
            .collect();
    }
    b.opacity = opacity.iter().map(|p| p[1]).fold(0.0, f32::max);
    if b.opacity > 0.0 && (!opacity_curve.is_empty() || !multiplier_curve.is_empty()) {
        b.opacity_curve = opacity
            .into_iter()
            .map(|[p, o]| [p, o / b.opacity])
            .collect();
    }
    b.hardness = base("hardness").unwrap_or(1.0);
    b.eraser = base("eraser").unwrap_or(0.0) > 0.5;
    b.smoothing = (base("slow_tracking").unwrap_or(0.0) / 10.0).clamp(0.0, 1.0);

    // Both density terms contribute to travel-based dabs. Convert to
    // diameter-relative spacing at the preset's nominal radius.
    let actual = base("dabs_per_actual_radius").unwrap_or(2.0).max(0.0);
    let basic = base("dabs_per_basic_radius").unwrap_or(0.0).max(0.0);
    b.spacing = 0.5 / (actual + basic).max(0.05);
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
    fn maps_additive_pressure_offsets_before_converting_radius_and_opacity() {
        let data = br#"{"settings":{
            "radius_logarithmic":{"base_value":1,"inputs":{"pressure":[[0,-1],[1,1]]}},
            "opaque":{"base_value":0.5,"inputs":{"pressure":[[0,0],[1,0.5]]}},
            "opaque_multiply":{"base_value":0.25,"inputs":{"pressure":[[0,0],[1,0.75]]}},
            "dabs_per_basic_radius":{"base_value":1},
            "dabs_per_actual_radius":{"base_value":3}
        }}"#;
        let brush = super::preset(data, "Ink").unwrap().sanitise();
        assert!((brush.diameter - 2.0 * 2.0f32.exp()).abs() < 1e-5);
        assert!((brush.diameter * brush.size_at(0.5) - 2.0 * 1.0f32.exp()).abs() < 1e-5);
        assert_eq!(brush.opacity * brush.opacity_at(0.0), 0.125);
        assert_eq!(brush.opacity * brush.opacity_at(0.5), 0.46875);
        assert_eq!(brush.spacing, 0.125);
    }

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
