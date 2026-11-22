use serde::{Deserialize, Deserializer, Serialize};

/// A parameter that is either constant or keyframed (a function, in
/// Harmony's words). Keys are sorted by frame, frames 1-based. Before the
/// first key and after the last, the value holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Constant(f32),
    Keys(Vec<Key>),
}

/// How a function moves from one key to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interp {
    /// Straight line.
    #[default]
    Linear,
    /// Curve shaped by the key's right handle and the next key's left
    /// handle (Harmony's Bezier function).
    Bezier,
    /// Hold the value until the next key (Harmony's stop-motion keyframe).
    Stop,
}

/// How a key's two Bézier handles move together (Harmony's continuity).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Continuity {
    /// Same angle and same length.
    #[default]
    Smooth,
    /// Same angle, independent lengths.
    Straight,
    /// Independent.
    Corner,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Key {
    pub frame: u32,
    pub value: f32,
    /// The segment from this key to the next.
    pub interp: Interp,
    /// Handle offsets from the key, in (frames, value). `left` points
    /// back (negative frames), `right` forward.
    pub left: [f32; 2],
    pub right: [f32; 2],
    pub continuity: Continuity,
}

impl Key {
    pub fn new(frame: u32, value: f32) -> Self {
        Key {
            frame,
            value,
            interp: Interp::Linear,
            left: [0.0, 0.0],
            right: [0.0, 0.0],
            continuity: Continuity::Smooth,
        }
    }
}

/// Keys load from the full form or from scenes saved before handles
/// existed, where a key was a plain `[frame, value]` pair.
impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Full {
            frame: u32,
            value: f32,
            #[serde(default)]
            interp: Interp,
            #[serde(default)]
            left: [f32; 2],
            #[serde(default)]
            right: [f32; 2],
            #[serde(default)]
            continuity: Continuity,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Pair(u32, f32),
            Full(Full),
        }
        Ok(match Repr::deserialize(d)? {
            Repr::Pair(frame, value) => Key::new(frame, value),
            Repr::Full(f) => Key {
                frame: f.frame,
                value: f.value,
                interp: f.interp,
                left: f.left,
                right: f.right,
                continuity: f.continuity,
            },
        })
    }
}

fn cubic(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
}

