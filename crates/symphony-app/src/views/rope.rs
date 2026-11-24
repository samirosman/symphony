//! Node cables as ropes: a chain of points with momentum, gravity and a
//! fixed length (Verlet integration with distance constraints), pinned
//! where it's plugged in. Used while a cable is dragged and for the moments
//! after a connection changes; a resting cable is drawn as its plain curve,
//! and the rope eases into exactly that curve before it stops, so the
//! hand-off doesn't jump.

use eframe::egui::{Pos2, Vec2};

/// Points along a rope.
const POINTS: usize = 28;
/// Simulation step, seconds.
const STEP: f32 = 1.0 / 240.0;
/// Cap catch-up so slow artwork frames do not trigger a burst of rope work.
const MAX_CATCH_UP: f32 = 12.0 * STEP;
/// Velocity kept per step (air drag).
const DAMPING: f32 = 0.955;
/// Constraint passes per step: higher is a stiffer, less stretchy rope.
const PASSES: usize = 10;
/// How strongly the rope resists bending (0 = thread).
const BEND: f32 = 0.4;
/// Cable gravity at 100% graph zoom, in points per second squared.
pub const GRAVITY: f32 = 900.0;
/// Small allowance for the cable to bow while following the pointer.
pub const DRAG_SLACK: f32 = 1.025;
/// A small pull towards the resting curve removes ripples during a drag.
pub const DRAG_PULL: f32 = 0.055;

#[derive(Clone)]
pub struct Rope {
    pts: Vec<Pos2>,
    prev: Vec<Pos2>,
    /// Unsimulated time carried to the next frame.
    spare: f32,
}

/// Where the rope is held.
#[derive(Clone, Copy)]
pub struct Pins {
    /// The output end (always held).
    pub start: Pos2,
    /// The other end: held, or None to swing free.
    pub end: Option<Pos2>,
}

impl Rope {
    /// A still rope lying along `path` (resampled evenly).
    pub fn along(path: &[Pos2]) -> Self {
        let pts = resample(path, POINTS);
        Rope {
            prev: pts.clone(),
            pts,
            spare: 0.0,
        }
    }

    pub fn points(&self) -> &[Pos2] {
        &self.pts
    }

    /// The rope as a smooth line for drawing (Catmull-Rom, a few points
    /// between each pair).
    pub fn smooth(&self) -> Vec<Pos2> {
        let p = &self.pts;
        let n = p.len();
        let mut out = Vec::with_capacity(n * 3);
        for i in 0..n.saturating_sub(1) {
            let (p0, p1, p2, p3) = (
                p[i.saturating_sub(1)],
                p[i],
                p[i + 1],
                p[(i + 2).min(n - 1)],
            );
            for k in 0..3 {
                let t = k as f32 / 3.0;
                let (t2, t3) = (t * t, t * t * t);
                let f = |a: f32, b: f32, c: f32, d: f32| {
                    0.5 * (2.0 * b
                        + (-a + c) * t
                        + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                        + (-a + 3.0 * b - 3.0 * c + d) * t3)
                };
                out.push(Pos2::new(
                    f(p0.x, p1.x, p2.x, p3.x),
                    f(p0.y, p1.y, p2.y, p3.y),
                ));
            }
        }
        if let Some(last) = p.last() {
            out.push(*last);
        }
        out
    }

    /// Give the rope a push: velocity `v` at the free end, fading to none
    /// at the held start (a whip).
    pub fn kick(&mut self, v: Vec2) {
        let n = (self.pts.len() - 1).max(1) as f32;
        for (i, prev) in self.prev.iter_mut().enumerate() {
            let u = i as f32 / n;
            *prev -= v * STEP * u * u;
        }
    }

    #[cfg(test)]
    pub fn end(&self) -> Pos2 {
        *self.pts.last().unwrap_or(&Pos2::ZERO)
    }

