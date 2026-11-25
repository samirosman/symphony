//! Bending an imported image with a bone chain or a Mesh Warp: the image
//! is cut into a triangle mesh, each vertex is moved by the bend, and each
//! triangle is
//! filled with its own piece of the image.
//!
//! The mesh is an adaptive quadtree. Cells start at
//! [`MeshDetail::largest`] and split in four while the bend moves their
//! inside more than [`MeshDetail::tolerance`] output pixels away from a
//! flat cell, down to [`MeshDetail::smallest`]. A cell next to smaller
//! cells takes their corners on its edges and becomes a fan around its
//! centre, so the mesh has no T-junctions and no cracks.
//!
//! Triangles are filled here, not by tiny-skia: an anti-aliased edge
//! inside the image would show as a hairline seam where two triangles
//! meet. Vertices are snapped to 1/256 pixel and edges follow a top-left
//! rule, so every pixel on a shared edge is drawn exactly once. The outer
//! edge is anti-aliased apart from that: the triangles are filled a pixel
//! past the image's border, and the result is multiplied by the coverage
//! of the whole mesh, which tiny-skia fills as one path. Inside that path
//! the coverage is full, so the inner edges keep no seam.

use std::collections::{HashMap, HashSet};

use symphony_core::deform::MeshDetail;
use symphony_core::doc::Scene;
use symphony_core::math::Affine;
use symphony_core::stroke::PlacedImage;
use tiny_skia::{FillRule, Mask, PathBuilder, Pixmap, Transform};

use crate::bitmap;

/// Triangles over an image: where each vertex lands (output pixels) and
/// which image point it shows (image pixels, 0 to the width).
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub points: Vec<[f32; 2]>,
    pub uv: Vec<[f32; 2]>,
    pub triangles: Vec<[u32; 3]>,
}

/// Draw the drawing's image with every point moved by `bend` (drawing
/// space, as [`symphony_core::warp::Warp::bend`] or
/// [`ChainPose::map`]), then `m` (to pixels). Returns the mesh it used,
/// for display and tests.
pub fn paint_image_bent(
    pix: &mut Pixmap,
    scene: &Scene,
    r: PlacedImage,
    m: Affine,
    bend: impl Fn([f32; 2]) -> [f32; 2],
    detail: &MeshDetail,
) -> Option<Mesh> {
    let img = bitmap::cached(scene, r)?;
    let [x0, y0, x1, y1] = bitmap::corners(r);
    // The same cap as the plain image, so a bend never changes its size.
    let m = bitmap::cap_zoom(m, [x1 - x0, y1 - y0], pix);
    let (iw, ih) = (img.width(), img.height());
    let (kx, ky) = ((x1 - x0) / iw as f32, (y1 - y0) / ih as f32);
    let mesh = build(
        iw,
        ih,
        |[u, v]| {
            let q = bend([x0 + u * kx, y0 + v * ky]);
            let (x, y) = m.apply(q[0], q[1]);
            [x, y]
        },
        detail,
    );
    draw(pix, &img, &mesh);
    Some(mesh)
}

/// Lattice steps, the smallest cell, at most 2^10 to a largest cell.
const MAX_LEVELS: u32 = 10;

