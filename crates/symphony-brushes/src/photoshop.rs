//! Sampled Photoshop tips, using GIMP's ABR loader as a format reference.
//! https://github.com/GNOME/gimp/blob/master/app/core/gimpbrush-load.c

use image::GrayImage;
use std::collections::HashMap;

mod descriptor;

use crate::{Brush, Error, Imported, Source, MAX_BRUSHES, MAX_FILE};

fn damaged() -> Error {
    Error::Damaged("invalid Photoshop brush data".into())
}

pub fn pack(bytes: &[u8], stem: &str) -> Result<Imported, Error> {
    let mut input = bytes;
    let version = u16::from_be_bytes(take(&mut input).ok_or_else(damaged)?);
    let variant = u16::from_be_bytes(take(&mut input).ok_or_else(damaged)?);
    let mut metadata = Vec::new();
    let (mut records, limit, prefix) = match version {
        1 | 2 => (input, usize::from(variant), 0),
        6 | 10 if matches!(variant, 1 | 2) => {
            let (samples, desc) = sections(input);
            metadata = desc.and_then(descriptor::presets).unwrap_or_default();
            let samples = samples.ok_or_else(damaged)?;
            (samples, usize::MAX, if variant == 1 { 47 } else { 301 })
        }
        _ => return Err(Error::NotYet("this Photoshop brush format version")),
    };

    let mut out = Imported {
        brushes: Vec::new(),
        problems: Vec::new(),
    };
    let settings: HashMap<_, _> = metadata
        .iter()
        .filter_map(|preset| Some((preset.nested(b"Brsh")?.text(b"sampledData")?, preset)))
        .collect();
    let mut remaining_pixels = MAX_FILE as usize;
    for index in 0..limit.min(MAX_BRUSHES) {
        if records.is_empty() && prefix != 0 {
            break;
        }
        let Some((kind, record)) = next_record(&mut records, prefix != 0) else {
            out.problems
                .push("The Photoshop brush pack ends partway through an entry.".into());
            break;
        };
        let name = format!("{stem} {}", index + 1);
        if kind != 2 {
            out.problems
                .push(format!("{name}: only sampled brush tips can be imported."));
            continue;
        }
        if let Some(mut brush) = sample(record, version, prefix, &name, &mut remaining_pixels) {
            if prefix != 0 {
                if let Some(preset) = sample_id(record).and_then(|id| settings.get(id)) {
                    translate(&mut brush, preset);
                }
            }
            out.brushes.push(brush);
        } else {
            out.problems
                .push(format!("{name}: could not read the sampled tip."));
        }
    }
    Ok(out)
}

fn sections(mut input: &[u8]) -> (Option<&[u8]>, Option<&[u8]>) {
    let (mut samples, mut descriptor) = (None, None);
    while let Some((header, tail)) = input.split_at_checked(12) {
        if &header[..4] != b"8BIM" {
            break;
        }
        let len = u32::from_be_bytes(header[8..].try_into().unwrap()) as usize;
        let Some((data, rest)) = tail.split_at_checked(len) else {
            break;
        };
        match &header[4..8] {
            b"samp" => samples = Some(data),
            b"desc" => descriptor = Some(data),
            _ => {}
        }
        input = rest;
    }
    (samples, descriptor)
}

fn sample_id(record: &[u8]) -> Option<&str> {
    let (&len, id) = record.split_first()?;
    let len = usize::from(len);
    if len > 36 {
        return None;
    }
    std::str::from_utf8(id.get(..len)?).ok()
}

fn translate(brush: &mut Brush, preset: &descriptor::Object<'_>) {
    let Some(tip) = preset.nested(b"Brsh") else {
        return;
    };
    if let Some(name) = preset.text(b"Nm  ").or_else(|| tip.text(b"Nm  ")) {
        brush.name = name.to_owned();
    }
    brush.diameter = tip.number(b"Dmtr").unwrap_or(brush.diameter);
    brush.angle = tip.number(b"Angl").unwrap_or_default();
    if tip.flag(b"Intr") {
        brush.spacing = tip.number(b"Spcn").map_or(brush.spacing, |v| v / 100.0);
    } else if tip.number(b"Spcn").is_some() {
        brush.spacing = 0.01;
    }
    if let Some(shape) = &mut brush.tip {
        let roundness = tip.number(b"Rndn").unwrap_or(100.0).clamp(1.0, 100.0) / 100.0;
        if roundness < 1.0 {
            let h = (shape.height() as f32 * roundness).round().max(1.0) as u32;
            *shape = image::imageops::resize(
                shape,
                shape.width(),
                h,
                image::imageops::FilterType::Triangle,
            );
        }
        if tip.flag(b"flipX") {
            image::imageops::flip_horizontal_in_place(shape);
        }
        if tip.flag(b"flipY") {
            image::imageops::flip_vertical_in_place(shape);
        }
    }
    let options = preset.nested(b"toolOptions");
    if let Some(options) = options {
        brush.opacity = options.number(b"Opct").unwrap_or(100.0) / 100.0;
        brush.flow = options.number(b"flow").unwrap_or(100.0) / 100.0;
    }

    // Photoshop control 2 is pen pressure, and 7 is stroke direction.
    let control = |key| preset.nested(key).and_then(|option| option.number(b"bVTy"));
    if preset.flag(b"useTipDynamics") {
        if control(b"szVr") == Some(2.0) {
            brush.min_fraction = preset.number(b"minimumDiameter").unwrap_or_default() / 100.0;
        }
        brush.follow = control(b"angleDynamics") == Some(7.0);
    }
    if preset.flag(b"usePaintDynamics") {
        let pressure_curve = |key| {
            let option = preset.nested(key)?;
            (option.number(b"bVTy") == Some(2.0)).then(|| {
                vec![
                    [0.0, option.number(b"Mnm ").unwrap_or_default() / 100.0],
                    [1.0, 1.0],
                ]
            })
        };
        brush.opacity_curve = pressure_curve(b"opVr").unwrap_or_default();
        brush.flow_curve = pressure_curve(b"prVr").unwrap_or_default();
    }
}

