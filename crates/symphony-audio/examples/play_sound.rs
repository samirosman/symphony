//! Play a sound file through Symphony's mixer, to hear it and to check
//! the sync clock: `cargo run --release -p symphony-audio --example
//! play_sound -- <file> [seconds]`. `--silent` keeps the output muted
//! and only prints the clock.

use std::path::Path;
use std::time::{Duration, Instant};

use symphony_audio::{prepare, Clip, Command, Player};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let silent = args.iter().any(|a| a == "--silent");
    let rest: Vec<&String> = args.iter().filter(|a| *a != "--silent").collect();
    let mut player = Player::open().unwrap_or_else(|e| panic!("{e}"));
    println!(
        "output: {} Hz, {} channels",
        player.rate(),
        player.channels()
    );
    let mut length = 1.0;
    if let Some(path) = rest.first() {
        let (_, file, samples) = symphony_audio::import(Path::new(path.as_str())).unwrap();
        let t = Instant::now();
        let ready = prepare(&samples, player.rate(), player.channels()).unwrap();
        println!(
            "{:?} {} Hz {} ch, {:.2} s, prepared in {} ms",
            file.format,
            file.rate,
            file.channels,
            file.seconds(),
            t.elapsed().as_millis()
        );
        length = file.seconds();
        player.send(Command::SetClips(vec![Clip::new(
            ready,
            0.0,
            true,
            player.rate(),
        )]));
    }
    if let Some(s) = rest.get(1) {
        length = s.parse().unwrap();
    }
    player.send(Command::SetEnabled(!silent));
    let start = Instant::now();
    player.send(Command::Play {
        from: 0.0,
        until: length,
        looping: false,
    });
    while start.elapsed() < Duration::from_secs_f64(length + 0.5) {
        std::thread::sleep(Duration::from_millis(250));
        println!(
            "wall {:.3} s\theard {:.3} s\tplaying {}",
            start.elapsed().as_secs_f64(),
            player.time(),
            player.is_playing()
        );
    }
}
