//! Playback: a mixer that runs in the audio device's callback, and the
//! handle the app uses to control it.
//!
//! The callback runs on a real-time thread, so it never locks, allocates
//! or frees (a late callback is a click the user hears). Commands reach it
//! through a lock-free ring buffer. Track data it no longer needs goes
//! back through a second ring, so the app thread frees it, since freeing
//! a long sound can take milliseconds.
//!
//! Times are scene times: 0.0 is the start of frame 1. Each clip is
//! resampled to the device rate and channel count once, on a worker
//! thread ([`prepare`]), so the callback only adds samples together.
//!
//! While playing, the app reads [`Player::time`] and shows the frame the
//! listener hears, so the picture follows the sound, as in Harmony.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use rtrb::{Consumer, Producer, RingBuffer};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

use crate::decode::Samples;

/// One sound ready to play: samples at the device rate and channel count,
/// and where it starts in the scene.
#[derive(Clone, Debug)]
pub struct Clip {
    pub samples: Arc<Vec<f32>>,
    /// Device frames from the start of the scene to the clip's start.
    pub start: u64,
    pub enabled: bool,
}

impl Clip {
    /// A clip that starts `start_seconds` into the scene.
    pub fn new(samples: Arc<Vec<f32>>, start_seconds: f64, enabled: bool, rate: u32) -> Clip {
        Clip {
            samples,
            start: (start_seconds.max(0.0) * f64::from(rate)).round() as u64,
            enabled,
        }
    }
}

/// Convert decoded samples to the device rate and channel count. Slow for
/// long sounds: run it on a worker thread (E5.2).
pub fn prepare(s: &Samples, rate: u32, channels: u16) -> Result<Arc<Vec<f32>>, String> {
    let mixed = remix(s, channels);
    if s.rate == rate || s.is_empty() {
        return Ok(Arc::new(mixed));
    }
    let ch = usize::from(channels);
    let frames = mixed.len() / ch;
    let fail = |e: &dyn std::fmt::Display| format!("the sound couldn't be resampled ({e})");
    let mut r = Fft::<f32>::new(s.rate as usize, rate as usize, 1024, ch, FixedSync::Both)
        .map_err(|e| fail(&e))?;
    let needed = r.process_all_needed_output_len(frames);
    let mut out = vec![0.0; needed * ch];
    let input = InterleavedSlice::new(&mixed, ch, frames).map_err(|e| fail(&e))?;
    let mut output = InterleavedSlice::new_mut(&mut out, ch, needed).map_err(|e| fail(&e))?;
    let (_, produced) = r
        .process_all_into_buffer(&input, &mut output, frames, None)
        .map_err(|e| fail(&e))?;
    out.truncate(produced * ch);
    Ok(Arc::new(out))
}

/// Map channels: mono plays on every device channel; otherwise source
/// channel `c` goes to device channel `c % channels`, averaged.
fn remix(s: &Samples, channels: u16) -> Vec<f32> {
    let (from, to) = (usize::from(s.channels.max(1)), usize::from(channels.max(1)));
    if from == to {
        return s.data.clone();
    }
    let mut out = Vec::with_capacity(s.len() * to);
    for frame in s.data.chunks_exact(from) {
        for d in 0..to {
            if from == 1 {
                out.push(frame[0]);
            } else {
                let (sum, n) = frame
                    .iter()
                    .enumerate()
                    .filter(|(c, _)| c % to == d)
                    .fold((0.0, 0), |(s, n), (_, &x)| (s + x, n + 1));
                out.push(if n > 0 { sum / n as f32 } else { 0.0 });
            }
        }
    }
    out
}

/// What the app asks the mixer to do.
pub enum Command {
    /// Replace every clip (a track was added, muted, moved).
    SetClips(Vec<Clip>),
    /// Play from `from` seconds. Stop at `until`, or go back to `from`
    /// there when `looping`.
    Play {
        from: f64,
        until: f64,
        looping: bool,
    },
    Stop,
    /// Play `length` seconds from `at` once, then stop (scrubbing).
    Scrub {
        at: f64,
        length: f64,
    },
    /// The Enable Sound switch: off keeps time running in silence.
    SetEnabled(bool),
}

