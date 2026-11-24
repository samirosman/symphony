//! Points deformation (the Mesh Warp's Points mode, in the spirit of
//! Harmony 24's Shape-Aware Deformation): a triangle mesh built over the
//! artwork's shape, with points pinned to some of its vertices. Moving
//! the points moves the mesh as rigidly as it can (as-rigid-as-possible,
//! the method of Igarashi, Moscovich and Hughes, 2005): each triangle
//! keeps its shape as far as the points allow, so limbs bend and swing
//! instead of smearing, and only the artwork connected to a point
//! follows it (the mesh only covers the shape, not the empty space
//! between parts).
//!
//! Solving needs one-off factorisations per mesh (cached); each frame is
//! then two cheap back-substitutions per refinement step.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// A mesh over the artwork, in the warp's space, with the points' vertices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PuppetMesh {
    pub vertices: Vec<[f32; 2]>,
    pub triangles: Vec<[u32; 3]>,
    /// The vertex each point is pinned to.
    pub pins: Vec<u32>,
}

/// How hard points hold their place compared with the mesh's shape.
const PIN_WEIGHT: f64 = 1.0e4;
/// A whisper of "move along with your part's points" (or stay, for a
/// part no point is on), so a part held by one point slides with it
/// instead of turning, and nothing is left undefined.
const REST_WEIGHT: f64 = 1.0e-4;
/// Rigidity refinement passes per solve.
const PASSES: usize = 3;

/// Cholesky factor of a symmetric positive definite matrix (dense; the
/// meshes here are a few hundred vertices).
struct Cholesky {
    n: usize,
    l: Vec<f64>,
}

impl Cholesky {
    fn new(mut a: Vec<f64>, n: usize) -> Option<Self> {
        for j in 0..n {
            let mut d = a[j * n + j];
            for k in 0..j {
                d -= a[j * n + k] * a[j * n + k];
            }
            if d <= 0.0 || !d.is_finite() {
                return None;
            }
            let d = d.sqrt();
            a[j * n + j] = d;
            for i in j + 1..n {
                let mut s = a[i * n + j];
                for k in 0..j {
                    s -= a[i * n + k] * a[j * n + k];
                }
                a[i * n + j] = s / d;
            }
        }
        Some(Cholesky { n, l: a })
    }

    fn solve(&self, b: &mut [f64]) {
        let (n, l) = (self.n, &self.l);
        for i in 0..n {
            let mut s = b[i];
            for k in 0..i {
                s -= l[i * n + k] * b[k];
            }
            b[i] = s / l[i * n + i];
        }
        for i in (0..n).rev() {
            let mut s = b[i];
            for k in i + 1..n {
                s -= l[k * n + i] * b[k];
            }
            b[i] = s / l[i * n + i];
        }
    }
}

/// The factorised systems for one mesh.
struct Solver {
    /// Step 1 (shape up to rotation and scale), unknowns x0 y0 x1 y1 ...
    similarity: Cholesky,
    /// Step 2 (rigid edges), one coordinate at a time.
    edges: Cholesky,
    /// Which separate part (connected piece of mesh) each vertex is in.
    part: Vec<usize>,
}

fn solver(mesh: &PuppetMesh) -> Option<Arc<Solver>> {
    static CACHE: Mutex<Option<HashMap<u64, Arc<Solver>>>> = Mutex::new(None);
    let key = mesh_key(mesh);
    if let Some(found) = CACHE
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(&key).cloned()))
    {
        return Some(found);
    }
    let built = Arc::new(build(mesh)?);
    if let Ok(mut g) = CACHE.lock() {
        let m = g.get_or_insert_with(HashMap::new);
        if m.len() > 64 {
            m.clear();
        }
        m.insert(key, built.clone());
    }
    Some(built)
}

fn mesh_key(mesh: &PuppetMesh) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for v in &mesh.vertices {
        (v[0].to_bits(), v[1].to_bits()).hash(&mut h);
    }
    mesh.triangles.hash(&mut h);
    mesh.pins.hash(&mut h);
    h.finish()
}