/// The mesh for a `w` x `h` image whose point `uv` lands at `to_out(uv)`.
pub fn build(w: u32, h: u32, to_out: impl Fn([f32; 2]) -> [f32; 2], detail: &MeshDetail) -> Mesh {
    let mut mesh = Mesh::default();
    if w == 0 || h == 0 {
        return mesh;
    }
    let (w, h) = (w as f32, h as f32);
    let small = finite_or(detail.smallest, 2.0).max(0.5);
    let large = finite_or(detail.largest, 64.0).max(small);
    let tol = finite_or(detail.tolerance, 0.25).max(0.01);
    let levels = ((large / small).log2().floor() as u32).min(MAX_LEVELS);
    let root = 1u32 << levels;
    // Lattice points past the image's far edges sit on those edges.
    let (nx, ny) = ((w / small).ceil() as u32, (h / small).ceil() as u32);
    let at = |x: u32, y: u32| [(x as f32 * small).min(w), (y as f32 * small).min(h)];

    // Split cells while the map bends them.
    let mut leaves = Vec::new();
    let mut stack: Vec<(u32, u32, u32)> = Vec::new();
    for y in (0..ny).step_by(root as usize) {
        for x in (0..nx).step_by(root as usize) {
            stack.push((x, y, root));
        }
    }
    while let Some((x, y, n)) = stack.pop() {
        if n > 1 && bent(&at, &to_out, x, y, n, tol) {
            let k = n / 2;
            stack.extend([(x, y, k), (x + k, y, k), (x, y + k, k), (x + k, y + k, k)]);
        } else {
            leaves.push((x, y, n));
        }
    }

    // Cells wholly past the image's far edges hold nothing.
    leaves.retain(|&(x, y, _)| x < nx && y < ny);
    let corners: HashSet<(u32, u32)> = leaves
        .iter()
        .flat_map(|&(x, y, n)| [(x, y), (x + n, y), (x, y + n), (x + n, y + n)])
        .collect();
    // Keyed by image position: lattice points past the far edges land on
    // them, and must share one vertex.
    let mut index: HashMap<(u32, u32), u32> = HashMap::new();
    let mut vertex = |mesh: &mut Mesh, uv: [f32; 2]| -> u32 {
        *index
            .entry((uv[0].to_bits(), uv[1].to_bits()))
            .or_insert_with(|| {
                mesh.uv.push(uv);
                mesh.points.push(to_out(uv));
                (mesh.uv.len() - 1) as u32
            })
    };
    let mut ring: Vec<u32> = Vec::new();
    for &(x, y, n) in &leaves {
        // The cell's outline, with every neighbour's corner on its edges.
        ring.clear();
        let edges = [
            (x, y, 1i32, 0i32),
            (x + n, y, 0, 1),
            (x + n, y + n, -1, 0),
            (x, y + n, 0, -1),
        ];
        for (sx, sy, dx, dy) in edges {
            for i in 0..n as i32 {
                let (px, py) = ((sx as i32 + dx * i) as u32, (sy as i32 + dy * i) as u32);
                if i == 0 || corners.contains(&(px, py)) {
                    let v = vertex(&mut mesh, at(px, py));
                    if ring.last() != Some(&v) {
                        ring.push(v);
                    }
                }
            }
        }
        if ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        let push = |mesh: &mut Mesh, t: [u32; 3]| {
            let [a, b, c] = t.map(|i| mesh.uv[i as usize]);
            if ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() > 1e-6 {
                mesh.triangles.push(t);
            }
        };
        match ring.len() {
            0..=2 => {}
            3 => push(&mut mesh, [ring[0], ring[1], ring[2]]),
            4 => {
                push(&mut mesh, [ring[0], ring[1], ring[2]]);
                push(&mut mesh, [ring[0], ring[2], ring[3]]);
            }
            _ => {
                let (a, b) = (at(x, y), at(x + n, y + n));
                let c = vertex(&mut mesh, [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]);
                for i in 0..ring.len() {
                    push(&mut mesh, [c, ring[i], ring[(i + 1) % ring.len()]]);
                }
            }
        }
    }
    mesh
}

fn finite_or(v: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        fallback
    }
}

/// Does the map move the inside of cell (`x`, `y`, side `n`) more than
/// `tol` away from the flat cell through its mapped corners? Checked on a
/// 5 x 5 grid, so a joint zone smaller than the cell is not missed.
fn bent(
    at: &impl Fn(u32, u32) -> [f32; 2],
    to_out: &impl Fn([f32; 2]) -> [f32; 2],
    x: u32,
    y: u32,
    n: u32,
    tol: f32,
) -> bool {
    let (a, b) = (at(x, y), at(x + n, y + n));
    if a[0] >= b[0] || a[1] >= b[1] {
        return false;
    }
    let c = [
        to_out(a),
        to_out([b[0], a[1]]),
        to_out([a[0], b[1]]),
        to_out(b),
    ];
    for j in 0..=4 {
        for i in 0..=4 {
            if (i == 0 || i == 4) && (j == 0 || j == 4) {
                continue;
            }
            let (s, t) = (i as f32 / 4.0, j as f32 / 4.0);
            let uv = [a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * t];
            let flat = bilerp(c, s, t);
            let real = to_out(uv);
            if (real[0] - flat[0]).hypot(real[1] - flat[1]) > tol {
                return true;
            }
        }
    }
    false
}

fn bilerp(c: [[f32; 2]; 4], s: f32, t: f32) -> [f32; 2] {
    let top = [
        c[0][0] + (c[1][0] - c[0][0]) * s,
        c[0][1] + (c[1][1] - c[0][1]) * s,
    ];
    let bot = [
        c[2][0] + (c[3][0] - c[2][0]) * s,
        c[2][1] + (c[3][1] - c[2][1]) * s,
    ];
    [
        top[0] + (bot[0] - top[0]) * t,
        top[1] + (bot[1] - top[1]) * t,
    ]
}

/// Sub-pixel steps for vertex positions.
const SUB: i64 = 256;

/// How far past the image's border the fill reaches, in output pixels,
/// so the anti-aliased edge has colour to fade.
const MARGIN: i64 = 1;

/// Larger output coordinates than this leave the edge hard: tiny-skia
/// refuses paths it cannot rasterize exactly.
const MAX_PATH: f32 = 1e6;

