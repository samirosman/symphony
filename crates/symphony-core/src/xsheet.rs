/// One xsheet column: which drawing is exposed on each frame.
/// Frames are 1-based to match how animators count.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Column {
    pub name: String,
    cells: Vec<Option<String>>,
}

impl Column {
    pub fn new(name: impl Into<String>) -> Self {
        Column {
            name: name.into(),
            cells: Vec::new(),
        }
    }

    pub fn get(&self, frame: u32) -> Option<&str> {
        let i = frame.checked_sub(1)? as usize;
        self.cells.get(i).and_then(|c| c.as_deref())
    }

    /// Set one cell. `None` clears it (an empty cell renders nothing).
    pub fn set(&mut self, frame: u32, drawing: Option<&str>) {
        let Some(i) = frame.checked_sub(1).map(|f| f as usize) else {
            return;
        };
        if self.cells.len() <= i {
            self.cells.resize(i + 1, None);
        }
        self.cells[i] = drawing.map(str::to_owned);
    }

    /// Expose `drawing` for `len` frames starting at `start`
    /// (e.g. on twos: `expose("1", 1, 2); expose("2", 3, 2)`).
    pub fn expose(&mut self, drawing: &str, start: u32, len: u32) {
        for f in start..start + len {
            self.set(f, Some(drawing));
        }
    }

    /// Last frame with a cell (exposed or explicitly empty).
    pub fn len(&self) -> u32 {
        self.cells.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}

/// The exposure sheet: named drawing columns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Xsheet {
    pub columns: Vec<Column>,
}

impl Xsheet {
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    pub fn column_mut(&mut self, name: &str) -> Option<&mut Column> {
        self.columns.iter_mut().find(|c| c.name == name)
    }

    pub fn add_column(&mut self, name: impl Into<String>) -> &mut Column {
        self.columns.push(Column::new(name));
        self.columns.last_mut().expect("just pushed")
    }

    /// Scene length = longest column.
    pub fn length(&self) -> u32 {
        self.columns.iter().map(Column::len).max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_twos() {
        let mut col = Column::new("Arm");
        col.expose("1", 1, 2);
        col.expose("2", 3, 2);
        assert_eq!(col.get(0), None);
        assert_eq!(col.get(2), Some("1"));
        assert_eq!(col.get(3), Some("2"));
        assert_eq!(col.get(5), None);
        assert_eq!(col.len(), 4);
    }
}
