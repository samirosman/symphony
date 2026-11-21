/// A parameter that is either constant or keyframed.
/// Keys are `(frame, value)`, frames 1-based. Between keys: linear for now
/// (Bezier/ease curves come with the Function view). Outside keys: hold.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Constant(f32),
    Keys(Vec<(u32, f32)>),
}

impl Value {
    pub fn at(&self, frame: u32) -> f32 {
        match self {
            Value::Constant(v) => *v,
            Value::Keys(keys) => {
                let Some(first) = keys.first() else {
                    return 0.0;
                };
                if frame <= first.0 {
                    return first.1;
                }
                for w in keys.windows(2) {
                    let ((f0, v0), (f1, v1)) = (w[0], w[1]);
                    if frame <= f1 {
                        let t = (frame - f0) as f32 / (f1 - f0).max(1) as f32;
                        return v0 + (v1 - v0) * t;
                    }
                }
                keys.last().map(|k| k.1).unwrap_or(0.0)
            }
        }
    }

    /// Insert or replace a key, keeping keys sorted by frame.
    pub fn set_key(&mut self, frame: u32, value: f32) {
        if let Value::Constant(_) = self {
            *self = Value::Keys(Vec::new());
        }
        if let Value::Keys(keys) = self {
            match keys.binary_search_by_key(&frame, |k| k.0) {
                Ok(i) => keys[i].1 = value,
                Err(i) => keys.insert(i, (frame, value)),
            }
        }
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
}
