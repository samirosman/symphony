//! Clip Studio Paint `.sut`: an SQLite database. The `Node` table holds the
//! name and the `Variant` table the settings. Read from brush files users
//! bought or made themselves. Tip images use Clip Studio's own material
//! format and aren't read yet, so these brushes import with a round tip.

use rusqlite::{Connection, OpenFlags};

use crate::{Brush, Error, Source};

pub fn preset(bytes: &[u8], stem: &str) -> Result<Brush, Error> {
    if !bytes.starts_with(b"SQLite format 3\0") {
        return Err(Error::Damaged("not a Clip Studio brush".into()));
    }
    // SQLite reads from a file; use a private temporary copy.
    let dir = std::env::temp_dir().join(format!(
        "symphony-sut-{}-{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| Error::Io(e.to_string()))?;
    let path = dir.join("brush.sut");
    let result = std::fs::write(&path, bytes)
        .map_err(|e| Error::Io(e.to_string()))
        .and_then(|_| read(&path, stem));
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn read(path: &std::path::Path, stem: &str) -> Result<Brush, Error> {
    let bad = |_| Error::Damaged("unreadable Clip Studio brush".into());
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(bad)?;
    let name: String = db
        .query_row(
            "SELECT NodeName FROM Node WHERE NodeName IS NOT NULL LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| stem.to_owned());
    let name = name.split(" - ").next().unwrap_or(&name).trim().to_owned();
    let mut b = Brush::new(name, Source::ClipStudio);
    let row = db.query_row(
        "SELECT BrushSize, Opacity, BrushInterval, Stickness FROM Variant LIMIT 1",
        [],
        |r| {
            Ok((
                r.get::<_, Option<f64>>(0)?,
                r.get::<_, Option<f64>>(1)?,
                r.get::<_, Option<f64>>(2)?,
                r.get::<_, Option<f64>>(3)?,
            ))
        },
    );
    if let Ok((size, opacity, interval, stick)) = row {
        b.diameter = size.unwrap_or(10.0) as f32;
        // Clip Studio stores opacity as 0..100.
        b.opacity = opacity.map_or(1.0, |o| (o / 100.0) as f32);
        b.spacing = interval.map_or(0.1, |i| (i / 100.0) as f32);
        b.smoothing = stick.map_or(0.0, |s| (s / 100.0) as f32);
    }
    // The pressure graph is a binary blob not read yet: assume pressure
    // sets the size, as most Clip Studio pens do.
    b.min_fraction = 0.0;
    Ok(b)
}
