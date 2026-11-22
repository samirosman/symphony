//! Sound in the app: the audio output, the decoded sound of each file the
//! scene uses, and the background work that imports and decodes files.
//!
//! Decoding and resampling a long sound takes most of a second, so both
//! run on worker threads (E5.2) and show in the status bar (D17.5). The
//! results are kept by content key, so undo, redo and reopening a scene
//! never decode the same file twice.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use eframe::egui;
use symphony_audio::{decode, prepare, Clip, Command, Peaks, Player, Samples};
use symphony_core::sound::SoundFile;

use crate::session::Session;

/// Waveform slices a second: 16 a frame at 25 fps.
const PEAKS_PER_SECOND: u32 = 400;

/// A file ready to draw and play.
pub struct Ready {
    /// At the output's rate and channel count; empty with no output.
    pcm: Arc<Vec<f32>>,
    pub peaks: Peaks,
}

enum Done {
    Imported {
        name: String,
        result: Result<(u64, SoundFile, Ready), String>,
    },
    Decoded {
        key: u64,
        result: Result<Ready, String>,
    },
}

/// What the mixer last got, to send clips again only when they change.
#[derive(PartialEq)]
struct Sent {
    fps: u32,
    /// Key, on, start frame, trims, decoded.
    tracks: Vec<(u64, bool, u32, u64, u64, bool)>,
}

pub struct SoundState {
    player: Option<Player>,
    /// Opening the output was tried (it is tried once).
    opened: bool,
    ready: HashMap<u64, Ready>,
    decoding: HashSet<u64>,
    /// Files whose decode failed, so they aren't tried every frame.
    failed: HashSet<u64>,
    importing: usize,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    sent: Option<Sent>,
    /// Play > Enable Sound.
    enabled: bool,
    /// What the mixer last got for Enable Sound.
    sent_enabled: Option<bool>,
    /// Scene length and frame rate of the range the mixer is playing.
    started: Option<(u32, u32)>,
    /// The frame last shown from the sound, to see the user move it.
    shown: u32,
}

impl Default for SoundState {
    fn default() -> Self {
        let (tx, rx) = channel();
        SoundState {
            player: None,
            opened: false,
            ready: HashMap::new(),
            decoding: HashSet::new(),
            failed: HashSet::new(),
            importing: 0,
            tx,
            rx,
            sent: None,
            enabled: true,
            sent_enabled: None,
            started: None,
            shown: 0,
        }
    }
}

impl SoundState {
    /// Waveform peaks of a file, once it is decoded.
    pub fn peaks(&self, key: u64) -> Option<&Peaks> {
        self.ready.get(&key).map(|r| &r.peaks)
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Play > Enable Sound. Off, a scene with sound still plays in time
    /// with it, in silence.
    pub fn toggle_enabled(&mut self) {
        self.enabled = !self.enabled;
    }

    /// While a scene with sound plays, set the frame from the sound the
    /// listener hears: the picture follows the sound, as in Harmony, and
    /// skips images rather than letting the sound stutter. `false` when
    /// the app's own timer runs playback instead (no sound, or no output).
    pub fn drive(&mut self, s: &mut Session) -> bool {
        let Some(player) = &mut self.player else {
            return false;
        };
        if self.sent_enabled != Some(self.enabled) && player.send(Command::SetEnabled(self.enabled))
        {
            self.sent_enabled = Some(self.enabled);
        }
        if !s.playing || s.doc.sounds.is_empty() {
            if self.started.take().is_some() {
                player.send(Command::Stop);
            }
            return false;
        }
        let fps = s.doc.settings.fps.max(1.0);
        let len = s.length().max(1);
        let range = (len, fps.to_bits());
        // Started, the scene changed length or rate, or the user moved the
        // playhead: play again from the frame shown, looping to frame 1.
        if self.started != Some(range) || s.frame != self.shown {
            let per = f64::from(fps);
            let play = Command::Play {
                from: 0.0,
                until: f64::from(len) / per,
                at: f64::from(s.frame.clamp(1, len) - 1) / per,
                looping: true,
            };
            if player.send(play) {
                self.started = Some(range);
                self.shown = s.frame;
            }
            return true;
        }
        if player.caught_up() {
            let frame = (player.time() * f64::from(fps)).floor() as u32 + 1;
            s.frame = frame.clamp(1, len);
            self.shown = s.frame;
        }
        true
    }

    /// Words for the status bar while work runs (D17.5).
    pub fn activity(&self) -> Option<String> {
        match (self.importing, self.decoding.len()) {
            (0, 0) => None,
            (1, _) => Some("Importing sound".into()),
            (0, _) => Some("Reading sound".into()),
            (n, _) => Some(format!("Importing {n} sounds")),
        }
    }

    /// The output's rate and channels; 48 kHz stereo when there is none,
    /// which only the waveform uses then.
    fn format(&self) -> (u32, u16) {
        self.player
            .as_ref()
            .map_or((48_000, 2), |p| (p.rate(), p.channels()))
    }

    /// Open the default output the first time sound is needed. Without
    /// one the app works in silence and says so once.
    fn open(&mut self, s: &mut Session) {
        if self.opened {
            return;
        }
        self.opened = true;
        match Player::open() {
            Ok(p) => self.player = Some(p),
            Err(e) => s.notify(format!("Sound won't be heard: {e}")),
        }
    }

    /// Ask for files to import (File > Import > Sound), each decoded on its
    /// own worker thread. Each becomes a track at the bottom of the list.
    pub fn import_dialog(&mut self, s: &mut Session, ctx: &egui::Context) {
        let Some(paths) = rfd::FileDialog::new()
            .set_title("Import Sound")
            .add_filter("Sound", &["wav", "aif", "aiff", "mp3"])
            .pick_files()
        else {
            return;
        };
        self.import(paths, s, ctx);
    }

    pub fn import(&mut self, paths: Vec<PathBuf>, s: &mut Session, ctx: &egui::Context) {
        self.open(s);
        let (rate, channels) = self.format();
        let has_output = self.player.is_some();
        for path in paths {
            self.importing += 1;
            let (tx, ctx) = (self.tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                let name = track_name(&path);
                let result = symphony_audio::import(&path).and_then(|(key, file, samples)| {
                    let ready = make_ready(&samples, rate, channels, has_output)
                        .map_err(|e| format!("Couldn't import {}: {e}.", path.display()))?;
                    Ok((key, file, ready))
                });
                let _ = tx.send(Done::Imported { name, result });
                ctx.request_repaint();
            });
        }
    }

