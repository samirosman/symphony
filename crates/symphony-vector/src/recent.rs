//! A small content-keyed cache for geometry that's slow to build and
//! asked for again and again (stroke outlines for the eraser, vector-brush
//! outlines for the canvas and export). Two generations make it roughly
//! least-recently-used without bookkeeping: lookups promote, and when the
//! young generation fills up it becomes the old one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub(crate) struct Recent<V> {
    young: HashMap<u64, Arc<V>>,
    old: HashMap<u64, Arc<V>>,
}

/// A shared cache: `get_or_build(key, build)` builds at most once while
/// the result is in use. Building happens outside the lock, so two threads
/// asking at once may both build (harmless; the result is the same).
pub(crate) struct Shared<V> {
    inner: Mutex<Option<Recent<V>>>,
    kept: usize,
}

impl<V> Shared<V> {
    pub(crate) const fn new(kept: usize) -> Self {
        Shared {
            inner: Mutex::new(None),
            kept,
        }
    }

    pub(crate) fn get(&self, key: u64) -> Option<Arc<V>> {
        let mut g = self.inner.lock().ok()?;
        let r = g.as_mut()?;
        if let Some(found) = r.young.get(&key) {
            return Some(found.clone());
        }
        let found = r.old.remove(&key)?;
        r.young.insert(key, found.clone());
        Some(found)
    }

    pub(crate) fn insert(&self, key: u64, value: Arc<V>) {
        if let Ok(mut g) = self.inner.lock() {
            let r = g.get_or_insert_with(|| Recent {
                young: HashMap::new(),
                old: HashMap::new(),
            });
            if r.young.len() >= self.kept {
                r.old = std::mem::take(&mut r.young);
            }
            r.young.insert(key, value);
        }
    }

    pub(crate) fn get_or_build(&self, key: u64, build: impl FnOnce() -> V) -> Arc<V> {
        if let Some(found) = self.get(key) {
            return found;
        }
        let value = Arc::new(build());
        self.insert(key, value.clone());
        value
    }
}
