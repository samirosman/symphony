//! What Symphony reads from sound files, and how long decoding takes:
//! `cargo run --release -p symphony-audio --example sound_info -- <file>...`

use std::path::Path;
use std::time::Instant;

fn main() {
    for arg in std::env::args().skip(1) {
        let t = Instant::now();
        match symphony_audio::import(Path::new(&arg)) {
            Ok((_, file, samples)) => {
                let ms = t.elapsed().as_millis();
                let peak = samples.data.iter().fold(0.0f32, |m, x| m.max(x.abs()));
                println!(
                    "{arg}\t{:?}\t{} Hz\t{} ch\t{} samples\t{:.3} s\tpeak {peak:.3}\t{ms} ms",
                    file.format,
                    file.rate,
                    file.channels,
                    file.samples,
                    file.seconds(),
                );
            }
            Err(e) => println!("{arg}\t{e}"),
        }
    }
}