    /// Once a UI frame: take finished work, decode files of an opened
    /// scene, and give the mixer the scene's clips when they change.
    pub fn update(&mut self, s: &mut Session, ctx: &egui::Context) {
        while let Ok(done) = self.rx.try_recv() {
            match done {
                Done::Imported { name, result } => {
                    self.importing -= 1;
                    match result {
                        Ok((key, file, ready)) => {
                            self.failed.remove(&key);
                            self.ready.insert(key, ready);
                            s.import_sound(&name, key, file);
                        }
                        Err(e) => s.notify(e),
                    }
                }
                Done::Decoded { key, result } => {
                    self.decoding.remove(&key);
                    match result {
                        Ok(ready) => {
                            self.ready.insert(key, ready);
                        }
                        Err(e) => {
                            self.failed.insert(key);
                            s.notify(format!("A sound in this scene couldn't be read: {e}."));
                        }
                    }
                }
            }
        }
        if !s.doc.sound_files.is_empty() {
            self.open(s);
        }
        self.decode_missing(s, ctx);
        // Keep only what this scene uses.
        self.ready.retain(|k, _| s.doc.sound_files.contains_key(k));
        self.send_clips(s);
        if let Some(p) = &mut self.player {
            p.collect();
        }
    }

    fn decode_missing(&mut self, s: &Session, ctx: &egui::Context) {
        let (rate, channels) = self.format();
        let has_output = self.player.is_some();
        for (&key, file) in &s.doc.sound_files {
            if self.ready.contains_key(&key)
                || self.decoding.contains(&key)
                || self.failed.contains(&key)
            {
                continue;
            }
            self.decoding.insert(key);
            let (tx, ctx, bytes) = (self.tx.clone(), ctx.clone(), Arc::clone(&file.bytes));
            std::thread::spawn(move || {
                let result = decode::decode(&bytes)
                    .and_then(|samples| make_ready(&samples, rate, channels, has_output));
                let _ = tx.send(Done::Decoded { key, result });
                ctx.request_repaint();
            });
        }
    }

    fn send_clips(&mut self, s: &Session) {
        let fps = s.doc.settings.fps.max(1.0);
        let sent = Sent {
            fps: fps.to_bits(),
            tracks: s
                .doc
                .sounds
                .iter()
                .map(|t| {
                    (
                        t.key,
                        t.enabled,
                        t.start,
                        t.trim_start,
                        t.trim_end,
                        self.ready.contains_key(&t.key),
                    )
                })
                .collect(),
        };
        if self.sent.as_ref() == Some(&sent) {
            return;
        }
        let Some(player) = &mut self.player else {
            return;
        };
        let rate = player.rate();
        let clips = s
            .doc
            .sounds
            .iter()
            .filter_map(|t| {
                let r = self.ready.get(&t.key)?;
                let file = s.doc.sound_files.get(&t.key)?;
                let start = f64::from(t.start.saturating_sub(1)) / f64::from(fps);
                let clip = Clip::new(Arc::clone(&r.pcm), start, t.enabled, rate);
                Some(clip.trimmed(t.offset(file), t.seconds(file), rate))
            })
            .collect();
        if player.send(Command::SetClips(clips)) {
            self.sent = Some(sent);
        }
    }
}

/// Resample for the output and measure the waveform.
fn make_ready(
    samples: &Samples,
    rate: u32,
    channels: u16,
    has_output: bool,
) -> Result<Ready, String> {
    let pcm = if has_output {
        prepare(samples, rate, channels)?
    } else {
        Arc::new(Vec::new())
    };
    Ok(Ready {
        pcm,
        peaks: Peaks::new(samples, PEAKS_PER_SECOND),
    })
}

/// The track name: the file name without its extension, as in Harmony.
fn track_name(path: &Path) -> String {
    path.file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Sound".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_a_track_after_its_file() {
        assert_eq!(track_name(Path::new("/a/Dialogue 01.wav")), "Dialogue 01");
        assert_eq!(track_name(Path::new("/a/.wav")), ".wav");
        assert_eq!(track_name(Path::new("/")), "Sound");
    }
}