    /// Advance `dt` seconds. `length` is the rope's length; `gravity` in
    /// points per second squared (down); `shape` pulls each point towards
    /// the matching point of a target curve with strength `pull` (0..1 per
    /// step), for settling into the resting cable.
    pub fn advance(
        &mut self,
        dt: f32,
        pins: Pins,
        length: f32,
        gravity: f32,
        shape: Option<&[Pos2]>,
        pull: f32,
    ) {
        self.spare = (self.spare + dt.max(0.0)).min(MAX_CATCH_UP);
        let target = shape.map(|s| resample(s, POINTS));
        while self.spare >= STEP {
            self.spare -= STEP;
            let n = self.pts.len();
            for i in 0..n {
                let p = self.pts[i];
                let v = (p - self.prev[i]) * DAMPING;
                self.prev[i] = p;
                self.pts[i] = p + v + Vec2::new(0.0, gravity * STEP * STEP);
                if let Some(t) = &target {
                    let here = self.pts[i];
                    self.pts[i] = here + (t[i] - here) * pull;
                }
            }
            let seg = (length / (n - 1) as f32).max(0.01);
            for _ in 0..PASSES {
                self.pts[0] = pins.start;
                if let Some(end) = pins.end {
                    self.pts[n - 1] = end;
                }
                let free = |i: usize| i != 0 && !(i == n - 1 && pins.end.is_some());
                // Length between neighbours, then a softer one two apart:
                // that resists sharp bends, so it curves like rope rather
                // than folding like thread.
                for (gap, rest, strength) in [(1, seg, 1.0), (2, seg * 1.96, BEND)] {
                    for i in 0..n - gap {
                        let j = i + gap;
                        let d = self.pts[j] - self.pts[i];
                        let len = d.length().max(1e-4);
                        if gap == 2 && len >= rest {
                            continue;
                        }
                        let fix = d * ((len - rest) / len) * strength;
                        match (free(i), free(j)) {
                            (true, true) => {
                                self.pts[i] += fix * 0.5;
                                self.pts[j] -= fix * 0.5;
                            }
                            (true, false) => self.pts[i] += fix,
                            (false, true) => self.pts[j] -= fix,
                            (false, false) => {}
                        }
                    }
                }
            }
            self.pts[0] = pins.start;
            if let Some(end) = pins.end {
                self.pts[n - 1] = end;
            }
        }
    }
}

