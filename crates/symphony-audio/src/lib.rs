//! Sound for Symphony: checking and decoding imported sound files, and
//! waveform peaks for the Timeline. Its own crate because it has its own
//! backend and its own reason to change (E1.4).
//!
//! - [`decode`]: WAV, AIFF and MP3 to float samples; the import check.
//! - [`peaks`]: waveform peaks, computed once per file.

pub mod decode;
pub mod peaks;

pub use decode::{decode, import, import_bytes, Samples};
pub use peaks::Peaks;