/// The widest or tallest triangle drawn, in output pixels. The edge
/// maths multiplies two sub-pixel distances in i64, which overflows past
/// about 8.4 million pixels. The zoom cap keeps an image far below this,
/// but a stretched bone can still pull a triangle that far; it is
/// dropped, not drawn wrong.
const MAX_TRIANGLE: i64 = 1 << 22;

/// Fill each triangle of `mesh` with its piece of `src` (premultiplied),
/// source-over onto `pix`, with an anti-aliased outer edge.
pub fn draw(pix: &mut Pixmap, src: &Pixmap, mesh: &Mesh) {
    let (pw, ph) = (pix.width() as i64, pix.height() as i64);
    let fixed: Vec<[i64; 2]> = mesh
        .points
        .iter()
        .map(|p| {
            let f = |v: f32| (v * SUB as f32).round().clamp(-1e15, 1e15) as i64;
            [f(p[0]), f(p[1])]
        })
        .collect();
    // Triangles wound one way; degenerate and oversized ones dropped.
    let tris: Vec<[usize; 3]> = mesh
        .triangles
        .iter()
        .filter_map(|t| {
            let [a, b, c] = t.map(|i| i as usize);
            let too_big = (0..2).any(|k| {
                let v = [fixed[a][k], fixed[b][k], fixed[c][k]];
                v.iter().max().unwrap() - v.iter().min().unwrap() > MAX_TRIANGLE * SUB
            });
            if too_big {
                return None;
            }
            match edge(fixed[a], fixed[b], fixed[c]) {
                0 => None,
                n if n < 0 => Some([a, c, b]),
                _ => Some([a, b, c]),
            }
        })
        .collect();
    if tris.is_empty() {
        return;
    }
    // The pixels any triangle (with its margin) can touch.
    let reach = (MARGIN + 2) * SUB;
    let (mut lo, mut hi) = ([i64::MAX; 2], [i64::MIN; 2]);
    for &i in tris.iter().flatten() {
        for k in 0..2 {
            lo[k] = lo[k].min(fixed[i][k]);
            hi[k] = hi[k].max(fixed[i][k]);
        }
    }
    let bx0 = ((lo[0] - reach).div_euclid(SUB)).max(0);
    let by0 = ((lo[1] - reach).div_euclid(SUB)).max(0);
    let bx1 = ((hi[0] + reach).div_euclid(SUB)).min(pw - 1);
    let by1 = ((hi[1] + reach).div_euclid(SUB)).min(ph - 1);
    if bx0 > bx1 || by0 > by1 {
        return;
    }
    let (bw, bh) = (bx1 - bx0 + 1, by1 - by0 + 1);
    let cover = coverage(&fixed, &tris, bx0, by0, bw, bh);
    // Fully covered pixels go straight to `pix`. The outline's pixels are
    // gathered here first, then faded by their coverage. Only those are
    // written, so most of these zeroed pages are never touched.
    let mut layer = vec![[0u8; 4]; (bw * bh) as usize];
    // 0: empty, 1: margin past the border, 2: inside a triangle.
    let mut state = vec![0u8; (bw * bh) as usize];
    let mut outline: Vec<usize> = Vec::new();
    let data = pix.data_mut();

    let (sw, sh) = (src.width() as f32, src.height() as f32);
    let on_border = |p: [f32; 2], q: [f32; 2]| {
        (p[0] == q[0] && (p[0] == 0.0 || p[0] == sw))
            || (p[1] == q[1] && (p[1] == 0.0 || p[1] == sh))
    };
    for &[ia, ib, ic] in &tris {
        let (a, b, c) = (fixed[ia], fixed[ib], fixed[ic]);
        let area = edge(a, b, c) as f32;
        let (ta, tb, tc) = (mesh.uv[ia], mesh.uv[ib], mesh.uv[ic]);
        let edges = [(b, c), (c, a), (a, b)];
        let bias = edges.map(|(p, q)| i64::from(!top_left(p, q)));
        // A border edge may be crossed by up to MARGIN pixels.
        let border = [(tb, tc), (tc, ta), (ta, tb)].map(|(p, q)| on_border(p, q));
        let slack: [i64; 3] = std::array::from_fn(|k| {
            if !border[k] {
                return 0;
            }
            let (p, q) = edges[k];
            let len = ((q[0] - p[0]) as f64).hypot((q[1] - p[1]) as f64);
            (len * (MARGIN * SUB) as f64) as i64
        });
        let pad = if border.contains(&true) {
            MARGIN * SUB
        } else {
            0
        };
        let lo = |k: usize| a[k].min(b[k]).min(c[k]) - pad;
        let hi = |k: usize| a[k].max(b[k]).max(c[k]) + pad;
        // Pixels whose centre (x + 0.5) can be inside.
        let x0 = ((lo(0) - SUB / 2).div_euclid(SUB)).max(bx0);
        let x1 = ((hi(0) - SUB / 2).div_euclid(SUB) + 1).min(bx1);
        let y0 = ((lo(1) - SUB / 2).div_euclid(SUB)).max(by0);
        let y1 = ((hi(1) - SUB / 2).div_euclid(SUB) + 1).min(by1);
        for py in y0..=y1 {
            let cy = py * SUB + SUB / 2;
            for px in x0..=x1 {
                let p = [px * SUB + SUB / 2, cy];
                let e = edges.map(|(s, q)| edge(s, q, p));
                // Exactly on an edge counts only for top or left edges.
                let inside = (0..3).all(|k| e[k] - bias[k] >= 0);
                if !inside && (0..3).any(|k| e[k] - bias[k] < -slack[k]) {
                    continue;
                }
                let i = ((py - by0) * bw + (px - bx0)) as usize;
                let m = match &cover {
                    Some(mask) => mask.data()[i],
                    None if inside => 255,
                    None => 0,
                };
                if m == 0 || (!inside && state[i] != 0) {
                    continue;
                }
                let (la, lb, lc) = (e[0] as f32 / area, e[1] as f32 / area, e[2] as f32 / area);
                let u = la * ta[0] + lb * tb[0] + lc * tc[0];
                let v = la * ta[1] + lb * tb[1] + lc * tc[1];
                let s = sample(src, u, v);
                if m == 255 && inside {
                    // A fold draws here twice, the image over itself.
                    let o = ((py * pw + px) * 4) as usize;
                    over(&mut data[o..o + 4], s);
                } else if inside && state[i] == 2 {
                    over(&mut layer[i], s);
                } else {
                    if state[i] == 0 {
                        outline.push(i);
                    }
                    layer[i] = s;
                    state[i] = if inside { 2 } else { 1 };
                }
            }
        }
    }

    let Some(mask) = cover else {
        return;
    };
    for i in outline {
        let m = mask.data()[i] as u32;
        let s = layer[i].map(|c| ((c as u32 * m + 127) / 255) as u8);
        let (x, y) = (i as i64 % bw, i as i64 / bw);
        let o = (((by0 + y) * pw + bx0 + x) * 4) as usize;
        over(&mut data[o..o + 4], s);
    }
}