fn next_record<'a>(input: &mut &'a [u8], modern: bool) -> Option<(u16, &'a [u8])> {
    let kind = if modern {
        2
    } else {
        u16::from_be_bytes(take(input)?)
    };
    let len = u32::from_be_bytes(take(input)?) as usize;
    let padded = if modern {
        len.checked_add(3)? & !3
    } else {
        len
    };
    let (data, rest) = input.split_at_checked(padded)?;
    *input = rest;
    Some((kind, &data[..len]))
}

fn sample(
    mut input: &[u8],
    version: u16,
    prefix: usize,
    fallback: &str,
    remaining_pixels: &mut usize,
) -> Option<Brush> {
    let mut name = fallback.to_owned();
    let spacing = if prefix != 0 {
        input = input.get(prefix..)?;
        0.25
    } else {
        take::<4>(&mut input)?;
        let spacing = f32::from(u16::from_be_bytes(take(&mut input)?)) / 100.0;
        if version == 2 {
            let units = u32::from_be_bytes(take(&mut input)?) as usize;
            let (text, rest) = input.split_at_checked(units.checked_mul(2)?)?;
            input = rest;
            if units <= 1024 {
                let title = String::from_utf16_lossy(
                    &text
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .copied()
                        .map(u16::from_be_bytes)
                        .collect::<Vec<_>>(),
                );
                let title = title.trim_matches('\0').trim();
                if !title.is_empty() {
                    name = title.to_owned();
                }
            }
        }
        input = input.get(9..)?;
        spacing
    };
    let top = i32::from_be_bytes(take(&mut input)?);
    let left = i32::from_be_bytes(take(&mut input)?);
    let bottom = i32::from_be_bytes(take(&mut input)?);
    let right = i32::from_be_bytes(take(&mut input)?);
    let width = right.checked_sub(left)?;
    let height = bottom.checked_sub(top)?;
    if !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    let pixels = w * h;
    *remaining_pixels = remaining_pixels.checked_sub(pixels)?;
    let depth = u16::from_be_bytes(take(&mut input)?);
    let [encoding] = take(&mut input)?;
    let data = match (encoding, depth) {
        (0, 8) => input.get(..pixels)?.to_vec(),
        (0, 16) => input
            .get(..pixels * 2)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| p[1])
            .collect(),
        (1, 8) => decode_rows(input, w, h)?,
        _ => return None,
    };
    let tip = GrayImage::from_raw(w as u32, h as u32, data)?;
    Some(crate::sampled_tip(name, Source::Photoshop, spacing, tip))
}

fn decode_rows(input: &[u8], width: usize, height: usize) -> Option<Vec<u8>> {
    let (lengths, mut rows) = input.split_at_checked(height * 2)?;
    let mut pixels = Vec::new();
    for &len in lengths.as_chunks::<2>().0 {
        let (mut row, rest) = rows.split_at_checked(u16::from_be_bytes(len) as usize)?;
        rows = rest;
        let start = pixels.len();
        while let Some((&opcode, tail)) = row.split_first() {
            row = tail;
            let count = match opcode {
                0..=127 => usize::from(opcode) + 1,
                128 => continue,
                _ => 257 - usize::from(opcode),
            };
            if pixels.len() - start + count > width {
                return None;
            }
            if opcode < 128 {
                let (literal, rest) = row.split_at_checked(count)?;
                pixels.extend_from_slice(literal);
                row = rest;
            } else {
                let (&value, rest) = row.split_first()?;
                pixels.resize(pixels.len() + count, value);
                row = rest;
            }
        }
        if pixels.len() - start != width {
            return None;
        }
    }
    Some(pixels)
}

