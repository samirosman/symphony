//! Golden-image tests (ENGINEERING E10.3): a render is compared with a
//! reference PNG in `tests/golden/`, channel by channel, within
//! [`TOLERANCE`].
//!
//! To write new references, run the tests with `SYMPHONY_GOLDEN_UPDATE=1`
//! and look at each changed PNG before committing it. A failed check
//! writes the render to `target/golden/<name>.png` for comparison.

use std::path::PathBuf;

use tiny_skia::Pixmap;

/// The largest difference allowed in any channel of any pixel, in 8-bit
/// steps. Room for float rounding that differs between CPUs, not for a
/// visible change.
const TOLERANCE: u8 = 2;

fn dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(sub)
}

/// Fail unless `actual` matches the reference `tests/golden/<name>.png`.
pub fn check(name: &str, actual: &Pixmap) {
    let reference = dir("tests/golden").join(format!("{name}.png"));
    // The reference is stored straight (PNG), so compare the render after
    // the same round trip: identical renders then match exactly.
    let png = actual.encode_png().expect("encode the render");
    let actual = Pixmap::decode_png(&png).expect("decode the render");
    if std::env::var_os("SYMPHONY_GOLDEN_UPDATE").is_some() {
        std::fs::create_dir_all(reference.parent().unwrap()).unwrap();
        std::fs::write(&reference, &png).unwrap();
        return;
    }
    let fail = |why: String| -> ! {
        let out = dir("../../target/golden");
        std::fs::create_dir_all(&out).unwrap();
        let path = out.join(format!("{name}.png"));
        std::fs::write(&path, &png).unwrap();
        panic!(
            "golden {name}: {why}\n  reference {}\n  render    {}\n  \
             set SYMPHONY_GOLDEN_UPDATE=1 to write a new reference",
            reference.display(),
            path.display()
        );
    };
    let Ok(expected) = Pixmap::load_png(&reference) else {
        fail("no reference image".into());
    };
    if (expected.width(), expected.height()) != (actual.width(), actual.height()) {
        fail(format!(
            "size {}x{}, reference {}x{}",
            actual.width(),
            actual.height(),
            expected.width(),
            expected.height()
        ));
    }
    let mut worst = (0u8, 0usize);
    let mut over = 0;
    for (i, (a, e)) in actual.data().iter().zip(expected.data()).enumerate() {
        let d = a.abs_diff(*e);
        if d > TOLERANCE {
            over += 1;
        }
        if d > worst.0 {
            worst = (d, i / 4);
        }
    }
    if over > 0 {
        let (d, px) = worst;
        let (x, y) = (px as u32 % actual.width(), px as u32 / actual.width());
        fail(format!(
            "{over} channels differ by more than {TOLERANCE}, \
             at most {d} at ({x}, {y})"
        ));
    }
}
