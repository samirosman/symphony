//! Stable IDs (ENGINEERING E2.2). An ID never changes and is never reused,
//! so renaming or reordering never breaks a reference.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Id(pub u64);

/// Hands out IDs for one document. Saved with it, so IDs stay unique
/// across sessions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdGen {
    next: u64,
}

impl Default for IdGen {
    fn default() -> Self {
        IdGen { next: 1 }
    }
}

impl IdGen {
    pub fn fresh(&mut self) -> Id {
        let id = Id(self.next);
        self.next += 1;
        id
    }
}