fn take<const N: usize>(input: &mut &[u8]) -> Option<[u8; N]> {
    let (value, rest) = input.split_at_checked(N)?;
    *input = rest;
    value.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Vec<u8> {
        [0i32, 0, 2, 2]
            .into_iter()
            .flat_map(i32::to_be_bytes)
            .collect()
    }

    #[test]
    fn reads_legacy_named_brush_after_unsupported_computed_brush() {
        let mut record = vec![0; 4];
        record.extend(30u16.to_be_bytes());
        record.extend(3u32.to_be_bytes());
        record.extend("Ink".encode_utf16().flat_map(u16::to_be_bytes));
        record.extend([0; 9]);
        record.extend(bounds());
        record.extend([0, 8, 0, 0, 99, 127, 255]);
        let mut file = vec![0, 2, 0, 2, 0, 1, 0, 0, 0, 0, 0, 2];
        file.extend((record.len() as u32).to_be_bytes());
        file.extend(record);
        let imported = crate::import_bytes("abr", "Pack", &file).unwrap();
        assert_eq!(imported.problems.len(), 1);
        assert_eq!(imported.brushes.len(), 1);
        let brush = &imported.brushes[0];
        assert_eq!(brush.name, "Ink");
        assert_eq!(brush.spacing, 0.3);
        assert_eq!(brush.tip.as_ref().unwrap().as_raw(), &[0, 99, 127, 255]);
    }

    #[test]
    fn reads_padded_modern_samples_and_rejects_overlong_rle_rows() {
        let mut record = vec![0; 301];
        record.extend(bounds());
        record.extend([0, 8, 1, 0, 3, 0, 2, 1, 0, 99, 255, 127]);
        let mut section = (record.len() as u32).to_be_bytes().to_vec();
        section.extend(record);
        section.resize(section.len().next_multiple_of(4), 0);
        for version in [6, 10] {
            let mut file = vec![0, version, 0, 2];
            file.extend(b"8BIMsamp");
            file.extend((section.len() as u32).to_be_bytes());
            file.extend(&section);
            let imported = crate::import_bytes("abr", "Pack", &file).unwrap();
            assert!(imported.problems.is_empty());
            let brush = &imported.brushes[0];
            assert_eq!(brush.name, "Pack 1");
            assert_eq!(brush.tip.as_ref().unwrap().as_raw(), &[0, 99, 127, 127]);
        }
        assert!(decode_rows(&[0, 2, 254, 7], 2, 1).is_none());
    }

    fn id(data: &mut Vec<u8>, id: &[u8]) {
        let size = if id.len() == 4 { 0 } else { id.len() as u32 };
        data.extend(size.to_be_bytes());
        data.extend(id);
    }

    fn object(data: &mut Vec<u8>, class: &[u8], fields: u32) {
        data.extend(0u32.to_be_bytes());
        id(data, class);
        data.extend(fields.to_be_bytes());
    }

    fn text(data: &mut Vec<u8>, key: &[u8], value: &str) {
        id(data, key);
        data.extend(b"TEXT");
        data.extend((value.encode_utf16().count() as u32).to_be_bytes());
        data.extend(value.encode_utf16().flat_map(u16::to_be_bytes));
    }

    fn number(data: &mut Vec<u8>, key: &[u8], value: f64) {
        id(data, key);
        data.extend(b"doub");
        data.extend(value.to_be_bytes());
    }

    #[test]
    fn matches_sample_ids_and_applies_descriptor_settings() {
        let mut desc = vec![0, 0, 0, 16];
        object(&mut desc, b"null", 1);
        id(&mut desc, b"Brsh");
        desc.extend(b"VlLs");
        desc.extend(1u32.to_be_bytes());
        desc.extend(b"Objc");
        object(&mut desc, b"brushPreset", 2);
        text(&mut desc, b"Nm  ", "Dry ink");
        id(&mut desc, b"Brsh");
        desc.extend(b"Objc");
        object(&mut desc, b"sampledBrush", 5);
        text(&mut desc, b"sampledData", "ink");
        number(&mut desc, b"Dmtr", 46.0);
        number(&mut desc, b"Spcn", 17.0);
        number(&mut desc, b"Angl", 30.0);
        id(&mut desc, b"Intr");
        desc.extend(b"bool");
        desc.push(1);
        let mut record = vec![0; 47];
        record[..4].copy_from_slice(b"\x03ink");
        record.extend(bounds());
        record.extend([0, 8, 0, 0, 99, 127, 255]);
        let mut samples = (record.len() as u32).to_be_bytes().to_vec();
        samples.extend(record);
        samples.resize(samples.len().next_multiple_of(4), 0);
        let mut file = vec![0, 6, 0, 1];
        for (tag, data) in [(b"8BIMdesc", desc), (b"8BIMsamp", samples)] {
            file.extend(tag);
            file.extend((data.len() as u32).to_be_bytes());
            file.extend(data);
        }
        let imported = crate::import_bytes("abr", "Pack", &file).unwrap();
        let brush = &imported.brushes[0];
        assert_eq!(brush.name, "Dry ink");
        assert_eq!(brush.diameter, 46.0);
        assert_eq!(brush.spacing, 0.17);
        assert_eq!(brush.angle, 30.0);
    }
}
