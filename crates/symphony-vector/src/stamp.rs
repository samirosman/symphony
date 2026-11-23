//! Textured vector strokes (Harmony's Textured Vector Brush): stamp a tip
//! image along the stroke's centreline. A paper grain shapes each stamp as
//! it lands (Krita's texturing modes), fixed to the drawing so overlapping
//! stamps agree on where the paper is high.
//!
//! The stroke stays vector data. This turns it into coverage for a given
//! transform each time, so moving, scaling or reshaping the line re-samples
//! the texture rather than stretching pixels. The Camera view and export
//! both use this, so they match.

use symphony_core::math::Affine;
use symphony_core::stroke::{Stroke, StrokeTexture, Texture};

use crate::scale_of;

/// Coverage for one stroke: `data[y * width + x]` in 0..=255, placed at
/// (`x`, `y`) in output pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Coverage {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Largest coverage buffer, a side. Bigger strokes are clipped to `clip`.
const MAX_SIDE: i32 = 4096;
/// Most stamps in one stroke, so a hostile file can't stall drawing.
const MAX_STAMPS: usize = 200_000;

fn sample(t: &Texture, u: f32, v: f32) -> f32 {
    // Bilinear, clamped to the edge (tips) by the caller's range check.
    let (w, h) = (t.width as i32, t.height as i32);
    let (x, y) = (u - 0.5, v - 0.5);
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |xi: i32, yi: i32| -> f32 {
        let xi = xi.clamp(0, w - 1);
        let yi = yi.clamp(0, h - 1);
        t.pixels[(yi * w + xi) as usize] as f32
    };
    let top = px(x0, y0) * (1.0 - fx) + px(x0 + 1, y0) * fx;
    let bottom = px(x0, y0 + 1) * (1.0 - fx) + px(x0 + 1, y0 + 1) * fx;
    (top * (1.0 - fy) + bottom * fy) / 255.0
}

fn tiled(t: &Texture, u: f32, v: f32) -> f32 {
    let (w, h) = (t.width as f32, t.height as f32);
    let (x, y) = (u.rem_euclid(w), v.rem_euclid(h));
    let i = (y as u32).min(t.height - 1) * t.width + (x as u32).min(t.width - 1);
    t.pixels[i as usize] as f32 / 255.0
}

/// Coverage of a textured stroke under transform `m` (drawing -> output
/// pixels), limited to `clip` = [x0, y0, x1, y1] in output pixels.
pub fn rasterize(
    stroke: &Stroke,
    tex: &StrokeTexture,
    tip: Option<&Texture>,
    grain: Option<&Texture>,
    m: Affine,
    clip: [i32; 4],
) -> Option<Coverage> {
    let outline: Vec<Vec<_>> = stroke
        .shape
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|point| m.apply(point[0], point[1]))
                .collect()
        })
        .collect();
    let mut clip = clip;
    if !outline.is_empty() {
        m.inverse()?;
        // Avoid stamping pixels outside the surviving piece's bounding box.
        let [mut left, mut top, mut right, mut bottom] = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for &(x, y) in outline.iter().flatten() {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
        clip = [
            clip[0].max(left.floor() as i32),
            clip[1].max(top.floor() as i32),
            clip[2].min(right.ceil() as i32),
            clip[3].min(bottom.ceil() as i32),
        ];
        if clip[0] >= clip[2] || clip[1] >= clip[3] {
            return None;
        }
    }

    // Stamp along the same smooth curve that untextured lines follow.
    let step = 0.75 / scale_of(m).max(1e-6);
    let curved = Stroke {
        points: crate::curve::spline(&stroke.points, step),
        ..stroke.clone()
    };
    let mut s = Stamper::new(*tex, m, clip);
    s.extend(&curved, tip, grain);
    let mut c = s.coverage()?;
    if !outline.is_empty() {
        mask_coverage(&mut c, &outline);
    }
    Some(c)
}

