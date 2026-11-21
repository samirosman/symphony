//! Print what Symphony reads from brush files:
//! `cargo run -p symphony-brushes --example inspect -- FILE...`

fn main() {
    for arg in std::env::args().skip(1) {
        let path = std::path::Path::new(&arg);
        let start = std::time::Instant::now();
        match symphony_brushes::import(path) {
            Ok(got) => {
                println!(
                    "{arg}: {} brushes in {:?}",
                    got.brushes.len(),
                    start.elapsed()
                );
                for b in &got.brushes {
                    println!(
                        "  {:<32} {:<10} size {:>7.1} min {:.2} curve {} opacity {:.2} spacing {:.3} smooth {:.2}{}{}{}",
                        b.name,
                        b.source.title(),
                        b.diameter,
                        b.min_fraction,
                        b.size_curve.len(),
                        b.opacity,
                        b.spacing,
                        b.smoothing,
                        b.tip.as_ref().map_or(String::new(), |t| format!(" tip {}x{}", t.width(), t.height())),
                        b.grain.as_ref().map_or(String::new(), |t| format!(" grain {}x{}", t.width(), t.height())),
                        if b.eraser { " eraser" } else { "" },
                    );
                }
                for p in &got.problems {
                    println!("  problem: {p}");
                }
            }
            Err(e) => println!("{arg}: {e}"),
        }
    }
}