/// The value of a Bézier segment at frame `f` (between the two keys).
fn bezier(a: &Key, b: &Key, f: f32) -> f32 {
    let span = (b.frame - a.frame) as f32;
    // Handles may not reach past the other key, so frames stay in order
    // along the curve and each frame has one value.
    let rx = a.right[0].clamp(0.0, span);
    let lx = b.left[0].clamp(-span, 0.0);
    let x = [
        a.frame as f32,
        a.frame as f32 + rx,
        b.frame as f32 + lx,
        b.frame as f32,
    ];
    let y = [a.value, a.value + a.right[1], b.value + b.left[1], b.value];
    // x(t) only grows, so bisection finds the t for this frame.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..32 {
        let mid = (lo + hi) * 0.5;
        if cubic(x[0], x[1], x[2], x[3], mid) < f {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    cubic(y[0], y[1], y[2], y[3], (lo + hi) * 0.5)
}

impl Value {
    pub fn at(&self, frame: u32) -> f32 {
        self.at_f(frame as f32)
    }

    /// The value at any point in time, for drawing curves between frames.
    pub fn at_f(&self, frame: f32) -> f32 {
        match self {
            Value::Constant(v) => *v,
            Value::Keys(keys) => {
                let Some(first) = keys.first() else {
                    return 0.0;
                };
                if frame <= first.frame as f32 {
                    return first.value;
                }
                for w in keys.windows(2) {
                    let (a, b) = (&w[0], &w[1]);
                    if frame <= b.frame as f32 {
                        let span = (b.frame - a.frame).max(1) as f32;
                        return match a.interp {
                            Interp::Stop if frame < b.frame as f32 => a.value,
                            Interp::Stop => b.value,
                            Interp::Linear => {
                                a.value + (b.value - a.value) * (frame - a.frame as f32) / span
                            }
                            Interp::Bezier => bezier(a, b, frame),
                        };
                    }
                }
                keys.last().map(|k| k.value).unwrap_or(0.0)
            }
        }
    }

    pub fn keys(&self) -> &[Key] {
        match self {
            Value::Keys(k) => k,
            Value::Constant(_) => &[],
        }
    }

    pub fn has_key(&self, frame: u32) -> bool {
        self.keys().iter().any(|k| k.frame == frame)
    }

    /// Insert or replace a key, keeping keys sorted by frame. A replaced
    /// key keeps its interpolation and handles; a new one takes the
    /// interpolation of the key before it.
    pub fn set_key(&mut self, frame: u32, value: f32) {
        if let Value::Constant(_) = self {
            *self = Value::Keys(Vec::new());
        }
        if let Value::Keys(keys) = self {
            match keys.binary_search_by_key(&frame, |k| k.frame) {
                Ok(i) => keys[i].value = value,
                Err(i) => {
                    let mut k = Key::new(frame, value);
                    if let Some(prev) = i.checked_sub(1).and_then(|p| keys.get(p)) {
                        k.interp = prev.interp;
                    }
                    if k.interp == Interp::Bezier {
                        k.left = [-1.0, 0.0];
                        k.right = [1.0, 0.0];
                    }
                    keys.insert(i, k);
                }
            }
        }
    }

    /// Remove the key at `frame`. The last key removed leaves its value as
    /// a constant.
    pub fn remove_key(&mut self, frame: u32) {
        if let Value::Keys(keys) = self {
            if let Ok(i) = keys.binary_search_by_key(&frame, |k| k.frame) {
                let k = keys.remove(i);
                if keys.is_empty() {
                    *self = Value::Constant(k.value);
                }
            }
        }
    }

    pub fn key_mut(&mut self, frame: u32) -> Option<&mut Key> {
        match self {
            Value::Keys(keys) => keys.iter_mut().find(|k| k.frame == frame),
            Value::Constant(_) => None,
        }
    }

    /// Move a key to another frame and value. Refused (false) if another
    /// key is already on that frame.
    pub fn move_key(&mut self, from: u32, to: u32, value: f32) -> bool {
        let Value::Keys(keys) = self else {
            return false;
        };
        if from != to && keys.iter().any(|k| k.frame == to) {
            return false;
        }
        let Some(i) = keys.iter().position(|k| k.frame == from) else {
            return false;
        };
        keys[i].frame = to.max(1);
        keys[i].value = value;
        keys.sort_by_key(|k| k.frame);
        true
    }
}

impl Default for Value {
    fn default() -> Self {
        Value::Constant(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_and_hold() {
        let mut v = Value::Constant(0.0);
        v.set_key(1, 0.0);
        v.set_key(11, 100.0);
        assert_eq!(v.at(1), 0.0);
        assert_eq!(v.at(6), 50.0);
        assert_eq!(v.at(30), 100.0);
    }

    #[test]
    fn stop_motion_holds_until_the_next_key() {
        let mut v = Value::Constant(0.0);
        v.set_key(1, 0.0);
        v.set_key(11, 100.0);
        v.key_mut(1).unwrap().interp = Interp::Stop;
        assert_eq!(v.at(10), 0.0);
        assert_eq!(v.at(11), 100.0);
    }

    #[test]
    fn bezier_eases_and_flat_handles_slow_the_ends() {
        let mut v = Value::Constant(0.0);
        v.set_key(1, 0.0);
        v.set_key(11, 100.0);
        let a = v.key_mut(1).unwrap();
        a.interp = Interp::Bezier;
        a.right = [4.0, 0.0];
        v.key_mut(11).unwrap().left = [-4.0, 0.0];
        // Symmetric ease: halfway in time is halfway in value...
        assert!((v.at(6) - 50.0).abs() < 0.5);
        // ...but it starts slower than linear.
        assert!(v.at(2) < 10.0);
        assert!(v.at(10) > 90.0);
        // Handles past the other key are clamped, so it still reaches it.
        v.key_mut(1).unwrap().right = [50.0, 0.0];
        assert!((v.at(11) - 100.0).abs() < 0.01);
    }

    #[test]
    fn old_scenes_with_pairs_still_load() {
        let v: Value = serde_json::from_str(r#"{"Keys":[[1,0.0],[11,100.0]]}"#).unwrap();
        assert_eq!(v.keys().len(), 2);
        assert_eq!(v.keys()[1].interp, Interp::Linear);
        let back: Value = serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn keys_move_and_go() {
        let mut v = Value::Constant(5.0);
        v.set_key(1, 0.0);
        v.set_key(5, 10.0);
        assert!(!v.move_key(1, 5, 3.0), "can't land on another key");
        assert!(v.move_key(5, 3, 20.0));
        assert_eq!(v.at(3), 20.0);
        v.remove_key(1);
        v.remove_key(3);
        assert_eq!(v, Value::Constant(20.0));
    }
}