/// Even–odd clipping at pixel centres. Intersect edges once per scanline,
/// then clear the gaps as spans instead of testing every individual pixel.
fn mask_coverage(c: &mut Coverage, outline: &[Vec<(f32, f32)>]) {
    let mut crossings = Vec::new();
    let width = c.width as usize;
    for (row, pixels) in c.data.chunks_exact_mut(width).enumerate() {
        let y = c.y as f32 + row as f32 + 0.5;
        crossings.clear();
        for ring in outline {
            let Some(mut previous) = ring.last().copied() else {
                continue;
            };
            for &next in ring {
                let ((ax, ay), (bx, by)) = (previous, next);
                if (ay > y) != (by > y) {
                    crossings.push(ax + (bx - ax) * ((y - ay) / (by - ay)));
                }
                previous = next;
            }
        }
        crossings.sort_unstable_by(f32::total_cmp);
        let pixel = |x: f32| ((x - c.x as f32 - 0.5).ceil() as usize).min(width);
        let mut start = 0;
        for span in crossings.as_chunks::<2>().0 {
            let left = pixel(span[0]);
            pixels[start..left].fill(0);
            start = pixel(span[1]);
        }
        pixels[start..].fill(0);
    }
}

/// Stamps a stroke a piece at a time, so the stroke being drawn only
/// stamps its new part each frame. Build one per transform.
pub struct Stamper {
    tex: StrokeTexture,
    m: Affine,
    k: f32,
    clip: [i32; 4],
    /// Coverage buffer and its bounds in output pixels.
    acc: Vec<f32>,
    /// The grain under each pixel (NaN until looked up), same layout.
    grain_at: Vec<f32>,
    bounds: [i32; 4],
    /// Points already stamped, and the distance left over to the next stamp.
    done: usize,
    carry: f32,
    /// The last point: position, diameter and pen pressure.
    last: Option<([f32; 2], f32, f32)>,
    stamps: usize,
}

impl Stamper {
    pub fn new(tex: StrokeTexture, m: Affine, clip: [i32; 4]) -> Self {
        Stamper {
            tex,
            m,
            k: scale_of(m),
            clip,
            acc: Vec::new(),
            grain_at: Vec::new(),
            bounds: [0, 0, 0, 0],
            done: 0,
            carry: 0.0,
            last: None,
            stamps: 0,
        }
    }

    /// Stamp the points of `stroke` not stamped yet. The stroke must only
    /// have grown since the last call (true while drawing).
    pub fn extend(&mut self, stroke: &Stroke, tip: Option<&Texture>, grain: Option<&Texture>) {
        let tip = tip.filter(|t| t.is_valid());
        let grain = grain.filter(|t| t.is_valid());
        let spacing = if self.tex.spacing.is_finite() {
            self.tex.spacing.clamp(0.01, 10.0)
        } else {
            0.1
        };
        let tex = self.tex;
        let k = self.k;
        // Krita's auto spacing: coefficient x sqrt(size), in drawing units.
        let auto = if tex.auto_spacing.is_finite() && tex.auto_spacing > 0.0 {
            tex.auto_spacing.min(10.0)
        } else {
            0.0
        };
        let gap = move |w: f32| -> f32 {
            if auto > 0.0 {
                let du = w / k.max(1e-6);
                auto * if du < 1.0 { du } else { du.sqrt() } * k
            } else {
                spacing * w
            }
        };
        let (lo, hi) = (stroke.min_width, stroke.max_width);
        let new: Vec<([f32; 2], f32, f32)> = stroke
            .points
            .iter()
            .skip(self.done)
            .map(|p| {
                let (x, y) = self.m.apply(p.x, p.y);
                (
                    [x, y],
                    ((lo + (hi - lo) * tex.size_at(p.pressure)) * k).max(0.0),
                    p.pressure,
                )
            })
            .collect();
        self.done = stroke.points.len();
        for (c, d, pr) in new {
            if !(c[0].is_finite() && c[1].is_finite() && d.is_finite()) {
                continue;
            }
            let Some((a, wa, pa)) = self.last else {
                self.stamp(c, d, 0.0, pr, tip, grain);
                self.last = Some((c, d, pr));
                continue;
            };
            let (dx, dy) = (c[0] - a[0], c[1] - a[1]);
            let len = dx.hypot(dy);
            if len <= 1e-4 {
                continue;
            }
            let heading = dy.atan2(dx);
            let mut t = self.carry;
            while t <= len {
                let f = t / len;
                let w = wa + (d - wa) * f;
                let p = pa + (pr - pa) * f;
                self.stamp([a[0] + dx * f, a[1] + dy * f], w, heading, p, tip, grain);
                t += gap(w).max(0.5);
            }
            self.carry = t - len;
            self.last = Some((c, d, pr));
        }
    }

