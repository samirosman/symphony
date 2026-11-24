//! Sound for Symphony: checking and decoding imported sound files,
//! waveform peaks for the Timeline, and playback. Its own crate because it has its own
//! backend and its own reason to change (E1.4).
//!
//! - [`decode`]: WAV, AIFF and MP3 to float samples; the import check.
//! - [`peaks`]: waveform peaks, computed once per file.
//! - [`play`]: the mixer in the device callback, and the app's handle.
//! - [`soundtrack`]: File > Export > Soundtrack, the tracks as WAV files.

pub mod decode;
pub mod peaks;
pub mod play;
pub mod soundtrack;

pub use decode::{decode, decode_path, import, import_bytes, Samples};
pub use peaks::Peaks;
pub use play::{prepare, Clip, Command, Player};
