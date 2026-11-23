//! Sampled Photoshop tips, using GIMP's ABR loader as a format reference.
//! https://github.com/GNOME/gimp/blob/master/app/core/gimpbrush-load.c

use image::GrayImage;

use crate::{Brush, Error, Imported, Source, MAX_BRUSHES, MAX_FILE};

fn damaged() -> Error {
    Error::Damaged("invalid Photoshop brush data".into())
}

pub fn pack(bytes: &[u8], stem: &str) -> Result<Imported, Error> {
    let mut input = bytes;
    let version = u16::from_be_bytes(take(&mut input).ok_or_else(damaged)?);
    let variant = u16::from_be_bytes(take(&mut input).ok_or_else(damaged)?);
    let (mut records, limit, prefix) = match version {
        1 | 2 => (input, usize::from(variant), 0),
        6 | 10 if matches!(variant, 1 | 2) => (
            sample_section(input).ok_or_else(damaged)?,
            usize::MAX,
            if variant == 1 { 47 } else { 301 },
        ),
        _ => return Err(Error::NotYet("this Photoshop brush format version")),
    };

    let mut out = Imported {
        brushes: Vec::new(),
        problems: Vec::new(),
    };
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
        if let Some(brush) = sample(record, version, prefix, &name, &mut remaining_pixels) {
            out.brushes.push(brush);
        } else {
            out.problems
                .push(format!("{name}: could not read the sampled tip."));
        }
    }
    Ok(out)
}

fn sample_section(mut input: &[u8]) -> Option<&[u8]> {
    while !input.is_empty() {
        if take::<4>(&mut input)? != *b"8BIM" {
            return None;
        }
        let tag = take::<4>(&mut input)?;
        let len = u32::from_be_bytes(take(&mut input)?) as usize;
        let (data, rest) = input.split_at_checked(len)?;
        if tag == *b"samp" {
            return Some(data);
        }
        input = rest;
    }
    None
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
}