    /// Make sure the buffer covers [x0, y0, x1, y1] (within the clip).
    fn grow(&mut self, want: [i32; 4]) -> bool {
        let want = [
            want[0].max(self.clip[0]),
            want[1].max(self.clip[1]),
            want[2].min(self.clip[2]),
            want[3].min(self.clip[3]),
        ];
        if want[2] <= want[0] || want[3] <= want[1] {
            return false;
        }
        let b = self.bounds;
        let empty = b[2] <= b[0];
        if !empty && want[0] >= b[0] && want[1] >= b[1] && want[2] <= b[2] && want[3] <= b[3] {
            return true;
        }
        // Grow with room to spare, so a stroke in progress rarely copies.
        let margin = 256;
        let nb = if empty {
            [
                want[0] - margin,
                want[1] - margin,
                want[2] + margin,
                want[3] + margin,
            ]
        } else {
            [
                if want[0] < b[0] {
                    want[0] - margin
                } else {
                    b[0]
                },
                if want[1] < b[1] {
                    want[1] - margin
                } else {
                    b[1]
                },
                if want[2] > b[2] {
                    want[2] + margin
                } else {
                    b[2]
                },
                if want[3] > b[3] {
                    want[3] + margin
                } else {
                    b[3]
                },
            ]
        };
        let nb = [
            nb[0].max(self.clip[0]),
            nb[1].max(self.clip[1]),
            nb[2]
                .min(self.clip[2])
                .min(nb[0].max(self.clip[0]) + MAX_SIDE),
            nb[3]
                .min(self.clip[3])
                .min(nb[1].max(self.clip[1]) + MAX_SIDE),
        ];
        let (nw, nh) = (
            (nb[2] - nb[0]).max(0) as usize,
            (nb[3] - nb[1]).max(0) as usize,
        );
        let mut acc = vec![0.0f32; nw * nh];
        let mut grain_at = vec![f32::NAN; nw * nh];
        if !empty {
            let ow = (b[2] - b[0]) as usize;
            for y in b[1].max(nb[1])..b[3].min(nb[3]) {
                for x in b[0].max(nb[0])..b[2].min(nb[2]) {
                    let to = (y - nb[1]) as usize * nw + (x - nb[0]) as usize;
                    let from = (y - b[1]) as usize * ow + (x - b[0]) as usize;
                    acc[to] = self.acc[from];
                    grain_at[to] = self.grain_at[from];
                }
            }
        }
        self.acc = acc;
        self.grain_at = grain_at;
        self.bounds = nb;
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn stamp(
        &mut self,
        c: [f32; 2],
        d: f32,
        heading: f32,
        pressure: f32,
        tip: Option<&Texture>,
        grain: Option<&Texture>,
    ) {
        self.stamps += 1;
        // Each stamp raises pixels toward its own opacity (faded by
        // pressure) and never past it; flow is how far each step goes.
        // This is how Krita's and Harmony's brushes combine dabs, so soft
        // edges stay soft and low flow builds up gently.
        let clean = |v: f32, d: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { d };
        let level = clean(self.tex.opacity_at(pressure), 1.0);
        let flow = clean(self.tex.flow_at(pressure), 1.0);
        let strength = clean(self.tex.grain_strength_at(pressure), 1.0);
        let mode = self.tex.grain_mode;
        let gscale = if self.tex.grain_scale.is_finite() && self.tex.grain_scale > 0.0 {
            self.tex.grain_scale.clamp(0.01, 100.0)
        } else {
            1.0
        };
        let goff = self
            .tex
            .grain_offset
            .map(|v| if v.is_finite() { v } else { 0.0 });
        let inv = grain.and_then(|_| self.m.inverse());
        if level <= 0.0 || flow <= 0.0 {
            return;
        }
        // Soft edge width for the round tip: a hard tip keeps 1 pixel of
        // anti-aliasing, a soft one fades over part of its radius.
        let soft = (d * 0.5 * (1.0 - clean(self.tex.hardness, 1.0))).max(1.0);
        if d < 0.3 || self.stamps > MAX_STAMPS {
            return;
        }
        let r = d * 0.5;
        let angle = self.tex.angle.to_radians() + if self.tex.follow { heading } else { 0.0 };
        let (sin, cos) = angle.sin_cos();
        // Exact half-size of the (rotated) tip square, or the round tip.
        let reach = if tip.is_some() {
            r * (cos.abs() + sin.abs()) + 1.0
        } else {
            r + 1.0
        };
        let want = [
            (c[0] - reach).floor() as i32,
            (c[1] - reach).floor() as i32,
            (c[0] + reach).ceil() as i32,
            (c[1] + reach).ceil() as i32,
        ];
        if !self.grow(want) {
            return;
        }
        let b = self.bounds;
        let w = (b[2] - b[0]) as usize;
        let (sx0, sy0) = (want[0].max(b[0]), want[1].max(b[1]));
        let (sx1, sy1) = (want[2].min(b[2]), want[3].min(b[3]));
        let inv_r = 1.0 / r;
        for py in sy0..sy1 {
            let dy = py as f32 + 0.5 - c[1];
            let row = (py - b[1]) as usize * w;
            for px in sx0..sx1 {
                let dx = px as f32 + 0.5 - c[0];
                let a = match tip {
                    Some(t) => {
                        // Into the tip's own frame, -1..1 across, the
                        // longer side fitted to the diameter.
                        let u = (dx * cos + dy * sin) * inv_r;
                        let v = (dy * cos - dx * sin) * inv_r;
                        if u.abs() >= 1.0 || v.abs() >= 1.0 {
                            continue;
                        }
                        let side = t.width.max(t.height) as f32;
                        let tu = (u + 1.0) * 0.5 * side - (side - t.width as f32) * 0.5;
                        let tv = (v + 1.0) * 0.5 * side - (side - t.height as f32) * 0.5;
                        if tu < 0.0 || tv < 0.0 || tu >= t.width as f32 || tv >= t.height as f32 {
                            continue;
                        }
                        if d >= side {
                            sample(t, tu, tv)
                        } else {
                            // Shrinking the tip: nearest is enough and faster.
                            t.pixels[tv as usize * t.width as usize + tu as usize] as f32 / 255.0
                        }
                    }
                    // A hard round tip with one pixel of anti-aliasing.
                    None => ((r - (dx * dx + dy * dy).sqrt()) / soft + 0.5).clamp(0.0, 1.0),
                };
                if a > 0.0 {
                    let i = row + (px - b[0]) as usize;
                    // The grain shapes the stamp before it lands (Krita
                    // applies its texture to each dab).
                    let a = match (grain, inv) {
                        (Some(g), Some(inv)) => {
                            let mut m = self.grain_at[i];
                            if m.is_nan() {
                                let (gx, gy) = inv.apply(px as f32 + 0.5, py as f32 + 0.5);
                                m = tiled(g, gx * gscale + goff[0], gy * gscale + goff[1]);
                                self.grain_at[i] = m;
                            }
                            mode.apply(a, m, strength)
                        }
                        _ => a,
                    };
                    if a <= 0.0 {
                        continue;
                    }
                    let target = a * level;
                    if target > self.acc[i] {
                        self.acc[i] += (target - self.acc[i]) * flow;
                    }
                }
            }
        }
    }

    /// The coverage so far, with the stroke's opacity cap.
    pub fn coverage(&self) -> Option<Coverage> {
        let b = self.bounds;
        if b[2] <= b[0] || b[3] <= b[1] {
            return None;
        }
        // Trim to the painted area.
        let w = (b[2] - b[0]) as usize;
        let h = (b[3] - b[1]) as usize;
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
        for y in 0..h {
            for x in 0..w {
                if self.acc[y * w + x] > 0.0 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let opacity = if self.tex.opacity.is_finite() {
            self.tex.opacity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let (cw, ch) = (x1 - x0, y1 - y0);
        let mut data = vec![0u8; cw * ch];
        for y in 0..ch {
            for x in 0..cw {
                let a = self.acc[(y + y0) * w + x + x0];
                if a <= 0.0 {
                    continue;
                }
                let v = a * opacity;
                data[y * cw + x] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        Some(Coverage {
            x: b[0] + x0 as i32,
            y: b[1] + y0 as i32,
            width: cw as u32,
            height: ch as u32,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphony_core::id::Id;
    use symphony_core::stroke::{Point, StrokeKind};

    fn line(width: f32) -> Stroke {
        Stroke {
            id: Id(1),
            kind: StrokeKind::Brush,
            art: Default::default(),
            swatch: Id(0),
            min_width: width,
            max_width: width,
            points: [(10.0, 20.0), (90.0, 20.0)]
                .iter()
                .map(|&(x, y)| Point {
                    x,
                    y,
                    pressure: 1.0,
                })
                .collect(),
            tips: Default::default(),
            texture: None,
            shape: Vec::new(),
        }
    }

    fn tex() -> StrokeTexture {
        StrokeTexture {
            tip: None,
            grain: None,
            spacing: 0.1,
            opacity: 1.0,
            angle: 0.0,
            follow: false,
            grain_scale: 1.0,
            flow: 1.0,
            hardness: 1.0,
            opacity_by_pressure: [1.0; 4],
            ..Default::default()
        }
    }

    const ALL: [i32; 4] = [-10_000, -10_000, 10_000, 10_000];

    fn at(c: &Coverage, x: i32, y: i32) -> u8 {
        let (cx, cy) = (x - c.x, y - c.y);
        if cx < 0 || cy < 0 || cx >= c.width as i32 || cy >= c.height as i32 {
            return 0;
        }
        c.data[(cy * c.width as i32 + cx) as usize]
    }

    #[test]
    fn scanline_mask_matches_point_sampling_for_rotated_outline_with_hole() {
        let local = vec![
            vec![[-9.2, -7.1], [8.3, -7.1], [8.3, 6.2], [-9.2, 6.2]],
            vec![[-4.1, -3.2], [4.4, -3.2], [4.4, 2.3], [-4.1, 2.3]],
        ];
        let transform = Affine::translate(16.4, 20.7).compose(Affine::rotate_deg(27.0));
        let inverse = transform.inverse().unwrap();
        let outline: Vec<_> = local
            .iter()
            .map(|ring| ring.iter().map(|[x, y]| transform.apply(*x, *y)).collect())
            .collect();
        let mut coverage = Coverage {
            x: 1,
            y: 3,
            width: 32,
            height: 34,
            data: vec![177; 32 * 34],
        };
        mask_coverage(&mut coverage, &outline);
        for y in 0..coverage.height {
            for x in 0..coverage.width {
                let (lx, ly) = inverse.apply(
                    coverage.x as f32 + x as f32 + 0.5,
                    coverage.y as f32 + y as f32 + 0.5,
                );
                let expected = if crate::shape::hits_shape(&local, lx, ly, 0.0) {
                    177
                } else {
                    0
                };
                assert_eq!(coverage.data[(y * coverage.width + x) as usize], expected);
            }
        }
    }

    #[test]
    fn round_tip_covers_the_line_and_nothing_far_away() {
        let c = rasterize(&line(8.0), &tex(), None, None, Affine::IDENTITY, ALL).unwrap();
        assert_eq!(at(&c, 50, 20), 255);
        assert_eq!(at(&c, 50, 27), 0);
    }

    #[test]
    fn subtract_grain_shapes_each_stamp_like_krita() {
        use symphony_core::stroke::GrainMode;
        let t = StrokeTexture {
            grain_mode: GrainMode::Subtract,
            ..tex()
        };
        // White grain removes all paint, black keeps it all (strength 1).
        let grain = |v: u8| Texture {
            width: 4,
            height: 4,
            pixels: vec![v; 16],
        };
        let white = rasterize(
            &line(8.0),
            &t,
            None,
            Some(&grain(255)),
            Affine::IDENTITY,
            ALL,
        );
        assert!(white.is_none_or(|c| c.data.iter().all(|&v| v == 0)));
        let black =
            rasterize(&line(8.0), &t, None, Some(&grain(0)), Affine::IDENTITY, ALL).unwrap();
        assert_eq!(at(&black, 50, 20), 255);
        // Mid grey at strength 1 leaves half of each stamp.
        let grey = rasterize(
            &line(8.0),
            &t,
            None,
            Some(&grain(128)),
            Affine::IDENTITY,
            ALL,
        )
        .unwrap();
        assert!(
            (120..=135).contains(&at(&grey, 50, 20)),
            "{}",
            at(&grey, 50, 20)
        );
    }

    #[test]
    fn opacity_caps_and_grain_multiplies() {
        let mut t = tex();
        t.opacity = 0.5;
        let half = rasterize(&line(8.0), &t, None, None, Affine::IDENTITY, ALL).unwrap();
        assert_eq!(at(&half, 50, 20), 128);
        // A grain with alternating black and white columns.
        let g = Texture {
            width: 2,
            height: 1,
            pixels: vec![0, 255],
        };
        t.opacity = 1.0;
        let grained = rasterize(&line(8.0), &t, None, Some(&g), Affine::IDENTITY, ALL).unwrap();
        assert_eq!(at(&grained, 50, 20), 0);
        assert_eq!(at(&grained, 51, 20), 255);
    }

    #[test]
    fn transforms_resample_instead_of_stretching() {
        // Twice the scale gives twice the coverage width, still solid.
        let c = rasterize(&line(8.0), &tex(), None, None, Affine::scale(2.0, 2.0), ALL).unwrap();
        assert_eq!(at(&c, 100, 40), 255);
        // The last pixel inside the edge is solid, give or take
        // anti-aliasing.
        assert!(at(&c, 100, 47) >= 245, "{}", at(&c, 100, 47));
        assert_eq!(at(&c, 100, 50), 0);
    }

    #[test]
    fn stamping_in_pieces_matches_stamping_at_once() {
        let mut long = line(8.0);
        long.points = (0..=40)
            .map(|i| symphony_core::stroke::Point {
                x: i as f32 * 25.0,
                y: (i as f32 * 0.4).sin() * 30.0,
                pressure: 1.0,
            })
            .collect();
        let mut at_once = Stamper::new(tex(), Affine::IDENTITY, ALL);
        at_once.extend(&long, None, None);
        let whole = at_once.coverage().unwrap();
        let mut s = Stamper::new(tex(), Affine::IDENTITY, ALL);
        let mut part = long.clone();
        for n in [1, 5, 6, 20, 41] {
            part.points = long.points[..n].to_vec();
            s.extend(&part, None, None);
        }
        assert_eq!(s.coverage().unwrap(), whole);
    }

    #[test]
    fn flow_builds_up_and_pressure_fades() {
        // Low flow: one stamp is faint, many overlapping ones build up.
        let mut t = tex();
        t.flow = 0.1;
        t.spacing = 0.05;
        let c = rasterize(&line(8.0), &t, None, None, Affine::IDENTITY, ALL).unwrap();
        let middle = at(&c, 50, 20);
        assert!(middle > 200, "built up: {middle}");
        let end = at(&c, 7, 20);
        assert!(end < middle, "fainter where fewer stamps overlap: {end}");
        // Opacity following pressure: light pressure lays down less.
        let mut t = tex();
        t.opacity_by_pressure = [0.0, 0.33, 0.66, 1.0];
        let mut light = line(8.0);
        for p in &mut light.points {
            p.pressure = 0.2;
        }
        light.min_width = 8.0;
        let faint = rasterize(&light, &t, None, None, Affine::IDENTITY, ALL).unwrap();
        let strong = rasterize(&line(8.0), &t, None, None, Affine::IDENTITY, ALL).unwrap();
        assert!(at(&faint, 50, 20) < at(&strong, 50, 20));
    }

    #[test]
    fn soft_round_tips_fade_at_the_edge() {
        let mut t = tex();
        t.hardness = 0.2;
        let soft = rasterize(&line(16.0), &t, None, None, Affine::IDENTITY, ALL).unwrap();
        let hard = rasterize(&line(16.0), &tex(), None, None, Affine::IDENTITY, ALL).unwrap();
        // Near the edge the soft line is partly transparent.
        assert_eq!(at(&hard, 50, 26), 255);
        assert!(at(&soft, 50, 26) < 255);
    }

    #[test]
    fn clip_and_bad_textures_are_safe() {
        let bad = Texture {
            width: 3,
            height: 3,
            pixels: vec![1],
        };
        let mut t = tex();
        t.spacing = f32::NAN;
        let c = rasterize(
            &line(8.0),
            &t,
            Some(&bad),
            Some(&bad),
            Affine::IDENTITY,
            [40, 0, 60, 100],
        )
        .unwrap();
        assert!(c.x >= 40 && c.x + c.width as i32 <= 60);
        assert!(rasterize(
            &line(8.0),
            &t,
            None,
            None,
            Affine::IDENTITY,
            [500, 500, 600, 600]
        )
        .is_none());
    }
}
