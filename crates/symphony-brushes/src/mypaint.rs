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