/// How much of each pixel of the `bw` x `bh` box at (`bx0`, `by0`) the
/// triangles cover together, or `None` if tiny-skia cannot fill them.
fn coverage(
    fixed: &[[i64; 2]],
    tris: &[[usize; 3]],
    bx0: i64,
    by0: i64,
    bw: i64,
    bh: i64,
) -> Option<Mask> {
    let at = |i: usize| {
        let x = (fixed[i][0] - bx0 * SUB) as f32 / SUB as f32;
        let y = (fixed[i][1] - by0 * SUB) as f32 / SUB as f32;
        (x.abs() <= MAX_PATH && y.abs() <= MAX_PATH).then_some((x, y))
    };
    let mut pb = PathBuilder::new();
    for ring in rings(tris) {
        let (x, y) = at(ring[0])?;
        pb.move_to(x, y);
        for &i in &ring[1..] {
            let (x, y) = at(i)?;
            pb.line_to(x, y);
        }
        pb.close();
    }
    let path = pb.finish()?;
    let mut mask = Mask::new(bw as u32, bh as u32)?;
    // One winding over the whole mesh: the coverage is full everywhere
    // inside and partial only on the outline.
    mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
    Some(mask)
}

/// The edges of `tris` (all wound one way) that do not cancel, as closed
/// rings of vertex indices. An inner edge is walked once each way by its
/// two triangles, so the pair adds nothing to the winding and is left
/// out; what is left is the outline, plus the edges of folds, where two
/// triangles walk an edge the same way. The rings give the same winding
/// as all the triangles, with far fewer edges for tiny-skia to fill.
fn rings(tris: &[[usize; 3]]) -> Vec<Vec<usize>> {
    let mut count: HashMap<(usize, usize), u32> = HashMap::new();
    for t in tris {
        for (p, q) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            match count.get_mut(&(q, p)) {
                Some(n) if *n > 0 => *n -= 1,
                _ => *count.entry((p, q)).or_insert(0) += 1,
            }
        }
    }
    let mut edges: Vec<(usize, usize)> = count
        .into_iter()
        .flat_map(|(e, n)| std::iter::repeat_n(e, n as usize))
        .collect();
    // Sorted, so the rings come out the same on every run.
    edges.sort_unstable();
    let mut next: HashMap<usize, Vec<usize>> = HashMap::new();
    for &(p, q) in edges.iter().rev() {
        next.entry(p).or_default().push(q);
    }
    // Every vertex has as many edges out as in, so a walk can only end
    // where it started.
    let mut rings = Vec::new();
    for &(start, _) in &edges {
        let mut ring = Vec::new();
        let mut at = start;
        while let Some(q) = next.get_mut(&at).and_then(Vec::pop) {
            ring.push(at);
            at = q;
        }
        if !ring.is_empty() {
            rings.push(ring);
        }
    }
    rings
}