fn build(mesh: &PuppetMesh) -> Option<Solver> {
    let n = mesh.vertices.len();
    if n == 0 {
        return None;
    }
    let v = |i: u32| {
        let p = mesh.vertices[i as usize];
        (p[0] as f64, p[1] as f64)
    };
    // Step 1: for each triangle and each corner, that corner where the
    // other two put it, keeping the triangle's shape.
    let m = 2 * n;
    let mut a = vec![0.0f64; m * m];
    for t in &mesh.triangles {
        for k in 0..3 {
            let (i0, i1, i2) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
            let (p0, p1, p2) = (v(i0), v(i1), v(i2));
            let d = (p1.0 - p0.0, p1.1 - p0.1);
            let e = (p2.0 - p0.0, p2.1 - p0.1);
            let dd = (d.0 * d.0 + d.1 * d.1).max(1e-12);
            // e = x d + y R(d), R(d) = (d.y, -d.x).
            let x = (e.0 * d.0 + e.1 * d.1) / dd;
            let y = (e.0 * d.1 - e.1 * d.0) / dd;
            let (c0, c1, c2) = (i0 as usize, i1 as usize, i2 as usize);
            // Row for x:  v2x - v0x - x (v1x - v0x) - y (v1y - v0y)
            let rx = [
                (2 * c2, 1.0),
                (2 * c0, -1.0 + x),
                (2 * c1, -x),
                (2 * c1 + 1, -y),
                (2 * c0 + 1, y),
            ];
            // Row for y:  v2y - v0y - x (v1y - v0y) + y (v1x - v0x)
            let ry = [
                (2 * c2 + 1, 1.0),
                (2 * c0 + 1, -1.0 + x),
                (2 * c1 + 1, -x),
                (2 * c1, y),
                (2 * c0, -y),
            ];
            for row in [rx, ry] {
                for &(i, wi) in &row {
                    for &(j, wj) in &row {
                        a[i * m + j] += wi * wj;
                    }
                }
            }
        }
    }
    for i in 0..m {
        a[i * m + i] += REST_WEIGHT;
    }
    for &p in &mesh.pins {
        let p = p as usize;
        a[(2 * p) * m + 2 * p] += PIN_WEIGHT;
        a[(2 * p + 1) * m + 2 * p + 1] += PIN_WEIGHT;
    }
    let similarity = Cholesky::new(a, m)?;
    // Step 2: every edge as long as at rest (turned as its triangle
    // turned).
    let mut h = vec![0.0f64; n * n];
    for t in &mesh.triangles {
        for k in 0..3 {
            let (i, j) = (t[k] as usize, t[(k + 1) % 3] as usize);
            h[i * n + i] += 1.0;
            h[j * n + j] += 1.0;
            h[i * n + j] -= 1.0;
            h[j * n + i] -= 1.0;
        }
    }
    for i in 0..n {
        h[i * n + i] += REST_WEIGHT;
    }
    for &p in &mesh.pins {
        h[p as usize * n + p as usize] += PIN_WEIGHT;
    }
    let edges = Cholesky::new(h, n)?;
    // Connected parts (union-find over triangle edges).
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for t in &mesh.triangles {
        for k in 0..3 {
            let (a, b) = (
                root(&mut parent, t[k] as usize),
                root(&mut parent, t[(k + 1) % 3] as usize),
            );
            parent[a] = b;
        }
    }
    let part = (0..n).map(|i| root(&mut parent, i)).collect();
    Some(Solver {
        similarity,
        edges,
        part,
    })
}

/// Where every mesh vertex goes when point `i` is at `targets[i]`.
/// Recent answers are kept, so a frame that didn't move a point costs
/// nothing.
pub fn deform(mesh: &PuppetMesh, targets: &[[f32; 2]]) -> Vec<[f32; 2]> {
    use std::hash::{Hash, Hasher};
    type Recent = Vec<(u64, Arc<Vec<[f32; 2]>>)>;
    static RECENT: Mutex<Recent> = Mutex::new(Vec::new());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    mesh_key(mesh).hash(&mut h);
    for t in targets {
        (t[0].to_bits(), t[1].to_bits()).hash(&mut h);
    }
    let key = h.finish();
    if let Some(found) = RECENT
        .lock()
        .ok()
        .and_then(|r| r.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()))
    {
        return found.as_ref().clone();
    }
    let now = solve(mesh, targets);
    if let Ok(mut r) = RECENT.lock() {
        if r.len() >= 16 {
            r.remove(0);
        }
        r.push((key, Arc::new(now.clone())));
    }
    now
}

