//! The exposure sheet: which drawing each column shows on each frame.
//! Frames are 1-based, as animators count (E2.4). Cells hold drawing IDs,
//! so renaming a drawing never breaks timing (E2.2).

use serde::{Deserialize, Serialize};

use crate::id::Id;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub id: Id,
    pub name: String,
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    cells: Vec<Option<Id>>,
}

impl Column {
    pub fn new(id: Id, name: impl Into<String>) -> Self {
        Column {
            id,
            name: name.into(),
            visible: true,
            locked: false,
            cells: Vec::new(),
        }
    }

    pub fn get(&self, frame: u32) -> Option<Id> {
        let i = frame.checked_sub(1)? as usize;
        self.cells.get(i).copied().flatten()
    }

    /// Set one cell. `None` clears it.
    pub fn set(&mut self, frame: u32, drawing: Option<Id>) {
        let Some(i) = frame.checked_sub(1).map(|f| f as usize) else {
            return;
        };
        if self.cells.len() <= i {
            if drawing.is_none() {
                return;
            }
            self.cells.resize(i + 1, None);
        }
        self.cells[i] = drawing;
        while self.cells.last() == Some(&None) {
            self.cells.pop();
        }
    }

    /// Expose `drawing` for `len` frames starting at `start`.
    pub fn expose(&mut self, drawing: Id, start: u32, len: u32) {
        for f in start..start + len {
            self.set(f, Some(drawing));
        }
    }

    /// Last frame with a drawing.
    pub fn len(&self) -> u32 {
        self.cells.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// First frame of the run of identical cells containing `frame`.
    pub fn run_start(&self, frame: u32) -> u32 {
        let d = self.get(frame);
        let mut f = frame;
        while f > 1 && self.get(f - 1) == d {
            f -= 1;
        }
        f
    }

    /// Last frame of the run of identical cells containing `frame`.
    pub fn run_end(&self, frame: u32) -> u32 {
        let d = self.get(frame);
        let mut f = frame;
        while self.get(f + 1) == d && f < self.len() {
            f += 1;
        }
        f
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Xsheet {
    pub columns: Vec<Column>,
}

impl Xsheet {
    pub fn column(&self, id: Id) -> Option<&Column> {
        self.columns.iter().find(|c| c.id == id)
    }

    pub fn column_mut(&mut self, id: Id) -> Option<&mut Column> {
        self.columns.iter_mut().find(|c| c.id == id)
    }

    pub fn length(&self) -> u32 {
        self.columns.iter().map(Column::len).max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_twos_and_runs() {
        let (a, b) = (Id(1), Id(2));
        let mut col = Column::new(Id(9), "Arm");
        col.expose(a, 1, 2);
        col.expose(b, 3, 2);
        assert_eq!(col.get(0), None);
        assert_eq!(col.get(2), Some(a));
        assert_eq!(col.get(3), Some(b));
        assert_eq!(col.get(5), None);
        assert_eq!(col.len(), 4);
        assert_eq!((col.run_start(4), col.run_end(3)), (3, 4));
    }

    #[test]
    fn clearing_trailing_cells_shrinks_column() {
        let mut col = Column::new(Id(9), "Arm");
        col.expose(Id(1), 1, 3);
        col.set(3, None);
        assert_eq!(col.len(), 2);
    }
}
