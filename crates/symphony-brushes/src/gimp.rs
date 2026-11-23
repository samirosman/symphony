//! GIMP `.gbr` brush tips: a big-endian header ("GIMP" magic, version 2),
//! a name, then raw greyscale or RGBA pixels. Reference: GIMP's published
//! brush file format notes (devel-docs/gbr.txt).

use image::GrayImage;

use crate::{Brush, Error, Imported, Source, MAX_BRUSHES};

pub fn tip(bytes: &[u8], stem: &str) -> Result<Brush, Error> {
    read_tip(bytes, stem).map(|(brush, _)| brush)
}

fn read_tip(bytes: &[u8], stem: &str) -> Result<(Brush, usize), Error> {
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
    let img = GrayImage::from_fn(w, h, |x, y| {
        let i = (y as usize * w as usize + x as usize) * depth as usize;
        // Greyscale tips store coverage directly; RGBA ones in alpha.
        image::Luma([if depth == 1 { data[i] } else { data[i + 3] }])
    });
    let brush = crate::sampled_tip(name, Source::Gimp, spacing as f32 / 100.0, img);
    Ok((brush, header + len))
}

/// An image pipe is two text lines followed by concatenated GBR records.
/// https://developer.gimp.org/core/standards/gih/
pub fn pipe(bytes: &[u8], stem: &str) -> Result<Imported, Error> {
    let bad = || Error::Damaged("invalid GIMP image pipe header".into());
    let mut lines = bytes.splitn(3, |&b| b == b'\n');
    let name = lines
        .next()
        .filter(|line| line.len() <= 4096)
        .ok_or_else(bad)?;
    let name = std::str::from_utf8(name)
        .ok()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(stem);
    let count: usize = lines
        .next()
        .filter(|line| line.len() <= 4096)
        .and_then(|line| std::str::from_utf8(line).ok())
        .and_then(|line| line.split_whitespace().next())
        .and_then(|word| word.parse().ok())
        .filter(|&count| count > 0)
        .ok_or_else(bad)?;
    let mut records = lines.next().ok_or_else(bad)?;
    let mut out = Imported {
        brushes: Vec::new(),
        problems: Vec::new(),
    };
    for index in 1..=count.min(MAX_BRUSHES) {
        let label = format!("{name} {index}");
        match read_tip(records, &label) {
            Ok((mut brush, consumed)) => {
                // Pipes often repeat the same GBR label across cells.
                brush.name = label;
                out.brushes.push(brush);
                records = &records[consumed..];
            }
            Err(error) => {
                out.problems.push(format!("{label}: {error}"));
                break;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dot() -> Vec<u8> {
        let mut f = Vec::new();
        let name = b"Dot\0";
        for v in [28 + name.len() as u32, 2, 2, 2, 1] {
            f.extend_from_slice(&v.to_be_bytes());
        }
        f.extend_from_slice(b"GIMP");
        f.extend_from_slice(&25u32.to_be_bytes());
        f.extend_from_slice(name);
        f.extend_from_slice(&[0, 255, 255, 0]);
        f
    }

    #[test]
    fn reads_a_small_greyscale_tip() {
        let f = dot();
        let b = tip(&f, "x").unwrap();
        assert_eq!(b.name, "Dot");
        assert_eq!(b.spacing, 0.25);
        assert_eq!(b.tip.unwrap().get_pixel(1, 0)[0], 255);
        assert!(tip(&f[..30], "x").is_err());
    }
    #[test]
    fn imports_each_pipe_cell_as_a_separate_tip() {
        let mut file = b"Brush Pipe\n3 ncells:3 dim:1 rank0:3\n".to_vec();
        file.extend(dot());
        file.extend(dot());
        let pack = crate::import_bytes("gih", "x", &file).unwrap();
        assert_eq!(pack.brushes.len(), 2);
        assert_eq!(pack.brushes[1].name, "Brush Pipe 2");
        assert_eq!(pack.problems.len(), 1);
    }
}