fn solve(mesh: &PuppetMesh, targets: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let n = mesh.vertices.len();
    let Some(s) = solver(mesh) else {
        return mesh.vertices.clone();
    };
    let rest: Vec<(f64, f64)> = mesh
        .vertices
        .iter()
        .map(|p| (p[0] as f64, p[1] as f64))
        .collect();
    let target = |k: usize| {
        let t = targets
            .get(k)
            .copied()
            .unwrap_or(mesh.vertices[mesh.pins[k] as usize]);
        (t[0] as f64, t[1] as f64)
    };
    // Each part's average point move: where its vertices lean.
    let mut moves: HashMap<usize, (f64, f64, f64)> = HashMap::new();
    for (k, &p) in mesh.pins.iter().enumerate() {
        let (t, r) = (target(k), rest[p as usize]);
        let e = moves.entry(s.part[p as usize]).or_default();
        e.0 += t.0 - r.0;
        e.1 += t.1 - r.1;
        e.2 += 1.0;
    }
    let rest_target: Vec<(f64, f64)> = rest
        .iter()
        .enumerate()
        .map(|(i, r)| match moves.get(&s.part[i]) {
            Some(m) => (r.0 + m.0 / m.2, r.1 + m.1 / m.2),
            None => *r,
        })
        .collect();
    // Step 1.
    let mut b = vec![0.0f64; 2 * n];
    for (i, r) in rest_target.iter().enumerate() {
        b[2 * i] += REST_WEIGHT * r.0;
        b[2 * i + 1] += REST_WEIGHT * r.1;
    }
    for (k, &p) in mesh.pins.iter().enumerate() {
        let t = target(k);
        b[2 * p as usize] += PIN_WEIGHT * t.0;
        b[2 * p as usize + 1] += PIN_WEIGHT * t.1;
    }
    s.similarity.solve(&mut b);
    let mut now: Vec<(f64, f64)> = (0..n).map(|i| (b[2 * i], b[2 * i + 1])).collect();
    // Step 2, a few times: each triangle turned to fit where it went,
    // then the edges solved to those turned rest edges.
    for _ in 0..PASSES {
        let mut bx = vec![0.0f64; n];
        let mut by = vec![0.0f64; n];
        for (i, r) in rest_target.iter().enumerate() {
            bx[i] += REST_WEIGHT * r.0;
            by[i] += REST_WEIGHT * r.1;
        }
        for (k, &p) in mesh.pins.iter().enumerate() {
            let t = target(k);
            bx[p as usize] += PIN_WEIGHT * t.0;
            by[p as usize] += PIN_WEIGHT * t.1;
        }
        for t in &mesh.triangles {
            let idx = t.map(|i| i as usize);
            // Best rotation from rest to now (2D Procrustes).
            let cr = idx.iter().fold((0.0, 0.0), |a, &i| {
                (a.0 + rest[i].0 / 3.0, a.1 + rest[i].1 / 3.0)
            });
            let cn = idx.iter().fold((0.0, 0.0), |a, &i| {
                (a.0 + now[i].0 / 3.0, a.1 + now[i].1 / 3.0)
            });
            let (mut dot, mut cross) = (0.0, 0.0);
            for &i in &idx {
                let (p, q) = (
                    (rest[i].0 - cr.0, rest[i].1 - cr.1),
                    (now[i].0 - cn.0, now[i].1 - cn.1),
                );
                dot += p.0 * q.0 + p.1 * q.1;
                cross += p.0 * q.1 - p.1 * q.0;
            }
            let a = cross.atan2(dot);
            let (sin, cos) = a.sin_cos();
            for k in 0..3 {
                let (i, j) = (idx[k], idx[(k + 1) % 3]);
                let e = (rest[j].0 - rest[i].0, rest[j].1 - rest[i].1);
                let e = (cos * e.0 - sin * e.1, sin * e.0 + cos * e.1);
                bx[j] += e.0;
                by[j] += e.1;
                bx[i] -= e.0;
                by[i] -= e.1;
            }
        }
        s.edges.solve(&mut bx);
        s.edges.solve(&mut by);
        now = bx.into_iter().zip(by).collect();
    }
    now.into_iter().map(|(x, y)| [x as f32, y as f32]).collect()
}

