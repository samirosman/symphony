//! Procreate `.brush` (one brush) and `.brushset` (several): a zip with a
//! folder per brush holding `Brush.archive` (a keyed-archive property
//! list of settings), `Shape.png` and `Grain.png`, white = paint. Read
//! from brush files users bought or made themselves.

use std::collections::BTreeMap;

use plist::Value;

use crate::{mask, read_entry, Brush, Error, Imported, Source};

#[derive(Default)]
struct Parts {
    archive: Option<Vec<u8>>,
    shape: Option<Vec<u8>>,
    grain: Option<Vec<u8>>,
}

pub fn archive(bytes: &[u8]) -> Result<Imported, Error> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| Error::Damaged("not a Procreate brush".into()))?;
    // Group entries by their folder ("" for a single .brush).
    let mut folders: BTreeMap<String, Parts> = BTreeMap::new();
    let mut total = 0u64;
    let mut order: Vec<String> = Vec::new();
    for i in 0..zip.len().min(10_000) {
        let Ok(entry) = zip.by_index(i) else { continue };
        let Ok(path) = entry.name().map(|n| n.into_owned()) else {
            continue;
        };
        let (folder, file) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
        // Each brush keeps its factory settings in a Reset folder; skip it.
        if folder == "Reset" || folder.ends_with("/Reset") {
            continue;
        }
        if file == "brushset.plist" {
            let size = entry.size();
            if let Some(data) = read_entry(entry, size) {
                order = plist::Value::from_reader(std::io::Cursor::new(data))
                    .ok()
                    .and_then(|v| {
                        let list = v.as_dictionary()?.get("brushes")?.as_array()?.clone();
                        Some(
                            list.iter()
                                .filter_map(|b| b.as_string().map(str::to_owned))
                                .collect(),
                        )
                    })
                    .unwrap_or_default();
            }
            continue;
        }
        let slot = match file {
            "Brush.archive" => 0,
            "Shape.png" => 1,
            "Grain.png" => 2,
            _ => continue,
        };
        let size = entry.size();
        total = total.saturating_add(size);
        if total > crate::MAX_FILE {
            return Err(Error::TooLarge);
        }
        let folder = folder.to_owned();
        let Some(data) = read_entry(entry, size) else {
            continue;
        };
        let parts = folders.entry(folder).or_default();
        match slot {
            0 => parts.archive = Some(data),
            1 => parts.shape = Some(data),
            _ => parts.grain = Some(data),
        }
    }
    let mut out = Imported::default();
    // The set's own order, then anything it didn't list.
    let mut folders: Vec<(String, Parts)> = folders.into_iter().collect();
    folders.sort_by_key(|(f, _)| order.iter().position(|o| o == f).unwrap_or(usize::MAX));
    for (folder, parts) in folders {
        let Some(archive) = parts.archive else {
            continue;
        };
        match brush(&archive) {
            Ok(mut b) => {
                b.tip = parts.shape.and_then(|d| mask(&d, false));
                b.grain = parts.grain.and_then(|d| mask(&d, false));
                out.brushes.push(b);
            }
            Err(e) => out.problems.push(format!("{folder}: {e}")),
        }
    }
    Ok(out)
}

/// The settings dictionary from a keyed archive.
fn root(archive: &[u8]) -> Option<(Vec<Value>, plist::Dictionary)> {
    let v = Value::from_reader(std::io::Cursor::new(archive)).ok()?;
    let d = v.as_dictionary()?;
    let objects = d.get("$objects")?.as_array()?.clone();
    let uid = d.get("$top")?.as_dictionary()?.get("root")?.as_uid()?.get();
    let top = objects.get(uid as usize)?.as_dictionary()?.clone();
    Some((objects, top))
}

fn brush(archive: &[u8]) -> Result<Brush, Error> {
    let (objects, top) =
        root(archive).ok_or_else(|| Error::Damaged("unreadable brush settings".into()))?;
    let get = |k: &str| -> Option<Value> {
        match top.get(k)? {
            Value::Uid(u) => objects.get(u.get() as usize).cloned(),
            v => Some(v.clone()),
        }
    };
    let num = |k: &str| {
        get(k).and_then(|v| match v {
            Value::Real(f) => Some(f as f32),
            Value::Integer(i) => i.as_signed().map(|i| i as f32),
            _ => None,
        })
    };
    let name = get("name")
        .and_then(|v| v.as_string().map(str::to_owned))
        .unwrap_or_else(|| "Procreate brush".into());
    // Brush sets often add the author after a bar: "B01. Liner | Studio".
    let name = name.split(" | ").next().unwrap_or(&name).trim().to_owned();
    let mut b = Brush::new(name, Source::Procreate);
    // Procreate's size slider (0..1) scaled by the brush's size limit. The
    // pixel scale is our best match, so artists may want to adjust it.
    let slider = num("paintSize").unwrap_or(0.1).max(0.0);
    let limit = num("maxSize").unwrap_or(1.0).max(0.01);
    b.diameter = 2.0 + slider * limit * 200.0;
    let pressure = num("dynamicsPressureSize").unwrap_or(0.0);
    b.min_fraction = (1.0 - pressure).clamp(0.0, 1.0);
    b.opacity = num("paintOpacity").unwrap_or(1.0);
    // How much pressure fades opacity, 0..1.
    let fade = num("dynamicsPressureOpacity")
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    if fade > 0.0 {
        b.opacity_curve = vec![[0.0, 1.0 - fade], [1.0, 1.0]];
    }
    b.spacing = num("plotSpacing").unwrap_or(0.1);
    b.smoothing = num("plotSmoothing")
        .unwrap_or(0.0)
        .max(num("plotMovingAverageStabilization").unwrap_or(0.0));
    Ok(b)
}
