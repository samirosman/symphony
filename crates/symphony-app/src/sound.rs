//! Sound in the app: the audio output, the decoded sound of each file the
//! scene uses, and the background work that decodes files. Import itself
//! is in [`crate::import`].
//!
//! Decoding and resampling a long sound takes most of a second, so both
//! run on worker threads (E5.2) and show in the status bar (D17.5). The
//! results are kept by content key, so undo, redo and reopening a scene
//! never decode the same file twice.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use eframe::egui;
use symphony_audio::{prepare, Clip, Command, Peaks, Player, Samples};
use symphony_core::doc::MediaKind;
use symphony_core::id::Id;

use crate::session::Session;

/// Waveform slices a second: 16 a frame at 25 fps.
const PEAKS_PER_SECOND: u32 = 400;

/// A file ready to draw and play.
pub struct Ready {
    /// At the output's rate and channel count; empty with no output.
    pcm: Arc<Vec<f32>>,
    peaks: Peaks,
    /// Scale that makes the loudest sample of the file fill the row.
    gain: f32,
}

struct Done {
    key: u64,
    result: Result<Ready, String>,
}

/// What the mixer last got, to send clips again only when they change.
#[derive(PartialEq)]
struct Sent {
    fps: u32,
    solo: Option<Id>,
    /// Key, muted, start frame, trims, decoded.
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
    tx: Sender<Done>,
    rx: Receiver<Done>,
    sent: Option<Sent>,
    /// Play > Enable Sound.
    enabled: bool,
    /// Play > Enable Sound Scrubbing.
    scrubbing: bool,
    /// What the mixer last got for Enable Sound.
    sent_enabled: Option<bool>,
    /// What the mixer is playing: the first and last frame, the frame
    /// rate, and whether it loops.
    started: Option<(u32, u32, u32, bool)>,
    /// The frame last shown from the sound, to see the user move it.
    shown: u32,
    /// The track the Sound editor is playing on its own.
    solo: Option<Id>,
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
            tx,
            rx,
            sent: None,
            enabled: true,
            scrubbing: false,
            sent_enabled: None,
            started: None,
            shown: 0,
            solo: None,
        }
    }
}

impl SoundState {
    /// Waveform peaks of a file, once it is decoded, and the scale that
    /// normalizes its drawing (the sound itself plays as it is).
    pub fn peaks(&self, key: u64) -> Option<(&Peaks, f32)> {
        self.ready.get(&key).map(|r| (&r.peaks, r.gain))
    }

    /// A decoded file's samples, interleaved at the output's rate and
    /// channel count, with that rate and count. Empty with no output.
    pub fn samples(&self, key: u64) -> Option<(&[f32], u32, u16)> {
        let (rate, channels) = self.format();
        self.ready
            .get(&key)
            .map(|r| (r.pcm.as_slice(), rate, channels))
    }

    /// Play track `id` alone, from the start to the end of its trims, once.
    /// Muted, it still plays, as the Sound editor asks for it by name.
    pub fn preview(&mut self, s: &Session, id: Id) {
        let Some(track) = s.doc.sounds.iter().find(|t| t.id == id) else {
            return;
        };
        let Some(file) = s.doc.media.get(&track.media) else {
            return;
        };
        let fps = f64::from(s.doc.settings.fps.max(1.0));
        let (at, length) = (f64::from(track.start.max(1) - 1) / fps, track.seconds(file));
        self.solo = Some(id);
        self.send_clips(s);
        if let Some(player) = &mut self.player {
            player.send(Command::Scrub { at, length });
        }
    }

    pub fn stop_preview(&mut self) {
        if self.solo.take().is_some() {
            if let Some(player) = &mut self.player {
                player.send(Command::Stop);
            }
        }
    }

    /// The scene time a preview has reached, in seconds, while it plays.
    pub fn preview_time(&self) -> Option<f64> {
        let player = self.player.as_ref()?;
        (self.solo.is_some() && player.caught_up() && player.is_playing()).then(|| player.time())
    }