/// Shared with the app: what the mixer is doing, without a lock.
#[derive(Debug)]
pub struct Clock {
    rate: f64,
    base: Instant,
    /// Scene time of the first frame of the last buffer, in device frames.
    position: AtomicU64,
    /// When that buffer was written, in nanoseconds after `base`.
    stamp: AtomicU64,
    /// Frames between writing a sample and hearing it.
    latency: AtomicU64,
    /// Playing when the last buffer started.
    active: AtomicBool,
    /// Still playing after it.
    playing: AtomicBool,
    from: AtomicU64,
    until: AtomicU64,
    looping: AtomicBool,
}

impl Clock {
    fn new(rate: u32) -> Clock {
        Clock {
            rate: f64::from(rate.max(1)),
            base: Instant::now(),
            position: AtomicU64::new(0),
            stamp: AtomicU64::new(0),
            latency: AtomicU64::new(0),
            active: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            from: AtomicU64::new(0),
            until: AtomicU64::new(0),
            looping: AtomicBool::new(false),
        }
    }

    /// The scene time heard at `now`, in seconds. Between two callbacks it
    /// moves on with the wall clock, so the picture doesn't step by whole
    /// buffers; the output latency is taken off.
    fn heard(&self, now: Instant) -> f64 {
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed) as f64;
        let pos = get(&self.position);
        if !self.active.load(Ordering::Relaxed) {
            return pos / self.rate;
        }
        let now_ns = now.saturating_duration_since(self.base).as_nanos() as f64;
        let since = (now_ns - get(&self.stamp)).max(0.0) * 1e-9 * self.rate;
        let (from, until) = (get(&self.from), get(&self.until));
        let mut t = pos + since - get(&self.latency);
        if self.looping.load(Ordering::Relaxed) && until > from {
            let span = until - from;
            if t >= until {
                t = from + (t - from) % span;
            } else if t < from {
                // Still hearing the end of the last time round.
                t = until - (from - t) % span;
            }
        } else {
            t = t.min(until);
        }
        t.max(0.0) / self.rate
    }
}

/// The part that runs in the device callback.
pub struct Mixer {
    rate: u32,
    channels: usize,
    clips: Vec<Clip>,
    position: u64,
    playing: bool,
    from: u64,
    until: u64,
    looping: bool,
    enabled: bool,
    commands: Consumer<Command>,
    garbage: Producer<Vec<Clip>>,
    /// Clips waiting for room in the garbage ring.
    held: Option<Vec<Clip>>,
    clock: Arc<Clock>,
}

/// Room for this many commands between two callbacks.
const COMMANDS: usize = 64;

impl Mixer {
    /// A mixer and the ends the app keeps.
    pub fn new(rate: u32, channels: u16) -> (Mixer, Producer<Command>, Consumer<Vec<Clip>>) {
        let (cmd_tx, cmd_rx) = RingBuffer::new(COMMANDS);
        let (gar_tx, gar_rx) = RingBuffer::new(COMMANDS);
        let mixer = Mixer {
            rate: rate.max(1),
            channels: usize::from(channels.max(1)),
            clips: Vec::new(),
            position: 0,
            playing: false,
            from: 0,
            until: 0,
            looping: false,
            enabled: true,
            commands: cmd_rx,
            garbage: gar_tx,
            held: None,
            clock: Arc::new(Clock::new(rate)),
        };
        (mixer, cmd_tx, gar_rx)
    }

    pub fn clock(&self) -> Arc<Clock> {
        Arc::clone(&self.clock)
    }

    fn frames(&self, seconds: f64) -> u64 {
        (seconds.max(0.0) * f64::from(self.rate)).round() as u64
    }

    fn take_commands(&mut self) {
        while let Ok(c) = self.commands.pop() {
            match c {
                Command::SetClips(clips) => {
                    let old = std::mem::replace(&mut self.clips, clips);
                    // An empty list owns nothing to free.
                    if old.capacity() > 0 {
                        if let Err(rtrb::PushError::Full(old)) = self.garbage.push(old) {
                            // The app hasn't collected for a while. Keep one
                            // list waiting; past that (more than 64 swaps
                            // uncollected) it is freed here, the lesser harm.
                            if self.held.is_none() {
                                self.held = Some(old);
                            }
                        }
                    }
                }
                Command::Play {
                    from,
                    until,
                    looping,
                } => {
                    self.from = self.frames(from);
                    self.until = self.frames(until).max(self.from + 1);
                    self.position = self.from;
                    self.looping = looping;
                    self.playing = true;
                }
                Command::Stop => self.playing = false,
                Command::Scrub { at, length } => {
                    self.from = self.frames(at);
                    self.until = self.from + self.frames(length).max(1);
                    self.position = self.from;
                    self.looping = false;
                    self.playing = true;
                }
                Command::SetEnabled(on) => self.enabled = on,
            }
        }
        if let Some(h) = self.held.take() {
            if let Err(rtrb::PushError::Full(h)) = self.garbage.push(h) {
                self.held = Some(h);
            }
        }
    }