/// Twice the signed area of (a, b, p), in sub-pixel units squared.
fn edge(a: [i64; 2], b: [i64; 2], p: [i64; 2]) -> i64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

/// One of the two directions along any edge is "top-left", so two
/// triangles that share an edge never both own a pixel exactly on it.
fn top_left(a: [i64; 2], b: [i64; 2]) -> bool {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    dy > 0 || (dy == 0 && dx < 0)
}

/// Bilinear sample of premultiplied `src` at image point (`u`, `v`);
/// pixel `i` has its centre at `i + 0.5`. Edges repeat outwards.
fn sample(src: &Pixmap, u: f32, v: f32) -> [u8; 4] {
    let (w, h) = (src.width() as i32, src.height() as i32);
    let (x, y) = (u - 0.5, v - 0.5);
    let (fx, fy) = (x.floor(), y.floor());
    let (ax, ay) = (x - fx, y - fy);
    let cx = |i: i32| i.clamp(0, w - 1) as usize;
    let cy = |i: i32| i.clamp(0, h - 1) as usize;
    let (x0, y0) = (fx as i32, fy as i32);
    let d = src.data();
    let px = |x: usize, y: usize| &d[(y * w as usize + x) * 4..][..4];
    let (p00, p10) = (px(cx(x0), cy(y0)), px(cx(x0 + 1), cy(y0)));
    let (p01, p11) = (px(cx(x0), cy(y0 + 1)), px(cx(x0 + 1), cy(y0 + 1)));
    std::array::from_fn(|k| {
        let top = p00[k] as f32 + (p10[k] as f32 - p00[k] as f32) * ax;
        let bot = p01[k] as f32 + (p11[k] as f32 - p01[k] as f32) * ax;
        (top + (bot - top) * ay).round() as u8
    })
}

