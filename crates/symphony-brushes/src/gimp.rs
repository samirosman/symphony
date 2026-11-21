//! GIMP `.gbr` brush tips: a big-endian header ("GIMP" magic, version 2),
//! a name, then raw greyscale or RGBA pixels. Reference: GIMP's published
//! brush file format notes (devel-docs/gbr.txt).

use image::GrayImage;

use crate::{Brush, Error, Source, MAX_IMAGE};

pub fn tip(bytes: &[u8], stem: &str) -> Result<Brush, Error> {
    let bad = || Error::Damaged("not a GIMP brush".into());
    let word = |i: usize| -> Option<u32> {
        Some(u32::from_be_bytes(bytes.get(i..i + 4)?.try_into().ok()?))
    };
    let header = word(0).ok_or_else(bad)? as usize;
    let version = word(4).ok_or_else(bad)?;
    let (w, h, depth) = (
        word(8).ok_or_else(bad)?,
        word(12).ok_or_else(bad)?,
        word(16).ok_or_else(bad)?,
    );
    if version != 2 || bytes.get(20..24) != Some(b"GIMP") || !(28..=4096).contains(&header) {
        return Err(bad());
    }
    if w == 0 || h == 0 || w > 8192 || h > 8192 || !(depth == 1 || depth == 4) {
        return Err(bad());
    }
    let spacing = word(24).ok_or_else(bad)?;
    let name = bytes
        .get(28..header)
        .map(|n| String::from_utf8_lossy(n).trim_end_matches('\0').to_owned())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| stem.to_owned());
    let len = (w as usize) * (h as usize) * depth as usize;
    let data = bytes.get(header..header + len).ok_or_else(bad)?;
    let mut img = GrayImage::from_fn(w, h, |x, y| {
        let i = (y as usize * w as usize + x as usize) * depth as usize;
        // Greyscale tips store coverage directly; RGBA ones in alpha.
        image::Luma([if depth == 1 { data[i] } else { data[i + 3] }])
    });
    if w.max(h) > MAX_IMAGE {
        img = image::imageops::thumbnail(
            &img,
            (w * MAX_IMAGE / w.max(h)).max(1),
            (h * MAX_IMAGE / w.max(h)).max(1),
        );
    }
    let mut b = Brush::new(name, Source::Gimp);
    b.diameter = w.max(h) as f32;
    b.spacing = spacing as f32 / 100.0;
    b.min_fraction = 1.0;
    b.tip = Some(img);
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_small_greyscale_tip() {
        let mut f = Vec::new();
        let name = b"Dot\0";
        for v in [28 + name.len() as u32, 2, 2, 2, 1] {
            f.extend_from_slice(&v.to_be_bytes());
        }
        f.extend_from_slice(b"GIMP");
        f.extend_from_slice(&25u32.to_be_bytes());
        f.extend_from_slice(name);
        f.extend_from_slice(&[0, 255, 255, 0]);
        let b = tip(&f, "x").unwrap();
        assert_eq!(b.name, "Dot");
        assert_eq!(b.spacing, 0.25);
        assert_eq!(b.tip.unwrap().get_pixel(1, 0)[0], 255);
        assert!(tip(&f[..30], "x").is_err());
    }
}
