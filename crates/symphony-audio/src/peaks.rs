//! Waveform peaks for drawing a sound in the Timeline. Computed once per
//! file on a worker thread (E5.2), then drawn every frame from the small
//! result, never from the samples.

use crate::decode::Samples;

/// The lowest and highest sample in each slice of time, over all
/// channels, in -1.0..=1.0.
#[derive(Clone, Debug, PartialEq)]
pub struct Peaks {
    /// Slices a second.
    pub per_second: u32,
    pub bins: Vec<(f32, f32)>,
}

impl Peaks {
    /// Peaks of `samples` with `per_second` slices a second. A higher
    /// value lets the Timeline zoom in further before the waveform turns
    /// blocky; 400 a second is enough for 16 slices a frame at 25 fps.
    pub fn new(samples: &Samples, per_second: u32) -> Peaks {
        let per_second = per_second.clamp(1, samples.rate.max(1));
        let channels = usize::from(samples.channels.max(1));
        let frames = samples.len();
        let bins = (frames as u64 * u64::from(per_second)).div_ceil(u64::from(samples.rate.max(1)));
        let bins = (0..bins as usize)
            .map(|b| {
                let from = b * frames / bins.max(1) as usize;
                let to = ((b + 1) * frames / bins.max(1) as usize)
                    .max(from + 1)
                    .min(frames);
                samples.data[from * channels..to * channels].iter().fold(
                    (0.0f32, 0.0f32),
                    |(lo, hi), &x| {
                        let x = if x.is_finite() {
                            x.clamp(-1.0, 1.0)
                        } else {
                            0.0
                        };
                        (lo.min(x), hi.max(x))
                    },
                )
            })
            .collect();
        Peaks { per_second, bins }
    }

    /// The peaks between two times in seconds, for one column of pixels.
    pub fn range(&self, from: f64, to: f64) -> (f32, f32) {
        let at = |t: f64| ((t.max(0.0) * f64::from(self.per_second)) as usize).min(self.bins.len());
        let (a, b) = (at(from), at(to).max(at(from) + 1).min(self.bins.len()));
        self.bins[a.min(b)..b]
            .iter()
            .fold((0.0, 0.0), |(lo, hi), &(l, h)| (lo.min(l), hi.max(h)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(data: Vec<f32>, channels: u16, rate: u32) -> Samples {
        Samples {
            rate,
            channels,
            data,
        }
    }

    #[test]
    fn one_bin_per_slice() {
        // One second at 8 Hz, in stereo, 4 slices.
        let mut data = Vec::new();
        for i in 0..8 {
            let v = i as f32 / 10.0;
            data.extend([v, -v]);
        }
        let p = Peaks::new(&samples(data, 2, 8), 4);
        assert_eq!(p.bins.len(), 4);
        assert_eq!(p.bins[0], (-0.1, 0.1));
        assert_eq!(p.bins[3], (-0.7, 0.7));
        assert_eq!(p.range(0.0, 1.0), (-0.7, 0.7));
        assert_eq!(p.range(0.0, 0.25), (-0.1, 0.1));
        assert_eq!(p.range(5.0, 6.0), (0.0, 0.0));
    }

    #[test]
    fn clips_and_ignores_bad_values() {
        let p = Peaks::new(&samples(vec![2.0, f32::NAN, -3.0], 1, 3), 1);
        assert_eq!(p.bins, [(-1.0, 1.0)]);
    }

    #[test]
    fn a_short_sound_has_a_bin() {
        let p = Peaks::new(&samples(vec![0.5], 1, 48_000), 400);
        assert_eq!(p.bins, [(0.0, 0.5)]);
    }
}