/// Premultiplied source-over.
fn over(dst: &mut [u8], s: [u8; 4]) {
    let a = s[3] as u32;
    if a == 0 {
        return;
    }
    if a == 255 {
        dst.copy_from_slice(&s);
        return;
    }
    let inv = 255 - a;
    for k in 0..4 {
        dst[k] = (s[k] as u32 + (dst[k] as u32 * inv + 127) / 255).min(255) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphony_core::deform::{BonePose, ChainPose, Influence};
    use tiny_skia::{IntSize, PremultipliedColorU8};

    /// Two 100-pixel bones along +x from (-100, 0), joint at the origin.
    fn pose(deg: f32) -> ChainPose {
        let mut root = BonePose::new(0.0, 100.0, 100.0);
        root.x = -100.0;
        let child = BonePose::new(0.0, 100.0, 100.0);
        let rest = [root, child];
        let mut anim = rest;
        anim[1].orientation = deg;
        // The golden images test the mesh and its edges, not the fold.
        anim[1].fold = symphony_core::deform::Fold::Arc;
        ChainPose::new(&rest, &anim, Influence::Infinite)
    }

    /// Image pixel to output pixel for a 200 x 40 image over the arm,
    /// drawn at 2x into a 600 x 600 target centred on the joint.
    fn to_out(p: &ChainPose) -> impl Fn([f32; 2]) -> [f32; 2] + '_ {
        move |[u, v]| {
            let q = p.map([u - 100.0, v - 20.0]);
            [300.0 + 2.0 * q[0], 300.0 + 2.0 * q[1]]
        }
    }

    /// Edges used by one triangle only must lie on the image's border;
    /// any other is a crack.
    fn cracks(mesh: &Mesh, w: f32, h: f32) -> usize {
        let mut count: HashMap<(u32, u32), i32> = HashMap::new();
        for t in &mesh.triangles {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let border = |i: u32| {
            let [u, v] = mesh.uv[i as usize];
            [u == 0.0 || u == w, v == 0.0 || v == h]
        };
        count
            .iter()
            .filter(|&(&(a, b), &n)| {
                let (ba, bb) = (border(a), border(b));
                let on_border =
                    (ba[0] && bb[0] && mesh.uv[a as usize][0] == mesh.uv[b as usize][0])
                        || (ba[1] && bb[1] && mesh.uv[a as usize][1] == mesh.uv[b as usize][1]);
                n != 2 && !(n == 1 && on_border)
            })
            .count()
    }

    #[test]
    fn the_crack_check_finds_a_t_junction() {
        // A big triangle beside two small ones that split its edge.
        let mesh = Mesh {
            points: vec![[0.0; 2]; 5],
            uv: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [4.0, 0.0], [2.0, 1.0]],
            triangles: vec![[0, 1, 2], [1, 3, 4], [4, 3, 2]],
        };
        assert!(cracks(&mesh, 4.0, 2.0) > 0);
    }

    #[test]
    fn a_rigid_map_keeps_the_largest_cells() {
        let p = pose(0.0);
        let mesh = build(200, 40, to_out(&p), &MeshDetail::default());
        // 200 x 40 at 64-pixel cells: 4 x 1 cells, 2 triangles each.
        assert_eq!(mesh.triangles.len(), 8);
        assert_eq!(cracks(&mesh, 200.0, 40.0), 0);
    }

    #[test]
    fn a_bend_refines_near_the_joint_only_and_leaves_no_cracks() {
        let p = pose(70.0);
        let mesh = build(200, 40, to_out(&p), &MeshDetail::default());
        assert_eq!(cracks(&mesh, 200.0, 40.0), 0);
        let near = |t: &[u32; 3]| {
            t.iter()
                .all(|&i| (mesh.uv[i as usize][0] - 100.0).abs() < 40.0)
        };
        let (n_near, n_far) =
            mesh.triangles.iter().fold(
                (0, 0),
                |(a, b), t| {
                    if near(t) {
                        (a + 1, b)
                    } else {
                        (a, b + 1)
                    }
                },
            );
        assert!(n_near > 4 * n_far, "near {n_near}, far {n_far}");
    }

    #[test]
    fn the_mesh_follows_the_map_within_the_tolerance() {
        // Where the art folds (past about 25 degrees here) cells stop at
        // the smallest size, and the error there may stay higher.
        let p = pose(20.0);
        let f = to_out(&p);
        let detail = MeshDetail::default();
        let mesh = build(200, 40, &f, &detail);
        // Inside each triangle, interpolated positions against the map.
        let mut worst = 0.0f32;
        for t in &mesh.triangles {
            let [a, b, c] = t.map(|i| i as usize);
            for (la, lb) in [(1.0 / 3.0, 1.0 / 3.0), (0.5, 0.25), (0.25, 0.5), (0.5, 0.5)] {
                let lc = 1.0 - la - lb;
                let mix = |v: &[[f32; 2]], k: usize| la * v[a][k] + lb * v[b][k] + lc * v[c][k];
                let uv = [mix(&mesh.uv, 0), mix(&mesh.uv, 1)];
                let real = f(uv);
                let flat = [mix(&mesh.points, 0), mix(&mesh.points, 1)];
                worst = worst.max((real[0] - flat[0]).hypot(real[1] - flat[1]));
            }
        }
        assert!(worst <= 1.5 * detail.tolerance, "worst {worst}");
    }

    fn checker(w: u32, h: u32) -> Pixmap {
        let mut p = Pixmap::new(w, h).unwrap();
        for (i, px) in p.pixels_mut().iter_mut().enumerate() {
            let (x, y) = (i as u32 % w, i as u32 / w);
            let v = if (x / 4 + y / 4) % 2 == 0 { 255 } else { 40 };
            *px = PremultipliedColorU8::from_rgba(v, (x * 3) as u8, (y * 5) as u8, 255).unwrap();
        }
        p
    }

    #[test]
    fn an_unbent_mesh_copies_pixels_exactly() {
        // One image pixel per output pixel, at an integer offset.
        let src = checker(40, 24);
        let mesh = build(40, 24, |[u, v]| [u + 5.0, v + 3.0], &MeshDetail::default());
        let mut pix = Pixmap::new(60, 40).unwrap();
        draw(&mut pix, &src, &mesh);
        for y in 0..24 {
            for x in 0..40 {
                assert_eq!(
                    pix.pixel(x + 5, y + 3).unwrap(),
                    src.pixel(x, y).unwrap(),
                    "pixel ({x}, {y})"
                );
            }
        }
        assert_eq!(pix.pixel(4, 3).unwrap().alpha(), 0);
        assert_eq!(pix.pixel(45, 3).unwrap().alpha(), 0);
    }

    fn white(w: u32, h: u32) -> Pixmap {
        Pixmap::from_vec(
            [255u8; 4].repeat((w * h) as usize),
            IntSize::from_wh(w, h).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn the_outer_edge_is_anti_aliased() {
        // A white image from x = 5.5 to 25.5: the pixels at 5 and 25 are
        // half covered, the ones inside fully, the ones past not at all.
        let src = white(20, 20);
        let mesh = build(20, 20, |[u, v]| [u + 5.5, v + 3.0], &MeshDetail::default());
        let mut pix = Pixmap::new(40, 30).unwrap();
        draw(&mut pix, &src, &mesh);
        let at = |x: u32| pix.pixel(x, 10).unwrap();
        for x in [5, 25] {
            let p = at(x);
            assert!((120..=136).contains(&p.alpha()), "pixel {x}: {}", p.alpha());
            assert_eq!(p.red(), p.alpha(), "pixel {x} is premultiplied white");
        }
        for x in 6..25 {
            assert_eq!(at(x).alpha(), 255, "pixel {x}");
        }
        assert_eq!(at(4).alpha(), 0);
        assert_eq!(at(26).alpha(), 0);
    }

    #[test]
    fn a_hard_bend_leaves_no_seam_inside() {
        // Opaque, bent past the fold: wherever a pixel's neighbours are
        // all fully covered, so is the pixel.
        let src = white(200, 40);
        let p = pose(70.0);
        let mesh = build(200, 40, to_out(&p), &MeshDetail::default());
        let mut pix = Pixmap::new(600, 600).unwrap();
        draw(&mut pix, &src, &mesh);
        let alpha = |x: u32, y: u32| pix.pixel(x, y).unwrap().alpha();
        let mut seams = 0;
        let mut edge = 0;
        for y in 1..599 {
            for x in 1..599 {
                let a = alpha(x, y);
                if 0 < a && a < 255 {
                    edge += 1;
                }
                let around = [
                    alpha(x - 1, y),
                    alpha(x + 1, y),
                    alpha(x, y - 1),
                    alpha(x, y + 1),
                ];
                if a < 255 && around.iter().all(|&n| n == 255) {
                    seams += 1;
                }
            }
        }
        assert_eq!(seams, 0);
        // The outline is soft.
        assert!(edge > 500, "{edge} partly covered pixels");
    }

    /// The 200 x 40 arm image as 20-pixel red and blue squares, opaque.
    fn arm_checker() -> Pixmap {
        let mut p = Pixmap::new(200, 40).unwrap();
        let [red, blue] = [[255, 0, 0, 255], [0, 0, 255, 255]];
        for (i, px) in p.data_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i % 200 / 20, i / 200 / 20);
            *px = if (x + y) % 2 == 0 { red } else { blue };
        }
        p
    }

    /// Golden scale: the arm at a quarter size, the joint at a point off
    /// the pixel grid in a 64 x 64 frame, so every edge is soft.
    const GOLDEN_SCALE: f32 = 0.25;
    const GOLDEN_JOINT: [f32; 2] = [32.3, 32.4];

    fn golden_out(p: &ChainPose) -> impl Fn([f32; 2]) -> [f32; 2] + '_ {
        move |[u, v]| {
            let q = p.map([u - 100.0, v - 20.0]);
            let [jx, jy] = GOLDEN_JOINT;
            [jx + GOLDEN_SCALE * q[0], jy + GOLDEN_SCALE * q[1]]
        }
    }

    /// The arm drawn without the warp, as `paint_image_bent` does when
    /// the chain moves nothing.
    fn golden_plain() -> Pixmap {
        let [jx, jy] = GOLDEN_JOINT;
        let t = Transform::from_scale(GOLDEN_SCALE, GOLDEN_SCALE)
            .post_translate(jx - GOLDEN_SCALE * 100.0, jy - GOLDEN_SCALE * 20.0);
        let mut pix = Pixmap::new(64, 64).unwrap();
        bitmap::draw(&mut pix, &arm_checker(), t);
        pix
    }

    fn golden_bent(deg: f32) -> Pixmap {
        let p = pose(deg);
        let mesh = build(200, 40, golden_out(&p), &MeshDetail::default());
        let mut pix = Pixmap::new(64, 64).unwrap();
        draw(&mut pix, &arm_checker(), &mesh);
        pix
    }

    #[test]
    fn golden_edges_of_a_bent_image() {
        crate::golden::check("bent-plain", &golden_plain());
        crate::golden::check("bent-1", &golden_bent(1.0));
        crate::golden::check("bent-70-fold", &golden_bent(70.0));
    }

    #[test]
    fn a_small_bend_keeps_the_plain_edge() {
        // Over the root bone, away from the joint, a 1 degree bend moves
        // nothing: the warp's edge must cover what the plain image's edge
        // covers. (Colours may differ by a step or two: the warp samples
        // the image at slightly different points.)
        let (plain, bent) = (golden_plain(), golden_bent(1.0));
        let mut worst = 0;
        let mut soft = 0;
        for y in 0..64 {
            for x in 5..22 {
                let (a, b) = (
                    plain.pixel(x, y).unwrap().alpha(),
                    bent.pixel(x, y).unwrap().alpha(),
                );
                if 0 < a && a < 255 {
                    soft += 1;
                }
                worst = worst.max(a.abs_diff(b));
            }
        }
        assert!(soft > 20, "{soft} soft edge pixels");
        assert_eq!(worst, 0, "alpha differs");
    }

    #[test]
    fn golden_hard_edge_of_a_bent_image_at_a_huge_zoom() {
        // A bent mesh, then zoomed 1e4 times about a point on the top
        // edge of the root bone: vertices land past MAX_PATH, so the edge
        // is hard. Far larger zooms overflow `edge`.
        let p = pose(30.0);
        let f = to_out(&p);
        let mut mesh = build(200, 40, &f, &MeshDetail::default());
        let o = f([50.0, 0.0]);
        for q in &mut mesh.points {
            *q = [1e4 * (q[0] - o[0]) + 32.3, 1e4 * (q[1] - o[1]) + 32.4];
        }
        let mut pix = Pixmap::new(64, 64).unwrap();
        draw(&mut pix, &arm_checker(), &mesh);
        crate::golden::check("bent-huge-zoom", &pix);
    }

    #[test]
    fn a_triangle_too_large_for_the_edge_maths_is_dropped() {
        // As a bone stretched 1e7 times would pull it: the i64 edge maths
        // overflowed here before. Every triangle is too large, so nothing
        // is drawn.
        let p = pose(30.0);
        let f = to_out(&p);
        let mut mesh = build(200, 40, &f, &MeshDetail::default());
        let o = f([50.0, 0.0]);
        for q in &mut mesh.points {
            *q = [1e7 * (q[0] - o[0]) + 32.3, 1e7 * (q[1] - o[1]) + 32.4];
        }
        let mut pix = Pixmap::new(64, 64).unwrap();
        draw(&mut pix, &arm_checker(), &mesh);
        assert!(pix.data().iter().all(|&v| v == 0));
    }

    #[test]
    fn a_bent_image_has_no_holes_or_double_edges() {
        // Grey at 50% alpha: a pixel drawn twice turns darker, a pixel
        // missed stays clear, and the anti-aliased outline is lighter. A
        // gentle bend, because past about 25 degrees this arm folds on the
        // inside and the fold is drawn twice.
        let src = Pixmap::from_vec(
            [128u8, 128, 128, 128].repeat(200 * 40),
            IntSize::from_wh(200, 40).unwrap(),
        )
        .unwrap();
        let p = pose(20.0);
        let mesh = build(200, 40, to_out(&p), &MeshDetail::default());
        let mut pix = Pixmap::new(600, 600).unwrap();
        draw(&mut pix, &src, &mesh);
        let alpha = |x: u32, y: u32| pix.pixel(x, y).unwrap().alpha();
        let mut holes = 0;
        for y in 1..599 {
            for x in 1..599 {
                let a = alpha(x, y);
                assert!(a <= 128, "pixel ({x}, {y}) drawn twice: {a}");
                let around = [
                    alpha(x - 1, y),
                    alpha(x + 1, y),
                    alpha(x, y - 1),
                    alpha(x, y + 1),
                ];
                // A clear or lighter pixel inside: a hole or a seam.
                if a < 128 && around.iter().all(|&n| n == 128) {
                    holes += 1;
                }
            }
        }
        assert_eq!(holes, 0);
        // And something was drawn on both sides of the joint.
        assert_eq!(alpha(300 - 150, 300), 128);
        let end = to_out(&p)([190.0, 20.0]);
        assert_eq!(alpha(end[0] as u32, end[1] as u32), 128);
    }

    #[test]
    fn the_rings_cover_like_all_the_triangles() {
        // The coverage of the rings left after inner edges cancel must
        // be the coverage of every triangle filled as one path, at a
        // plain bend and at a fold. tiny-skia rounds a few outline
        // pixels one step apart when the edges come in another order.
        for deg in [30.0, 135.0] {
            let p = pose(deg);
            let mesh = build(200, 40, to_out(&p), &MeshDetail::default());
            let fixed: Vec<[i64; 2]> = mesh
                .points
                .iter()
                .map(|q| q.map(|v| (v * SUB as f32).round() as i64))
                .collect();
            let tris: Vec<[usize; 3]> = mesh
                .triangles
                .iter()
                .filter_map(|t| {
                    let [a, b, c] = t.map(|i| i as usize);
                    match edge(fixed[a], fixed[b], fixed[c]) {
                        0 => None,
                        n if n < 0 => Some([a, c, b]),
                        _ => Some([a, b, c]),
                    }
                })
                .collect();
            let edges: usize = rings(&tris).iter().map(Vec::len).sum();
            assert!(edges * 4 < tris.len() * 3, "{edges} ring edges");
            let fast = coverage(&fixed, &tris, 0, 0, 600, 600).unwrap();
            let mut pb = PathBuilder::new();
            for t in &tris {
                let [a, b, c] = t.map(|i| fixed[i].map(|v| v as f32 / SUB as f32));
                pb.move_to(a[0], a[1]);
                pb.line_to(b[0], b[1]);
                pb.line_to(c[0], c[1]);
                pb.close();
            }
            let mut all = Mask::new(600, 600).unwrap();
            let path = pb.finish().unwrap();
            all.fill_path(&path, FillRule::Winding, true, Transform::identity());
            let worst = (fast.data().iter().zip(all.data()))
                .map(|(a, b)| a.abs_diff(*b))
                .max();
            assert!(worst <= Some(1), "{worst:?} at {deg} degrees");
        }
    }
}