    pub fn previewing(&self) -> bool {
        self.solo.is_some()
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Play > Enable Sound. Off, a scene with sound still plays in time
    /// with it, in silence.
    pub fn toggle_enabled(&mut self) {
        self.enabled = !self.enabled;
    }

    pub fn scrubbing(&self) -> bool {
        self.scrubbing
    }

    /// Play > Enable Sound Scrubbing. Off at start, as in Harmony, so
    /// stepping through frames while drawing stays quiet.
    pub fn toggle_scrubbing(&mut self) {
        self.scrubbing = !self.scrubbing;
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
        if s.playing {
            self.solo = None;
        }
        if !s.playing || s.doc.sounds.is_empty() {
            if self.started.take().is_some() {
                player.send(Command::Stop);
            }
            if s.frame != self.shown {
                self.shown = s.frame;
                // Scrubbing: the playhead moved by hand, so play the slice
                // of sound under the new frame, once.
                if self.scrubbing && self.enabled && !s.doc.sounds.is_empty() {
                    let per = f64::from(s.doc.settings.fps.max(1.0));
                    player.send(Command::Scrub {
                        at: f64::from(s.frame.max(1) - 1) / per,
                        length: 1.0 / per,
                    });
                }
            }
            return false;
        }
        let fps = s.doc.settings.fps.max(1.0);
        let per = f64::from(fps);
        let len = s.length().max(1);
        let (first, last, at) = play_span(s.frame, s.play_range, len);
        let range = (first, last, fps.to_bits(), s.loop_playback);
        // Started, the range, rate or loop changed, or the user moved the
        // playhead: play again from `at`.
        if self.started != Some(range) || s.frame != self.shown {
            let play = Command::Play {
                from: f64::from(first - 1) / per,
                until: f64::from(last) / per,
                at: f64::from(at - 1) / per,
                looping: s.loop_playback,
            };
            if player.send(play) {
                self.started = Some(range);
                s.frame = at;
                self.shown = at;
            }
            return true;
        }
        if player.caught_up() {
            let time = player.time();
            let frame = (time * per).floor() as u32 + 1;
            s.frame = frame.clamp(first, last);
            self.shown = s.frame;
            // Loop off: the sound holds at the range end, and playback stops.
            if !s.loop_playback && time >= f64::from(last) / per - 1e-9 {
                s.playing = false;
            }
        }
        true
    }

    /// Words for the status bar while work runs (D17.5).
    pub fn activity(&self) -> Option<String> {
        (!self.decoding.is_empty()).then(|| "Reading sound".into())
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

    /// Once a UI frame: take finished work, decode files of an opened
    /// scene, and give the mixer the scene's clips when they change.
    pub fn update(&mut self, s: &mut Session, ctx: &egui::Context) {
        while let Ok(Done { key, result }) = self.rx.try_recv() {
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
        if !s.doc.sounds.is_empty() {
            self.open(s);
        }
        self.decode_missing(s, ctx);
        // Keep only what this scene uses.
        self.ready.retain(|k, _| s.doc.media.contains_key(k));
        if let Some(p) = &self.player {
            if self.solo.is_some() && p.caught_up() && !p.is_playing() {
                self.solo = None;
            }
        }
        self.send_clips(s);
        if let Some(p) = &mut self.player {
            p.collect();
        }
    }

    fn decode_missing(&mut self, s: &Session, ctx: &egui::Context) {
        let (rate, channels) = self.format();
        let has_output = self.player.is_some();
        for (&key, m) in &s.doc.media {
            if m.kind != MediaKind::Sound
                || self.ready.contains_key(&key)
                || self.decoding.contains(&key)
                || self.failed.contains(&key)
            {
                continue;
            }
            let Some(path) = s.doc.media_path(key) else {
                // Not on disk (yet): a scene before its files are copied.
                continue;
            };
            self.decoding.insert(key);
            let (tx, ctx) = (self.tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                let result = symphony_audio::decode_path(&path)
                    .and_then(|samples| make_ready(&samples, rate, channels, has_output));
                let _ = tx.send(Done { key, result });
                ctx.request_repaint();
            });
        }
    }

    fn send_clips(&mut self, s: &Session) {
        let fps = s.doc.settings.fps.max(1.0);
        let sent = Sent {
            fps: fps.to_bits(),
            solo: self.solo,
            tracks: s
                .doc
                .sounds
                .iter()
                .map(|t| {
                    (
                        t.media,
                        t.muted,
                        t.start,
                        t.trim_start,
                        t.trim_end,
                        self.ready.contains_key(&t.media),
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
            .filter(|t| self.solo.is_none_or(|id| id == t.id))
            .filter_map(|t| {
                let r = self.ready.get(&t.media)?;
                let file = s.doc.media.get(&t.media)?;
                let start = f64::from(t.start.saturating_sub(1)) / f64::from(fps);
                let on = !t.muted || self.solo.is_some();
                let clip = Clip::new(Arc::clone(&r.pcm), start, on, rate);
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
    let peaks = Peaks::new(samples, PEAKS_PER_SECOND);
    Ok(Ready {
        pcm,
        gain: normalize(&peaks),
        peaks,
    })
}

/// The scale that makes the loudest peak reach 1. A silent file is left
/// as it is, so noise isn't blown up to full height.
fn normalize(peaks: &Peaks) -> f32 {
    let loudest = peaks
        .bins
        .iter()
        .fold(0.0f32, |m, &(lo, hi)| m.max(-lo).max(hi));
    if loudest < 1e-3 {
        1.0
    } else {
        1.0 / loudest
    }
}

/// The frames sound playback covers, first and last, and the frame to
/// start from: the playback range (Play menu) held inside the scene, or
/// the whole scene; the frame shown, or the range's first frame when the
/// playhead is outside it.
fn play_span(frame: u32, range: Option<(u32, u32)>, len: u32) -> (u32, u32, u32) {
    let len = len.max(1);
    let (a, b) = range.map_or((1, len), |(a, b)| (a.clamp(1, len), b.clamp(1, len)));
    let (first, last) = (a.min(b), a.max(b));
    let at = if (first..=last).contains(&frame) {
        frame
    } else {
        first
    };
    (first, last, at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_plays_inside_the_playback_range() {
        // No range: the whole scene, from the frame shown.
        assert_eq!(play_span(7, None, 75), (1, 75, 7));
        // A range: from the playhead inside it, else from its first frame.
        assert_eq!(play_span(12, Some((5, 19)), 75), (5, 19, 12));
        assert_eq!(play_span(37, Some((5, 19)), 75), (5, 19, 5));
        assert_eq!(play_span(1, Some((19, 5)), 75), (5, 19, 5));
        // A range past the scene end is held to the scene.
        assert_eq!(play_span(1, Some((70, 90)), 75), (70, 75, 70));
    }

    #[test]
    fn the_loudest_peak_fills_the_row() {
        let peaks = |bins| Peaks {
            per_second: 1,
            bins,
        };
        assert_eq!(normalize(&peaks(vec![(-0.25, 0.1), (0.0, 0.2)])), 4.0);
        assert_eq!(normalize(&peaks(vec![(0.0, 0.0)])), 1.0);
    }
}