/// A deformed mesh, ready to move any point: found in its rest triangle,
/// carried by that triangle's move. Points off the mesh follow the
/// nearest triangle.
#[derive(Debug)]
pub struct PuppetField {
    /// Identifies the mesh and where it is (cheap to compare).
    key: u64,
    mesh: Arc<PuppetMesh>,
    now: Vec<[f32; 2]>,
    /// Triangles by grid cell, to find a point's triangle quickly.
    cells: HashMap<(i32, i32), Vec<u32>>,
    cell: f32,
}

impl PartialEq for PuppetField {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl PuppetField {
    pub fn new(mesh: Arc<PuppetMesh>, now: Vec<[f32; 2]>) -> Self {
        // About two triangles across a cell.
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in &mesh.vertices {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let area = ((hi[0] - lo[0]) * (hi[1] - lo[1])).max(1.0);
        let cell = (area / mesh.triangles.len().max(1) as f32).sqrt().max(1e-3) * 2.0;
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (ti, t) in mesh.triangles.iter().enumerate() {
            let ps = t.map(|i| mesh.vertices[i as usize]);
            let (x0, x1) = (
                ps.iter().map(|p| p[0]).fold(f32::MAX, f32::min),
                ps.iter().map(|p| p[0]).fold(f32::MIN, f32::max),
            );
            let (y0, y1) = (
                ps.iter().map(|p| p[1]).fold(f32::MAX, f32::min),
                ps.iter().map(|p| p[1]).fold(f32::MIN, f32::max),
            );
            for cx in (x0 / cell).floor() as i32..=(x1 / cell).floor() as i32 {
                for cy in (y0 / cell).floor() as i32..=(y1 / cell).floor() as i32 {
                    cells.entry((cx, cy)).or_default().push(ti as u32);
                }
            }
        }
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            mesh_key(&mesh).hash(&mut h);
            for p in &now {
                (p[0].to_bits(), p[1].to_bits()).hash(&mut h);
            }
            h.finish()
        };
        PuppetField {
            key,
            mesh,
            now,
            cells,
            cell,
        }
    }

    /// Identifies this mesh and its pose.
    pub fn key(&self) -> u64 {
        self.key
    }

    pub fn mesh(&self) -> &PuppetMesh {
        &self.mesh
    }

    /// Where each mesh vertex is now.
    pub fn now(&self) -> &[[f32; 2]] {
        &self.now
    }

    pub fn is_flat(&self) -> bool {
        self.mesh
            .vertices
            .iter()
            .zip(&self.now)
            .all(|(a, b)| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4)
    }

    /// The typical triangle size (bent edges are split finer than this).
    pub fn cell(&self) -> f32 {
        self.cell / 2.0
    }

    fn barycentric(&self, t: u32, p: [f32; 2]) -> [f32; 3] {
        let [a, b, c] = self.mesh.triangles[t as usize].map(|i| self.mesh.vertices[i as usize]);
        let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
        if d.abs() < 1e-12 {
            return [1.0, 0.0, 0.0];
        }
        let l0 = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
        let l1 = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
        [l0, l1, 1.0 - l0 - l1]
    }