    /// Fill `out` (interleaved, device channels) with the next samples.
    /// Never locks, allocates or frees.
    pub fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        self.take_commands();
        let c = &self.clock;
        let now = Instant::now().saturating_duration_since(c.base).as_nanos();
        c.stamp.store(now as u64, Ordering::Relaxed);
        c.position.store(self.position, Ordering::Relaxed);
        c.from.store(self.from, Ordering::Relaxed);
        c.until.store(self.until, Ordering::Relaxed);
        c.looping.store(self.looping, Ordering::Relaxed);
        c.active.store(self.playing, Ordering::Relaxed);
        let ch = self.channels;
        for frame in out.chunks_exact_mut(ch) {
            if !self.playing {
                break;
            }
            if self.position >= self.until {
                if self.looping {
                    self.position = self.from;
                } else {
                    self.playing = false;
                    break;
                }
            }
            if self.enabled {
                for clip in self.clips.iter().filter(|c| c.enabled) {
                    let Some(i) = self.position.checked_sub(clip.start) else {
                        continue;
                    };
                    let at = i as usize * ch;
                    if let Some(src) = clip.samples.get(at..at + ch) {
                        for (o, s) in frame.iter_mut().zip(src) {
                            *o += s;
                        }
                    }
                }
                for o in frame.iter_mut() {
                    *o = o.clamp(-1.0, 1.0);
                }
            }
            self.position += 1;
        }
        if !self.playing {
            // Stopped in this buffer: report where, not a guess.
            self.clock.position.store(self.position, Ordering::Relaxed);
        }
        self.clock.playing.store(self.playing, Ordering::Relaxed);
    }

    /// Record how far ahead of the speaker the callback writes.
    pub fn set_latency(&mut self, seconds: f64) {
        let frames = self.frames(seconds);
        self.clock.latency.store(frames, Ordering::Relaxed);
    }
}

/// The app's handle on playback, on the default output device. Dropping
/// it stops the sound.
pub struct Player {
    rate: u32,
    channels: u16,
    commands: Producer<Command>,
    garbage: Consumer<Vec<Clip>>,
    clock: Arc<Clock>,
    _stream: cpal::Stream,
}

/// Largest callback buffer the mixer fills in one go, in frames; bigger
/// requests are filled in parts.
const CHUNK: usize = 4096;

impl Player {
    /// Open the default output device. An error means there's no sound
    /// output; the app keeps working in silence.
    pub fn open() -> Result<Player, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        use cpal::SampleFormat;
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no sound output was found")?;
        let config = device
            .default_output_config()
            .map_err(|e| format!("the sound output can't be used ({e})"))?;
        let format = config.sample_format();
        let config: cpal::StreamConfig = config.into();
        let (rate, channels) = (config.sample_rate, config.channels);
        let (mixer, commands, garbage) = Mixer::new(rate, channels);
        let clock = mixer.clock();
        let stream = match format {
            SampleFormat::F32 => stream::<f32>(&device, config, mixer),
            SampleFormat::I16 => stream::<i16>(&device, config, mixer),
            SampleFormat::I32 => stream::<i32>(&device, config, mixer),
            SampleFormat::U16 => stream::<u16>(&device, config, mixer),
            other => return Err(format!("the sound output wants {other} samples")),
        }?;
        stream
            .play()
            .map_err(|e| format!("the sound output didn't start ({e})"))?;
        Ok(Player {
            rate,
            channels,
            commands,
            garbage,
            clock,
            _stream: stream,
        })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Send a command. `false` if the mixer is too far behind to take it.
    pub fn send(&mut self, c: Command) -> bool {
        self.collect();
        self.commands.push(c).is_ok()
    }

    /// Free track data the mixer has let go of. Called by [`send`]; call it
    /// once a UI frame too.
    pub fn collect(&mut self) {
        while self.garbage.pop().is_ok() {}
    }