/// Length along a polyline.
pub fn length(path: &[Pos2]) -> f32 {
    path.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// `n` points evenly spaced along `path`.
fn resample(path: &[Pos2], n: usize) -> Vec<Pos2> {
    if path.len() < 2 {
        return vec![*path.first().unwrap_or(&Pos2::ZERO); n];
    }
    let total = length(path).max(1e-4);
    let mut out = Vec::with_capacity(n);
    let mut seg = 0;
    let mut walked = 0.0;
    for k in 0..n {
        let want = total * k as f32 / (n - 1) as f32;
        while seg + 1 < path.len() - 1 && walked + path[seg].distance(path[seg + 1]) < want {
            walked += path[seg].distance(path[seg + 1]);
            seg += 1;
        }
        let len = path[seg].distance(path[seg + 1]).max(1e-4);
        let t = ((want - walked) / len).clamp(0.0, 1.0);
        out.push(path[seg] + (path[seg + 1] - path[seg]) * t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(a: Pos2, b: Pos2) -> Vec<Pos2> {
        vec![a, b]
    }

    /// A rope held at both ends sags under gravity and keeps its length;
    /// pulled towards a curve it settles onto it.
    #[test]
    fn a_rope_sags_keeps_its_length_and_settles() {
        let (a, b) = (Pos2::new(0.0, 0.0), Pos2::new(200.0, 0.0));
        let mut r = Rope::along(&line(a, b));
        let pins = Pins {
            start: a,
            end: Some(b),
        };
        for _ in 0..60 {
            r.advance(1.0 / 60.0, pins, 240.0, 1500.0, None, 0.0);
        }
        let mid = r.points()[POINTS / 2];
        assert!(mid.y > 40.0, "it sags: {mid:?}");
        assert!(
            (length(r.points()) - 240.0).abs() < 12.0,
            "keeps its length"
        );
        let target = line(a, b);
        for _ in 0..90 {
            r.advance(1.0 / 60.0, pins, 200.0, 0.0, Some(&target), 0.08);
        }
        assert!(
            r.points()[POINTS / 2].y.abs() < 2.0,
            "settles onto the curve"
        );
    }

    /// A free end swings down under gravity, and shortening the rope
    /// pulls it in to the held end.
    #[test]
    fn a_free_end_swings_and_winds_in() {
        let a = Pos2::new(0.0, 0.0);
        let mut r = Rope::along(&line(a, Pos2::new(150.0, 0.0)));
        let pins = Pins {
            start: a,
            end: None,
        };
        for _ in 0..30 {
            r.advance(1.0 / 60.0, pins, 150.0, 1500.0, None, 0.0);
        }
        assert!(r.end().y > 30.0, "swings down: {:?}", r.end());
        for k in 0..30 {
            let len = 150.0 * (1.0 - (k + 1) as f32 / 30.0);
            r.advance(1.0 / 60.0, pins, len, 1500.0, None, 0.0);
        }
        assert!(r.end().distance(a) < 5.0, "wound in: {:?}", r.end());
    }

    /// Design review: frames of a dragged cable, let go onto a port, and
    /// one winding in, as a PNG. `SYMPHONY_ROPE_SHEET=out.png cargo test
    /// rope_contact_sheet -- --ignored`.
    #[test]
    #[ignore = "design review"]
    fn rope_contact_sheet() {
        let Some(out) = std::env::var_os("SYMPHONY_ROPE_SHEET") else {
            return;
        };
        let (w, h) = (180u32, 260u32);
        let frames = 8;
        let mut img = image::RgbImage::from_pixel(w * frames, h * 2, image::Rgb([28, 29, 32]));
        let plot = |img: &mut image::RgbImage, pts: &[Pos2], ox: u32, oy: u32| {
            for s in pts.windows(2) {
                let steps = (s[0].distance(s[1]) * 2.0).ceil().max(1.0) as usize;
                for k in 0..=steps {
                    let p = s[0] + (s[1] - s[0]) * (k as f32 / steps as f32);
                    let (x, y) = (p.x as i64 + ox as i64, p.y as i64 + oy as i64);
                    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
                        img.put_pixel(x as u32, y as u32, image::Rgb([120, 210, 160]));
                    }
                }
            }
        };
        let fps = 60.0;
        // Row 1: dragged fast to the right, then let go onto a port below.
        let from = Pos2::new(40.0, 20.0);
        let port = Pos2::new(100.0, 220.0);
        let mut rope = Rope::along(&[from, from]);
        let mut frame = 0;
        for step in 0..70 {
            let t = step as f32 / fps;
            let (end, length) = if t < 0.35 {
                let p = Pos2::new(40.0 + 360.0 * t, 160.0);
                (p, from.distance(p) * DRAG_SLACK)
            } else {
                let k = ((t - 0.35) / 0.16).min(1.0);
                let start = Pos2::new(40.0 + 360.0 * 0.35, 160.0);
                let e = start + (port - start) * (1.0 - (1.0 - k).powi(3));
                (e, (from.distance(port) * 1.12).max(from.distance(e)))
            };
            rope.advance(
                1.0 / fps,
                Pins {
                    start: from,
                    end: Some(end),
                },
                length,
                GRAVITY,
                None,
                0.0,
            );
            if step % 9 == 0 && frame < frames {
                plot(&mut img, &rope.smooth(), frame * w, 0);
                frame += 1;
            }
        }
        // Row 2: disconnected, winding into the output.
        let mut rope = Rope::along(&[from, Pos2::new(140.0, 230.0)]);
        rope.kick(Vec2::new(-900.0, -300.0));
        let total = 230.0;
        for step in 0..28 {
            let p = step as f32 / 27.0;
            let left = total * (1.0 - p * p * (3.0 - 2.0 * p));
            rope.advance(
                1.0 / fps,
                Pins {
                    start: from,
                    end: None,
                },
                left,
                GRAVITY,
                None,
                0.0,
            );
            if step % 4 == 0 {
                plot(&mut img, &rope.smooth(), (step / 4) as u32 * w, h);
            }
        }
        img.save(out).unwrap();
    }
}