    /// Move a point (in the warp's space).
    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        let key = (
            (p[0] / self.cell).floor() as i32,
            (p[1] / self.cell).floor() as i32,
        );
        // Its own triangle, or the nearest one (in widening rings).
        let mut best: Option<(u32, [f32; 3], f32)> = None;
        for ring in 0..6 {
            for cx in key.0 - ring..=key.0 + ring {
                for cy in key.1 - ring..=key.1 + ring {
                    if ring > 0 && (cx - key.0).abs() != ring && (cy - key.1).abs() != ring {
                        continue;
                    }
                    for &t in self.cells.get(&(cx, cy)).into_iter().flatten() {
                        let l = self.barycentric(t, p);
                        // How far outside the triangle (0 inside).
                        let out = -l.iter().copied().fold(0.0f32, f32::min);
                        if best.is_none_or(|b| out < b.2) {
                            best = Some((t, l, out));
                        }
                    }
                }
            }
            if best.is_some_and(|b| b.2 == 0.0) || (best.is_some() && ring >= 1) {
                break;
            }
        }
        let Some((t, l, _)) = best else {
            return p;
        };
        let tri = self.mesh.triangles[t as usize];
        let mut q = [0.0f32; 2];
        for k in 0..3 {
            let v = self.now[tri[k] as usize];
            q[0] += l[k] * v[0];
            q[1] += l[k] * v[1];
        }
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A strip of squares, 10 x 1, split into triangles.
    fn strip() -> PuppetMesh {
        let mut vertices = Vec::new();
        for i in 0..=10 {
            vertices.push([i as f32 * 10.0, 0.0]);
            vertices.push([i as f32 * 10.0, 10.0]);
        }
        let mut triangles = Vec::new();
        for i in 0..10u32 {
            let (a, b, c, d) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
            triangles.push([a, c, b]);
            triangles.push([b, c, d]);
        }
        PuppetMesh {
            vertices,
            triangles,
            pins: vec![0, 1, 20],
        }
    }

    #[test]
    fn unmoved_points_change_nothing() {
        let m = strip();
        let targets: Vec<[f32; 2]> = m.pins.iter().map(|&p| m.vertices[p as usize]).collect();
        for (a, b) in deform(&m, &targets).iter().zip(&m.vertices) {
            assert!((a[0] - b[0]).abs() < 1e-2 && (a[1] - b[1]).abs() < 1e-2);
        }
    }

    /// Swinging the strip's free end round keeps it about its length (an
    /// arm bends; it doesn't stretch like a grid warp would).
    #[test]
    fn a_moved_end_swings_the_strip_rigidly() {
        let m = strip();
        // Base held (two pins), the end pulled up and in.
        let targets = [[0.0, 0.0], [0.0, 10.0], [70.0, -70.0]];
        let now = deform(&m, &targets);
        let end = now[20];
        assert!(
            (end[0] - 70.0).abs() < 1.0 && (end[1] + 70.0).abs() < 1.0,
            "{end:?}"
        );
        // Width kept along the strip (each square's sides stay ~10).
        for i in 0..=10 {
            let (a, b) = (now[2 * i], now[2 * i + 1]);
            let w = (a[0] - b[0]).hypot(a[1] - b[1]);
            assert!((w - 10.0).abs() < 2.5, "width at {i}: {w}");
        }
        // The middle bends out of the straight line between base and end.
        let mid = now[10];
        assert!(mid[1] < -5.0 || mid[0] > 40.0, "{mid:?}");
    }

    #[test]
    fn points_follow_their_triangles_and_off_mesh_points_follow_the_nearest() {
        let m = strip();
        let shift = |p: [f32; 2]| [p[0] + 5.0, p[1] + 3.0];
        let now: Vec<[f32; 2]> = m.vertices.iter().map(|&p| shift(p)).collect();
        let f = PuppetField::new(Arc::new(m), now);
        assert_eq!(f.apply([33.0, 4.0]), shift([33.0, 4.0]));
        let off = f.apply([50.0, 14.0]);
        assert!((off[0] - 55.0).abs() < 1e-3 && (off[1] - 17.0).abs() < 1e-3);
    }

    #[test]
    fn a_part_no_point_reaches_stays_put() {
        // Two separate squares; a point only on the first.
        let mesh = PuppetMesh {
            vertices: vec![
                [0.0, 0.0],
                [10.0, 0.0],
                [10.0, 10.0],
                [0.0, 10.0],
                [100.0, 0.0],
                [110.0, 0.0],
                [110.0, 10.0],
                [100.0, 10.0],
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]],
            pins: vec![0],
        };
        let now = deform(&mesh, &[[20.0, 0.0]]);
        assert!((now[2][0] - 30.0).abs() < 0.5, "the first square follows");
        assert!(
            (now[6][0] - 110.0).abs() < 0.5,
            "the second stays: {:?}",
            now[6]
        );
    }
}