    pub fn is_playing(&self) -> bool {
        self.clock.playing.load(Ordering::Relaxed)
    }

    /// The scene time the listener hears now, in seconds.
    pub fn time(&self) -> f64 {
        self.clock.heard(Instant::now())
    }
}

fn stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut mixer: Mixer,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;
    let ch = usize::from(config.channels.max(1));
    let mut scratch = vec![0.0f32; CHUNK * ch];
    device
        .build_output_stream(
            config,
            move |out: &mut [T], info: &cpal::OutputCallbackInfo| {
                let ts = info.timestamp();
                if let Some(d) = ts.playback.checked_duration_since(ts.callback) {
                    mixer.set_latency(d.as_secs_f64());
                }
                for part in out.chunks_mut(CHUNK * ch) {
                    let buf = &mut scratch[..part.len()];
                    mixer.render(buf);
                    for (o, &s) in part.iter_mut().zip(buf.iter()) {
                        *o = T::from_sample(s);
                    }
                }
            },
            // Errors reach the user as silence; the app can reopen.
            |_| {},
            None,
        )
        .map_err(|e| format!("the sound output can't be used ({e})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(samples: Vec<f32>, start: u64) -> Clip {
        Clip {
            samples: Arc::new(samples),
            start,
            enabled: true,
        }
    }

    /// A mono mixer at 10 Hz, so one sample is a tenth of a second.
    fn mixer() -> (Mixer, Producer<Command>, Consumer<Vec<Clip>>) {
        Mixer::new(10, 1)
    }

    /// The heard time `after` seconds past the last callback.
    fn heard_at(m: &Mixer, after: f64) -> f64 {
        let c = m.clock();
        let stamp = std::time::Duration::from_nanos(c.stamp.load(Ordering::Relaxed));
        let t = c.heard(c.base + stamp + std::time::Duration::from_secs_f64(after));
        (t * 1000.0).round() / 1000.0
    }

    fn render(m: &mut Mixer, n: usize) -> Vec<f32> {
        let mut out = vec![9.0; n];
        m.render(&mut out);
        out
    }

    #[test]
    fn silent_until_played() {
        let (mut m, mut tx, _) = mixer();
        tx.push(Command::SetClips(vec![clip(vec![0.5; 4], 0)])).ok();
        assert_eq!(render(&mut m, 3), [0.0; 3]);
    }

    #[test]
    fn plays_clips_at_their_start_and_adds_them() {
        let (mut m, mut tx, _) = mixer();
        let clips = vec![clip(vec![0.25; 3], 0), clip(vec![0.5; 3], 2)];
        tx.push(Command::SetClips(clips)).ok();
        tx.push(Command::Play {
            from: 0.0,
            until: 1.0,
            looping: false,
        })
        .ok();
        assert_eq!(render(&mut m, 6), [0.25, 0.25, 0.75, 0.5, 0.5, 0.0]);
        // The buffer started at 0.0 s, and time moves on after it.
        assert!(m.clock().playing.load(Ordering::Relaxed));
        assert_eq!(heard_at(&m, 0.0), 0.0);
        assert_eq!(heard_at(&m, 0.6), 0.6);
    }

    #[test]
    fn stops_at_the_end_or_loops() {
        let (mut m, mut tx, _) = mixer();
        tx.push(Command::SetClips(vec![clip(vec![0.1, 0.2, 0.3], 0)]))
            .ok();
        tx.push(Command::Play {
            from: 0.0,
            until: 0.2,
            looping: false,
        })
        .ok();
        assert_eq!(render(&mut m, 4), [0.1, 0.2, 0.0, 0.0]);
        assert!(!m.clock().playing.load(Ordering::Relaxed));
        tx.push(Command::Play {
            from: 0.1,
            until: 0.3,
            looping: true,
        })
        .ok();
        assert_eq!(render(&mut m, 5), [0.2, 0.3, 0.2, 0.3, 0.2]);
    }

    #[test]
    fn scrub_plays_a_slice_once() {
        let (mut m, mut tx, _) = mixer();
        tx.push(Command::SetClips(vec![clip(vec![0.1, 0.2, 0.3, 0.4], 0)]))
            .ok();
        tx.push(Command::Scrub {
            at: 0.1,
            length: 0.2,
        })
        .ok();
        assert_eq!(render(&mut m, 4), [0.2, 0.3, 0.0, 0.0]);
    }

    #[test]
    fn muted_clips_and_sound_off_are_silent_but_time_runs() {
        let (mut m, mut tx, _) = mixer();
        let mut muted = clip(vec![0.5; 4], 0);
        muted.enabled = false;
        tx.push(Command::SetClips(vec![muted, clip(vec![0.25; 4], 0)]))
            .ok();
        tx.push(Command::Play {
            from: 0.0,
            until: 1.0,
            looping: false,
        })
        .ok();
        assert_eq!(render(&mut m, 2), [0.25, 0.25]);
        tx.push(Command::SetEnabled(false)).ok();
        assert_eq!(render(&mut m, 2), [0.0, 0.0]);
        assert_eq!(heard_at(&m, 0.0), 0.2);
    }

    #[test]
    fn loud_mixes_are_clipped() {
        let (mut m, mut tx, _) = mixer();
        tx.push(Command::SetClips(vec![
            clip(vec![0.8], 0),
            clip(vec![0.8], 0),
        ]))
        .ok();
        tx.push(Command::Play {
            from: 0.0,
            until: 1.0,
            looping: false,
        })
        .ok();
        assert_eq!(render(&mut m, 1), [1.0]);
    }

    #[test]
    fn replaced_clips_go_back_to_the_app() {
        let (mut m, mut tx, mut garbage) = mixer();
        let first = Arc::new(vec![0.5; 4]);
        tx.push(Command::SetClips(vec![Clip {
            samples: Arc::clone(&first),
            start: 0,
            enabled: true,
        }]))
        .ok();
        render(&mut m, 1);
        tx.push(Command::SetClips(Vec::new())).ok();
        render(&mut m, 1);
        let back = garbage.pop().unwrap();
        // The mixer let go of it; the app drops the last reference.
        assert_eq!(Arc::strong_count(&first), 2);
        drop(back);
        assert_eq!(Arc::strong_count(&first), 1);
    }

    #[test]
    fn latency_is_taken_off_the_heard_time() {
        let (mut m, mut tx, _) = mixer();
        tx.push(Command::Play {
            from: 1.0,
            until: 10.0,
            looping: false,
        })
        .ok();
        m.set_latency(0.2);
        render(&mut m, 5);
        // The buffer starts at 1.0 s; the speaker is 0.2 s behind.
        assert_eq!(heard_at(&m, 0.0), 0.8);
        assert_eq!(heard_at(&m, 0.5), 1.3);
    }

    #[test]
    fn heard_time_wraps_with_a_loop_and_stops_at_the_end() {
        let (mut m, mut tx, _) = mixer();
        tx.push(Command::Play {
            from: 0.1,
            until: 0.3,
            looping: true,
        })
        .ok();
        render(&mut m, 1);
        assert_eq!(heard_at(&m, 0.25), 0.15);
        tx.push(Command::Play {
            from: 0.0,
            until: 0.3,
            looping: false,
        })
        .ok();
        render(&mut m, 1);
        assert_eq!(heard_at(&m, 5.0), 0.3);
        render(&mut m, 4);
        render(&mut m, 1);
        assert!(!m.clock().playing.load(Ordering::Relaxed));
        assert_eq!(heard_at(&m, 5.0), 0.3);
    }

    #[test]
    fn prepare_remixes_and_resamples() {
        let stereo = Samples {
            rate: 48_000,
            channels: 2,
            data: vec![0.2, 0.4, 0.2, 0.4],
        };
        let mono = prepare(&stereo, 48_000, 1).unwrap();
        assert!(mono.iter().all(|&x| (x - 0.3).abs() < 1e-6));
        let up = Samples {
            rate: 1,
            channels: 1,
            data: vec![0.5],
        };
        assert_eq!(*prepare(&up, 1, 2).unwrap(), [0.5, 0.5]);
        // One second at 44.1 kHz becomes one second at 48 kHz.
        let tone: Vec<f32> = (0..44_100)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin() * 0.5)
            .collect();
        let s = Samples {
            rate: 44_100,
            channels: 1,
            data: tone,
        };
        let out = prepare(&s, 48_000, 1).unwrap();
        assert!((out.len() as i64 - 48_000).abs() <= 1, "{}", out.len());
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!((peak - 0.5).abs() < 0.02, "{peak}");
    }
}
