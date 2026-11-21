//! The open scene plus everything the UI needs around it: undo history,
//! file path, current frame, selection, tools and onion skin. Every change
//! to the document goes through `history` (ENGINEERING E3.1). The rest is
//! UI state and isn't undoable (E3.5).

use std::path::PathBuf;

use symphony_core::anim::Value;
use symphony_core::doc::{Scene, Settings};
use symphony_core::graph::PegParams;
use symphony_core::history::{Edit, History};
use symphony_core::id::Id;
use symphony_core::math::Affine;
use symphony_core::stroke::{Point, Stroke, StrokeKind};

/// Drawing tools, in Tools-toolbar order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Transform,
    Brush,
    Pencil,
    Eraser,
    Paint,
    CloseGap,
    Dropper,
    Hand,
    Zoom,
}

/// Harmony's four Paint tool modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintMode {
    /// Paints or repaints zones, whether or not they're filled.
    Paint,
    /// Only fills empty zones.
    PaintUnpainted,
    /// Only recolours zones that already have colour.
    Repaint,
    /// Removes colour from a zone.
    Unpaint,
}

impl PaintMode {
    pub fn title(self) -> &'static str {
        match self {
            PaintMode::Paint => "Paint",
            PaintMode::PaintUnpainted => "Paint Unpainted",
            PaintMode::Repaint => "Repaint",
            PaintMode::Unpaint => "Unpaint",
        }
    }
}

impl Tool {
    pub const ALL: [Tool; 10] = [
        Tool::Select,
        Tool::Transform,
        Tool::Brush,
        Tool::Pencil,
        Tool::Eraser,
        Tool::Paint,
        Tool::CloseGap,
        Tool::Dropper,
        Tool::Hand,
        Tool::Zoom,
    ];

    /// Which milestone brings this tool, if it isn't built yet.
    pub fn planned_for(self) -> Option<&'static str> {
        match self {
            Tool::Brush
            | Tool::Pencil
            | Tool::Eraser
            | Tool::Hand
            | Tool::Zoom
            | Tool::Select
            | Tool::Transform
            | Tool::Paint
            | Tool::CloseGap
            | Tool::Dropper => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BrushSettings {
    pub min_width: f32,
    pub max_width: f32,
    /// 0 = raw pen, 1 = very smooth.
    pub smoothing: f32,
}

/// A peg's values at the current frame, for the Transform tool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PegValues {
    pub x: f32,
    pub y: f32,
    pub rotation: f32,
    pub scale_x: f32,
    pub scale_y: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Onion {
    pub on: bool,
    pub before: u32,
    pub after: u32,
}

/// Strokes picked with the Select tool, in one drawing.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub drawing: Option<Id>,
    pub strokes: Vec<Id>,
}

pub struct Session {
    pub doc: Scene,
    pub history: History,
    pub path: Option<PathBuf>,
    pub saved_version: u64,
    /// Identifies this session's autosave file (E7.5).
    pub key: String,
    pub frame: u32,
    pub playing: bool,
    pub selected: usize,
    pub tool: Tool,
    pub brush: BrushSettings,
    pub pencil: BrushSettings,
    pub swatch: Id,
    /// Pressure response, as a gamma (E4.1). 1 = linear.
    pub pressure_gamma: f32,
    pub onion: Onion,
    /// Harmony's Brush Size Cursor preference (DESIGN D8.11).
    pub brush_size_cursor: bool,
    pub animate: bool,
    pub selection: Selection,
    pub paint_mode: PaintMode,
    /// Harmony's Show Strokes (K): reveal invisible strokes.
    pub show_strokes: bool,
    /// Harmony's Respect Protected Colours (Shift+S), on by default.
    pub respect_protected: bool,
    /// Select tool options, as in Harmony: Marquee instead of Lasso, and
    /// Easy Drag (drag from anywhere inside the box).
    pub marquee_mode: bool,
    pub easy_drag: bool,
    /// Copied strokes (Cut/Copy/Paste).
    pub clipboard: Vec<Stroke>,
    /// Last action, for the status bar ("Undo Brush stroke").
    pub notice: Option<String>,
}

fn new_key() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("scene-{n:x}")
}

impl Session {
    /// A new scene with one layer called "Drawing", like Harmony's new scene.
    pub fn new_scene() -> Self {
        let mut doc = Scene::new(Settings::default());
        let mut history = History::default();
        let snap = doc.make_layer("Drawing", 0);
        history.apply(&mut doc, "Add layer", vec![Edit::AddLayer(Box::new(snap))]);
        Self::from_doc(doc, None)
    }

    pub fn from_doc(doc: Scene, path: Option<PathBuf>) -> Self {
        let swatch = doc.palette.swatches.first().map(|s| s.id).unwrap_or(Id(0));
        Session {
            saved_version: doc.version,
            doc,
            history: History::default(),
            path,
            key: new_key(),
            frame: 1,
            playing: false,
            selected: 0,
            tool: Tool::Brush,
            brush: BrushSettings {
                min_width: 0.6,
                max_width: 10.0,
                smoothing: 0.4,
            },
            pencil: BrushSettings {
                min_width: 2.0,
                max_width: 3.0,
                smoothing: 0.3,
            },
            swatch,
            pressure_gamma: 1.0,
            onion: Onion {
                on: false,
                before: 1,
                after: 1,
            },
            brush_size_cursor: false,
            selection: Selection::default(),
            paint_mode: PaintMode::Paint,
            show_strokes: false,
            respect_protected: true,
            marquee_mode: false,
            easy_drag: false,
            clipboard: Vec::new(),
            animate: true,
            notice: None,
        }
    }

    /// The demo scene used for screenshots and first impressions.
    pub fn demo() -> Self {
        let mut s = Self::new_scene();
        s.doc.settings.length = 48;
        s.rename_layer(0, "Head");
        s.add_layer_named("Body", 1);
        s.add_layer_named("BG", 2);
        let circle = |cx: f32, cy: f32, r: f32| -> Vec<Point> {
            (0..=48)
                .map(|i| {
                    let t = i as f32 / 48.0 * std::f32::consts::TAU;
                    Point {
                        x: cx + r * t.cos(),
                        y: cy + r * t.sin(),
                        pressure: 0.55 + 0.45 * (t * 1.5).sin().abs(),
                    }
                })
                .collect()
        };
        let line = |pts: &[(f32, f32)]| -> Vec<Point> {
            pts.iter()
                .enumerate()
                .map(|(i, &(x, y))| Point {
                    x,
                    y,
                    pressure: 0.3 + 0.7 * ((i as f32) * 0.9).sin().abs(),
                })
                .collect()
        };
        let art: [(usize, Vec<Vec<Point>>, usize); 3] = [
            (
                0,
                vec![
                    circle(0.0, -180.0, 110.0),
                    circle(-38.0, -195.0, 12.0),
                    circle(38.0, -195.0, 12.0),
                ],
                0,
            ),
            (
                1,
                vec![line(&[
                    (-90.0, -60.0),
                    (-120.0, 120.0),
                    (-60.0, 300.0),
                    (60.0, 300.0),
                    (120.0, 120.0),
                    (90.0, -60.0),
                ])],
                0,
            ),
            (
                2,
                vec![line(&[
                    (-900.0, 380.0),
                    (-300.0, 340.0),
                    (300.0, 360.0),
                    (900.0, 330.0),
                ])],
                4,
            ),
        ];
        for (layer, strokes, swatch) in art {
            let swatch = s.doc.palette.swatches[swatch].id;
            for points in strokes {
                let st = Stroke {
                    id: s.doc.ids.fresh(),
                    kind: StrokeKind::Brush,
                    art: Default::default(),
                    swatch,
                    min_width: 1.0,
                    max_width: 9.0,
                    points,
                };
                s.commit_stroke(layer, 1, st);
            }
            let col = s.doc.layers[layer].column;
            if let Some(d) = s.doc.exposed(layer, 1) {
                if let Some(c) = s.doc.xsheet.column_mut(col) {
                    c.expose(d, 1, 48);
                }
            }
        }
        // Paint the head with skin, so the demo shows ink and paint.
        let line = s.swatch;
        s.swatch = s.doc.palette.swatches[1].id;
        s.paint_at(0, 1, 0.0, -130.0, 1.0);
        s.swatch = line;
        let mut peg = s.doc.peg(0).cloned().unwrap_or_default();
        peg.x.set_key(1, 0.0);
        peg.x.set_key(24, 120.0);
        peg.x.set_key(48, 0.0);
        s.set_peg(0, peg);
        s.history.clear();
        s.saved_version = s.doc.version;
        s
    }

    pub fn dirty(&self) -> bool {
        self.doc.version != self.saved_version
    }

    pub fn name(&self) -> String {
        self.path
            .as_deref()
            .map(symphony_io::project_name)
            .unwrap_or_else(|| "Untitled scene".into())
    }

    pub fn length(&self) -> u32 {
        self.doc.settings.length
    }

    pub fn layer_count(&self) -> usize {
        self.doc.layers.len()
    }

    pub fn layer_name(&self, i: usize) -> &str {
        self.doc.column(i).map(|c| c.name.as_str()).unwrap_or("")
    }

    pub fn layer_visible(&self, i: usize) -> bool {
        self.doc.column(i).is_some_and(|c| c.visible)
    }

    /// Name of the drawing exposed on `layer` at `frame`.
    pub fn drawing_name(&self, layer: usize, frame: u32) -> Option<&str> {
        let id = self.doc.exposed(layer, frame)?;
        self.doc.drawings.get(&id).map(|d| d.name.as_str())
    }

    fn note(&mut self, text: impl Into<String>) {
        self.notice = Some(text.into());
    }

    // ---- Undo ------------------------------------------------------------

    pub fn undo(&mut self) {
        match self.history.undo(&mut self.doc) {
            Some(name) => self.note(format!("Undo {name}")),
            None => self.note("Nothing to undo"),
        }
        self.clamp_selection();
    }

    pub fn redo(&mut self) {
        match self.history.redo(&mut self.doc) {
            Some(name) => self.note(format!("Redo {name}")),
            None => self.note("Nothing to redo"),
        }
        self.clamp_selection();
    }

    /// End the current gesture (drag, erase), so it undoes as one step.
    pub fn end_gesture(&mut self) {
        self.history.seal();
    }

    fn clamp_selection(&mut self) {
        if self.doc.palette.get(self.swatch).is_none() {
            if let Some(first) = self.doc.palette.swatches.first() {
                self.swatch = first.id;
            }
        }
        self.selected = self.selected.min(self.layer_count().saturating_sub(1));
    }

    // ---- Drawing ---------------------------------------------------------

    /// Add a finished stroke. On an empty cell this creates the next
    /// numbered drawing there first, like Harmony, all in one undo step.
    pub fn commit_stroke(&mut self, layer: usize, frame: u32, stroke: Stroke) {
        let Some(col) = self.doc.layers.get(layer).map(|l| l.column) else {
            return;
        };
        let name = match stroke.kind {
            StrokeKind::Brush => "Brush stroke",
            StrokeKind::Pencil => "Pencil stroke",
            StrokeKind::Invisible => "Close gap",
        };
        let mut edits = Vec::new();
        let drawing = match self.doc.exposed(layer, frame) {
            Some(d) => d,
            None => {
                let d = self.doc.make_drawing(col);
                let id = d.id;
                edits.push(Edit::AddDrawing { drawing: d });
                edits.push(Edit::SetCells {
                    column: col,
                    from: frame,
                    before: vec![None],
                    after: vec![Some(id)],
                });
                id
            }
        };
        edits.push(Edit::AddStroke { drawing, stroke });
        self.history.apply(&mut self.doc, name, edits);
    }

    /// Erase strokes by ID from `drawing`. Repeated calls during one eraser
    /// gesture undo as a single step.
    pub fn erase(&mut self, drawing: Id, stroke_ids: &[Id]) {
        for sid in stroke_ids {
            let Some(d) = self.doc.drawings.get(&drawing) else {
                return;
            };
            let Some(index) = d.strokes.iter().position(|s| s.id == *sid) else {
                continue;
            };
            let stroke = d.strokes[index].clone();
            self.history.apply_merged(
                &mut self.doc,
                "Erase",
                "erase",
                Edit::RemoveStroke {
                    drawing,
                    index,
                    stroke,
                },
            );
        }
    }

    /// Harmony's Create Empty Drawing: a new drawing in the current cell.
    pub fn new_drawing(&mut self) {
        let Some(col) = self.doc.layers.get(self.selected).map(|l| l.column) else {
            return;
        };
        let before = self.doc.exposed(self.selected, self.frame);
        let d = self.doc.make_drawing(col);
        let (id, name) = (d.id, d.name.clone());
        self.history.apply(
            &mut self.doc,
            "Create empty drawing",
            vec![
                Edit::AddDrawing { drawing: d },
                Edit::SetCells {
                    column: col,
                    from: self.frame,
                    before: vec![before],
                    after: vec![Some(id)],
                },
            ],
        );
        self.note(format!("Created drawing {name}"));
    }

    // ---- Ink and paint (M5) ---------------------------------------------

    /// The Paint tool at drawing-local (`x`, `y`). `cell` is about one
    /// screen pixel in drawing units, the precision of zone finding.
    pub fn paint_at(&mut self, layer: usize, frame: u32, x: f32, y: f32, cell: f32) {
        use symphony_vector::fill::{find_zone, ZoneError};
        let Some(drawing) = self.doc.exposed(layer, frame) else {
            self.note("There's no drawing here to paint");
            return;
        };
        let Some(d) = self.doc.drawings.get(&drawing) else {
            return;
        };
        let mode = self.paint_mode;
        let swatch = self.swatch;
        // Topmost existing zone under the point.
        let existing = d
            .fills
            .iter()
            .enumerate()
            .rev()
            .find(|(_, f)| f.contains(x, y))
            .map(|(i, f)| (i, f.clone()));
        if let Some((_, fill)) = &existing {
            let changes = match mode {
                PaintMode::Unpaint => true,
                PaintMode::PaintUnpainted => false,
                _ => fill.swatch != swatch,
            };
            if changes && self.is_locked(fill.swatch) {
                self.note_protected(fill.swatch);
                return;
            }
        }
        match (mode, existing) {
            (PaintMode::Unpaint, Some((index, fill))) => {
                self.history.apply(
                    &mut self.doc,
                    "Unpaint",
                    vec![Edit::RemoveFill {
                        drawing,
                        index,
                        fill,
                    }],
                );
            }
            (PaintMode::Paint | PaintMode::Repaint, Some((_, fill))) => {
                if fill.swatch != swatch {
                    self.history.apply(
                        &mut self.doc,
                        "Repaint",
                        vec![Edit::SetFillSwatch {
                            drawing,
                            fill: fill.id,
                            before: fill.swatch,
                            after: swatch,
                        }],
                    );
                }
            }
            (PaintMode::PaintUnpainted, Some(_))
            | (PaintMode::Unpaint | PaintMode::Repaint, None) => {
                // Harmony leaves these alone.
            }
            (PaintMode::Paint | PaintMode::PaintUnpainted, None) => {
                let strokes: Vec<&Stroke> = d.strokes.iter().collect();
                match find_zone(&strokes, x, y, cell) {
                    Ok(contours) => {
                        let fill = symphony_core::stroke::Fill {
                            id: self.doc.ids.fresh(),
                            swatch,
                            art: Default::default(),
                            contours,
                        };
                        self.history.apply(
                            &mut self.doc,
                            "Paint",
                            vec![Edit::AddFill { drawing, fill }],
                        );
                    }
                    Err(ZoneError::Open) => self.note(
                        "This area isn't closed. Close the gap (Close Gap tool) and paint again.",
                    ),
                    Err(ZoneError::OnLine) => self.paint_line(drawing, x, y, cell),
                }
            }
        }
    }

    /// Paint on a line recolours the line itself, as in Harmony.
    fn paint_line(&mut self, drawing: Id, x: f32, y: f32, cell: f32) {
        let swatch = self.swatch;
        let Some(d) = self.doc.drawings.get(&drawing) else {
            return;
        };
        let Some(st) = d.strokes.iter().rev().find(|st| {
            st.kind != StrokeKind::Invisible && symphony_vector::hits(st, x, y, cell * 2.0)
        }) else {
            return;
        };
        if st.swatch == swatch {
            return;
        }
        if self.is_locked(st.swatch) {
            let locked = st.swatch;
            self.note_protected(locked);
            return;
        }
        let before = st.clone();
        let mut after = st.clone();
        after.swatch = swatch;
        self.history.apply(
            &mut self.doc,
            "Paint line",
            vec![Edit::SetStrokes {
                drawing,
                before: vec![before],
                after: vec![after],
            }],
        );
    }

    /// Harmony's Close Gap: from a rough stroke near a gap, join the two
    /// nearest line ends with an invisible stroke. `reach` is how far from
    /// the rough stroke to look for line ends, in drawing units.
    pub fn close_gap(&mut self, layer: usize, frame: u32, rough: &[Point], reach: f32) {
        let Some(drawing) = self.doc.exposed(layer, frame) else {
            self.note("There's no drawing here to close a gap in");
            return;
        };
        let Some(d) = self.doc.drawings.get(&drawing) else {
            return;
        };
        let near = |p: &Point| {
            rough
                .iter()
                .map(|r| (r.x - p.x).hypot(r.y - p.y))
                .fold(f32::MAX, f32::min)
        };
        // Line ends close to the rough stroke.
        let ends: Vec<(usize, Point)> = d
            .strokes
            .iter()
            .enumerate()
            .filter(|(_, st)| st.kind != StrokeKind::Invisible)
            .flat_map(|(i, st)| {
                st.points
                    .first()
                    .into_iter()
                    .chain(st.points.last())
                    .map(move |p| (i, *p))
            })
            .filter(|(_, p)| near(p) <= reach)
            .collect();
        let mut best: Option<(f32, Point, Point)> = None;
        for a in 0..ends.len() {
            for b in a + 1..ends.len() {
                let (pa, pb) = (ends[a].1, ends[b].1);
                let dist = (pa.x - pb.x).hypot(pa.y - pb.y);
                if dist > 1e-3 && best.is_none_or(|(bd, _, _)| dist < bd) {
                    best = Some((dist, pa, pb));
                }
            }
        }
        let Some((_, a, b)) = best else {
            self.note("No line ends near that stroke. Draw it closer to the gap.");
            return;
        };
        let stroke = Stroke {
            id: self.doc.ids.fresh(),
            kind: StrokeKind::Invisible,
            art: Default::default(),
            swatch: self.swatch,
            min_width: 0.0,
            max_width: 0.0,
            points: vec![Point { pressure: 1.0, ..a }, Point { pressure: 1.0, ..b }],
        };
        self.history.apply(
            &mut self.doc,
            "Close gap",
            vec![Edit::AddStroke { drawing, stroke }],
        );
        self.note("Gap closed");
    }

    /// The Dropper: pick up the swatch under the point (a zone or a line).
    pub fn pick_colour(&mut self, layer: usize, frame: u32, x: f32, y: f32, cell: f32) {
        let Some(d) = self
            .doc
            .exposed(layer, frame)
            .and_then(|d| self.doc.drawings.get(&d))
        else {
            return;
        };
        let line = d
            .strokes
            .iter()
            .rev()
            .find(|st| {
                st.kind != StrokeKind::Invisible && symphony_vector::hits(st, x, y, cell * 2.0)
            })
            .map(|st| st.swatch);
        let zone = d
            .fills
            .iter()
            .rev()
            .find(|f| f.contains(x, y))
            .map(|f| f.swatch);
        if let Some(sw) = line.or(zone) {
            self.swatch = sw;
            let name = self
                .doc
                .palette
                .get(sw)
                .map(|w| w.name.clone())
                .unwrap_or_default();
            self.note(format!("Picked {name}"));
        }
    }

    // ---- Selection (Select tool) ------------------------------------------

    /// Drop the selection if it no longer points at the current drawing.
    pub fn validate_selection(&mut self) {
        let current = self.doc.exposed(self.selected, self.frame);
        if self.selection.drawing != current || current.is_none() {
            self.selection = Selection {
                drawing: current,
                strokes: Vec::new(),
            };
            return;
        }
        if let Some(d) = current.and_then(|d| self.doc.drawings.get(&d)) {
            self.selection
                .strokes
                .retain(|id| d.strokes.iter().any(|st| st.id == *id));
        }
    }

    pub fn selected_strokes(&self) -> Vec<&Stroke> {
        let Some(d) = self
            .selection
            .drawing
            .and_then(|d| self.doc.drawings.get(&d))
        else {
            return Vec::new();
        };
        d.strokes
            .iter()
            .filter(|st| self.selection.strokes.contains(&st.id))
            .collect()
    }

    pub fn select_all(&mut self) {
        self.validate_selection();
        if let Some(d) = self
            .selection
            .drawing
            .and_then(|d| self.doc.drawings.get(&d))
        {
            self.selection.strokes = d.strokes.iter().map(|st| st.id).collect();
        }
    }

    pub fn deselect(&mut self) {
        self.selection.strokes.clear();
    }

    /// Apply `m` (drawing-local) to the selected strokes as one undo step.
    pub fn transform_selection(&mut self, m: Affine, name: &str) {
        let Some(drawing) = self.selection.drawing else {
            return;
        };
        let before: Vec<Stroke> = self.selected_strokes().into_iter().cloned().collect();
        if before.is_empty() {
            return;
        }
        let after = before
            .iter()
            .map(|st| symphony_vector::transformed(st, m))
            .collect();
        self.history.apply(
            &mut self.doc,
            name,
            vec![Edit::SetStrokes {
                drawing,
                before,
                after,
            }],
        );
    }

    /// Flip the selection around its centre.
    pub fn flip_selection(&mut self, horizontal: bool) {
        let Some((lo, hi)) = symphony_vector::bounds_of(self.selected_strokes()) else {
            return;
        };
        let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
        let (sx, sy) = if horizontal { (-1.0, 1.0) } else { (1.0, -1.0) };
        let m = Affine::translate(cx, cy)
            .compose(Affine::scale(sx, sy))
            .compose(Affine::translate(-cx, -cy));
        self.transform_selection(
            m,
            if horizontal {
                "Flip horizontal"
            } else {
                "Flip vertical"
            },
        );
    }

    pub fn delete_selection(&mut self) {
        let Some(drawing) = self.selection.drawing else {
            return;
        };
        let ids = std::mem::take(&mut self.selection.strokes);
        let Some(d) = self.doc.drawings.get(&drawing) else {
            return;
        };
        // Remove from the back so earlier indices stay valid on undo.
        let mut edits: Vec<Edit> = d
            .strokes
            .iter()
            .enumerate()
            .filter(|(_, st)| ids.contains(&st.id))
            .map(|(index, st)| Edit::RemoveStroke {
                drawing,
                index,
                stroke: st.clone(),
            })
            .collect();
        edits.reverse();
        self.history.apply(&mut self.doc, "Delete", edits);
    }

    pub fn copy_selection(&mut self) {
        self.clipboard = self.selected_strokes().into_iter().cloned().collect();
        let n = self.clipboard.len();
        if n > 0 {
            self.note(format!(
                "Copied {n} stroke{}",
                if n == 1 { "" } else { "s" }
            ));
        }
    }

    pub fn cut_selection(&mut self) {
        self.copy_selection();
        self.delete_selection();
    }

    /// Paste into the current cell (creating a drawing there if it's empty),
    /// with fresh IDs, and select what was pasted.
    pub fn paste(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        let Some(col) = self.doc.layers.get(self.selected).map(|l| l.column) else {
            return;
        };
        let mut edits = Vec::new();
        let drawing = match self.doc.exposed(self.selected, self.frame) {
            Some(d) => d,
            None => {
                let d = self.doc.make_drawing(col);
                let id = d.id;
                edits.push(Edit::AddDrawing { drawing: d });
                edits.push(Edit::SetCells {
                    column: col,
                    from: self.frame,
                    before: vec![None],
                    after: vec![Some(id)],
                });
                id
            }
        };
        let mut ids = Vec::new();
        for st in self.clipboard.clone() {
            let mut st = st;
            if self.doc.palette.get(st.swatch).is_none() {
                st.swatch = self.swatch;
            }
            st.id = self.doc.ids.fresh();
            ids.push(st.id);
            edits.push(Edit::AddStroke {
                drawing,
                stroke: st,
            });
        }
        self.history.apply(&mut self.doc, "Paste", edits);
        self.selection = Selection {
            drawing: Some(drawing),
            strokes: ids,
        };
    }

    // ---- Transform tool (layers, through their pegs) ----------------------

    /// Change the selected layer's peg in one gesture step. In Animate mode
    /// values are keyed on the current frame; otherwise constants change.
    pub fn adjust_peg(&mut self, layer: usize, f: impl FnOnce(&mut PegValues)) {
        let Some(mut p) = self.doc.peg(layer).cloned() else {
            return;
        };
        let frame = self.frame;
        let mut v = PegValues {
            x: p.x.at(frame),
            y: p.y.at(frame),
            rotation: p.rotation.at(frame),
            scale_x: p.scale_x.as_ref().map_or(1.0, |s| s.at(frame)),
            scale_y: p.scale_y.as_ref().map_or(1.0, |s| s.at(frame)),
        };
        let old = v;
        f(&mut v);
        let animate = self.animate;
        let set = |value: &mut Value, n: f32| match value {
            Value::Constant(c) if !animate => *c = n,
            _ => value.set_key(frame, n),
        };
        if v.x != old.x {
            set(&mut p.x, v.x);
        }
        if v.y != old.y {
            set(&mut p.y, v.y);
        }
        if v.rotation != old.rotation {
            set(&mut p.rotation, v.rotation);
        }
        if v.scale_x != old.scale_x {
            set(p.scale_x.get_or_insert(Value::Constant(1.0)), v.scale_x);
        }
        if v.scale_y != old.scale_y {
            set(p.scale_y.get_or_insert(Value::Constant(1.0)), v.scale_y);
        }
        self.set_peg(layer, p);
    }

    /// Put the peg's pivot at `pivot` (drawing space) if the peg has never
    /// been rotated or scaled, so turning a drawing happens around its middle.
    pub fn ensure_pivot(&mut self, layer: usize, pivot: (f32, f32)) {
        let Some(p) = self.doc.peg(layer).cloned() else {
            return;
        };
        let untouched = p.pivot == (0.0, 0.0)
            && matches!(p.rotation, Value::Constant(r) if r == 0.0)
            && p.scale_x.is_none()
            && p.scale_y.is_none();
        if untouched {
            let mut p = p;
            p.pivot = pivot;
            self.set_peg(layer, p);
        }
    }

    // ---- Timing ----------------------------------------------------------

    /// Hold the current drawing one frame longer (+1) or shorter (-1),
    /// shifting everything after it.
    pub fn change_exposure(&mut self, delta: i32) {
        let Some(col) = self.doc.layers.get(self.selected).map(|l| l.column) else {
            return;
        };
        let Some(d) = self.doc.exposed(self.selected, self.frame) else {
            self.note("There's no drawing in this cell to extend");
            return;
        };
        if delta > 0 {
            if let Some(e) = Edit::splice(&self.doc, col, self.frame, 0, vec![Some(d)]) {
                self.history
                    .apply(&mut self.doc, "Increase exposure", vec![e]);
            }
        } else {
            let column = self.doc.xsheet.column(col);
            let run = column
                .map(|c| c.run_end(self.frame) - c.run_start(self.frame) + 1)
                .unwrap_or(1);
            if run <= 1 {
                self.note("This drawing is only exposed for one frame");
                return;
            }
            if let Some(e) = Edit::splice(&self.doc, col, self.frame, 1, vec![]) {
                self.history
                    .apply(&mut self.doc, "Decrease exposure", vec![e]);
            }
        }
    }

    pub fn set_length(&mut self, length: u32) {
        let before = self.doc.settings.length;
        if before != length {
            self.history.apply_merged(
                &mut self.doc,
                "Change scene length",
                "length",
                Edit::SetLength {
                    before,
                    after: length,
                },
            );
        }
    }

    /// Distinct drawings before and after the current one on the selected
    /// layer, nearest first, for onion skin.
    pub fn onion_drawings(&self) -> (Vec<Id>, Vec<Id>) {
        let Some(col) = self.doc.column(self.selected) else {
            return (vec![], vec![]);
        };
        let current = col.get(self.frame);
        let walk = |frames: &mut dyn Iterator<Item = u32>, n: u32| {
            let mut out: Vec<Id> = Vec::new();
            for f in frames {
                if let Some(d) = col.get(f) {
                    if Some(d) != current && !out.contains(&d) {
                        out.push(d);
                        if out.len() as u32 >= n {
                            break;
                        }
                    }
                }
            }
            out
        };
        let before = walk(&mut (1..self.frame).rev(), self.onion.before);
        let after = walk(&mut (self.frame + 1..=self.length()), self.onion.after);
        (before, after)
    }

    /// Add or remove one earlier (`before`) or later drawing from onion skin.
    pub fn onion_step(&mut self, before: bool, delta: i32) {
        let n = if before {
            &mut self.onion.before
        } else {
            &mut self.onion.after
        };
        *n = (*n as i32 + delta).clamp(0, 9) as u32;
        self.onion.on = true;
    }

    /// Harmony's "Onion Skin: N Previous/Next" shortcuts set a count and show it.
    pub fn onion_set(&mut self, before: bool, n: u32) {
        if before {
            self.onion.before = n;
        } else {
            self.onion.after = n;
        }
        self.onion.on = true;
    }

    // ---- Layers ----------------------------------------------------------

    fn add_layer_named(&mut self, name: &str, index: usize) {
        let snap = self.doc.make_layer(name, index);
        self.history.apply(
            &mut self.doc,
            "Add layer",
            vec![Edit::AddLayer(Box::new(snap))],
        );
    }

    /// Add a layer above the selected one, like Harmony's Add Drawing Layer.
    pub fn add_layer(&mut self) {
        let n = (1..)
            .find(|n| (0..self.layer_count()).all(|i| self.layer_name(i) != format!("Drawing {n}")))
            .unwrap_or(1);
        let index = self.selected.min(self.layer_count());
        self.add_layer_named(&format!("Drawing {n}"), index);
        self.selected = index;
    }

    pub fn delete_layer(&mut self, i: usize) {
        if self.layer_count() <= 1 {
            self.note("A scene needs at least one layer");
            return;
        }
        if let Some(snap) = self.doc.snapshot_layer(i) {
            let name = self.layer_name(i).to_owned();
            self.history.apply(
                &mut self.doc,
                "Delete layer",
                vec![Edit::RemoveLayer(Box::new(snap))],
            );
            self.clamp_selection();
            self.note(format!("Deleted {name}. Undo brings it back."));
        }
    }

    pub fn rename_layer(&mut self, i: usize, name: &str) {
        let name = name.trim();
        let Some(col) = self.doc.column(i) else {
            return;
        };
        if name.is_empty() || name == col.name {
            return;
        }
        let (column, before) = (col.id, col.name.clone());
        self.history.apply(
            &mut self.doc,
            "Rename layer",
            vec![Edit::Rename {
                column,
                before,
                after: name.into(),
            }],
        );
    }

    pub fn toggle_visible(&mut self, i: usize) {
        let Some(col) = self.doc.column(i) else {
            return;
        };
        let (column, before) = (col.id, col.visible);
        let name = if before { "Hide layer" } else { "Show layer" };
        self.history.apply(
            &mut self.doc,
            name,
            vec![Edit::SetVisible {
                column,
                before,
                after: !before,
            }],
        );
    }

    // ---- Pegs and colour -------------------------------------------------

    /// Set a peg's parameters. Repeated calls during one drag merge.
    pub fn set_peg(&mut self, layer: usize, after: PegParams) {
        let Some(l) = self.doc.layers.get(layer).copied() else {
            return;
        };
        let Some(before) = self.doc.peg(layer).cloned() else {
            return;
        };
        if before != after {
            self.history.apply_merged(
                &mut self.doc,
                "Move peg",
                &format!("peg-{}", l.peg.0),
                Edit::SetPeg {
                    node: l.peg,
                    before,
                    after,
                },
            );
        }
    }

    /// Change one peg value at the current frame: a key in Animate mode,
    /// otherwise the constant value.
    pub fn set_peg_value(&mut self, layer: usize, pick: fn(&mut PegParams) -> &mut Value, v: f32) {
        let Some(mut p) = self.doc.peg(layer).cloned() else {
            return;
        };
        let value = pick(&mut p);
        match value {
            Value::Constant(c) if !self.animate => *c = v,
            _ => value.set_key(self.frame, v),
        }
        self.set_peg(layer, p);
    }

    fn apply_palette(
        &mut self,
        name: &str,
        palette: symphony_core::palette::Palette,
        mut edits: Vec<Edit>,
    ) {
        edits.push(Edit::SetPalette {
            before: self.doc.palette.clone(),
            after: palette,
        });
        self.history.apply(&mut self.doc, name, edits);
    }

    pub fn add_swatch(&mut self) {
        let rgb = self.doc.palette.get(self.swatch).map_or([0; 3], |s| s.rgb);
        let id = self.doc.ids.fresh();
        let mut palette = self.doc.palette.clone();
        palette.swatches.push(symphony_core::palette::Swatch {
            id,
            name: "New colour".into(),
            rgb,
            protected: false,
        });
        self.apply_palette("Add colour", palette, Vec::new());
        self.swatch = id;
    }

    pub fn rename_swatch(&mut self, id: Id, name: &str) {
        let mut palette = self.doc.palette.clone();
        let Some(swatch) = palette.get_mut(id) else {
            return;
        };
        let name = name.trim();
        if name.is_empty() || swatch.name == name {
            return;
        }
        swatch.name = name.into();
        self.apply_palette("Rename colour", palette, Vec::new());
    }

    pub fn move_swatch(&mut self, id: Id, delta: isize) {
        let mut palette = self.doc.palette.clone();
        let Some(index) = palette.swatches.iter().position(|sw| sw.id == id) else {
            return;
        };
        let Some(next) = index
            .checked_add_signed(delta)
            .filter(|i| *i < palette.swatches.len())
        else {
            return;
        };
        palette.swatches.swap(index, next);
        self.apply_palette("Move colour", palette, Vec::new());
    }

    pub fn delete_swatch(&mut self, id: Id, replacement: Id) {
        let mut palette = self.doc.palette.clone();
        if id == replacement || palette.get(id).is_none() || palette.get(replacement).is_none() {
            return;
        }
        palette.swatches.retain(|sw| sw.id != id);
        let mut edits = Vec::new();
        for (&drawing, d) in &self.doc.drawings {
            for st in d.strokes.iter().filter(|s| s.swatch == id) {
                edits.push(Edit::SetStrokeSwatch {
                    drawing,
                    stroke: st.id,
                    before: id,
                    after: replacement,
                });
            }
            for fill in d.fills.iter().filter(|f| f.swatch == id) {
                edits.push(Edit::SetFillSwatch {
                    drawing,
                    fill: fill.id,
                    before: id,
                    after: replacement,
                });
            }
        }
        self.apply_palette("Remove colour", palette, edits);
        if self.swatch == id {
            self.swatch = replacement;
        }
    }

    /// Does painting have to leave this swatch's zones and lines alone?
    pub fn is_locked(&self, swatch: Id) -> bool {
        self.respect_protected && self.doc.palette.get(swatch).is_some_and(|sw| sw.protected)
    }

    fn note_protected(&mut self, swatch: Id) {
        let name = self
            .doc
            .palette
            .get(swatch)
            .map_or_else(String::new, |sw| sw.name.clone());
        self.note(format!(
            "{name} is protected. Unprotect it in the Colour view, or turn off Respect Protected Colours (Shift+S)."
        ));
    }

    /// Harmony's Protect Colour, from the Colour view. Undoable, like any
    /// palette change.
    pub fn toggle_protect_swatch(&mut self, id: Id) {
        let mut palette = self.doc.palette.clone();
        let Some(sw) = palette.get_mut(id) else {
            return;
        };
        sw.protected = !sw.protected;
        let name = if sw.protected {
            "Protect colour"
        } else {
            "Unprotect colour"
        };
        self.apply_palette(name, palette, Vec::new());
    }

    pub fn toggle_respect_protected(&mut self) {
        self.respect_protected = !self.respect_protected;
        self.note(if self.respect_protected {
            "Respecting protected colours"
        } else {
            "Protected colours can be painted over"
        });
    }

    /// Recolour a swatch. Every stroke using it changes (E2.3).
    pub fn set_swatch(&mut self, swatch: Id, rgb: [u8; 3]) {
        let before = self.doc.swatch_rgb(swatch);
        if before != rgb {
            self.history.apply_merged(
                &mut self.doc,
                "Edit swatch",
                &format!("swatch-{}", swatch.0),
                Edit::SetSwatch {
                    swatch,
                    before,
                    after: rgb,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &mut Session, x0: f32, x1: f32) -> Stroke {
        let swatch = s.swatch;
        Stroke {
            id: s.doc.ids.fresh(),
            kind: StrokeKind::Brush,
            art: Default::default(),
            swatch,
            min_width: 2.0,
            max_width: 2.0,
            points: vec![
                Point {
                    x: x0,
                    y: 0.0,
                    pressure: 1.0,
                },
                Point {
                    x: x1,
                    y: 0.0,
                    pressure: 1.0,
                },
            ],
        }
    }

    #[test]
    fn flip_selection_undoes_in_one_step() {
        let mut s = Session::new_scene();
        let st = line(&mut s, 0.0, 10.0);
        s.commit_stroke(0, 1, st);
        s.select_all();
        assert_eq!(s.selection.strokes.len(), 1);
        s.flip_selection(true);
        let d = s.doc.exposed(0, 1).unwrap();
        assert_eq!(s.doc.drawings[&d].strokes[0].points[0].x, 10.0);
        s.undo();
        assert_eq!(s.doc.drawings[&d].strokes[0].points[0].x, 0.0);
    }

    #[test]
    fn copy_paste_into_empty_cell_creates_a_drawing() {
        let mut s = Session::new_scene();
        let st = line(&mut s, 0.0, 10.0);
        s.commit_stroke(0, 1, st);
        s.select_all();
        s.copy_selection();
        s.frame = 2;
        s.paste();
        let d2 = s.doc.exposed(0, 2).expect("paste created a drawing");
        assert_ne!(Some(d2), s.doc.exposed(0, 1));
        assert_eq!(s.doc.drawings[&d2].strokes.len(), 1);
        // Pasted strokes get fresh IDs and are selected.
        assert_eq!(s.selection.strokes, vec![s.doc.drawings[&d2].strokes[0].id]);
        assert_ne!(s.doc.drawings[&d2].strokes[0].id, s.clipboard[0].id);
    }

    #[test]
    fn delete_selection_restores_order_on_undo() {
        let mut s = Session::new_scene();
        for x in [0.0, 20.0, 40.0] {
            let st = line(&mut s, x, x + 5.0);
            s.commit_stroke(0, 1, st);
        }
        let d = s.doc.exposed(0, 1).unwrap();
        let before: Vec<Id> = s.doc.drawings[&d].strokes.iter().map(|st| st.id).collect();
        s.validate_selection();
        s.selection.strokes = vec![before[0], before[2]];
        s.delete_selection();
        assert_eq!(s.doc.drawings[&d].strokes.len(), 1);
        s.undo();
        let after: Vec<Id> = s.doc.drawings[&d].strokes.iter().map(|st| st.id).collect();
        assert_eq!(after, before);
    }

    fn square_drawing(s: &mut Session) {
        let pts = [
            (10.0, 10.0),
            (90.0, 10.0),
            (90.0, 90.0),
            (10.0, 90.0),
            (10.0, 10.0),
        ];
        let swatch = s.swatch;
        let st = Stroke {
            id: s.doc.ids.fresh(),
            kind: StrokeKind::Brush,
            art: Default::default(),
            swatch,
            min_width: 2.0,
            max_width: 2.0,
            points: pts
                .iter()
                .map(|&(x, y)| Point {
                    x,
                    y,
                    pressure: 1.0,
                })
                .collect(),
        };
        s.commit_stroke(0, 1, st);
    }

    fn fills(s: &Session) -> Vec<symphony_core::stroke::Fill> {
        let d = s.doc.exposed(0, 1).unwrap();
        s.doc.drawings[&d].fills.clone()
    }

    #[test]
    fn palette_edits_preserve_and_restore_art_references() {
        let mut s = Session::new_scene();
        let original = s.swatch;
        square_drawing(&mut s);
        s.paint_mode = PaintMode::Paint;
        s.paint_at(0, 1, 50.0, 50.0, 1.0);
        s.add_swatch();
        let replacement = s.swatch;
        s.rename_swatch(replacement, "Shadow ink");
        s.move_swatch(replacement, -1);
        let before = s.doc.palette.clone();
        s.delete_swatch(original, replacement);
        let d = s.doc.exposed(0, 1).unwrap();
        assert_eq!(s.doc.drawings[&d].strokes[0].swatch, replacement);
        assert_eq!(fills(&s)[0].swatch, replacement);
        assert!(s.doc.palette.get(original).is_none());
        s.undo();
        assert_eq!(s.doc.palette, before);
        assert_eq!(s.doc.drawings[&d].strokes[0].swatch, original);
        assert_eq!(fills(&s)[0].swatch, original);
        s.redo();
        assert_eq!(fills(&s)[0].swatch, replacement);
    }

    #[test]
    fn paint_modes_follow_harmony() {
        let mut s = Session::new_scene();
        square_drawing(&mut s);
        let (a, b) = (s.doc.palette.swatches[1].id, s.doc.palette.swatches[2].id);

        s.paint_mode = PaintMode::Repaint;
        s.swatch = a;
        s.paint_at(0, 1, 50.0, 50.0, 1.0);
        assert!(fills(&s).is_empty(), "Repaint leaves empty zones alone");

        s.paint_mode = PaintMode::Paint;
        s.paint_at(0, 1, 50.0, 50.0, 1.0);
        assert_eq!(fills(&s).len(), 1);
        assert_eq!(fills(&s)[0].swatch, a);

        s.paint_mode = PaintMode::PaintUnpainted;
        s.swatch = b;
        s.paint_at(0, 1, 50.0, 50.0, 1.0);
        assert_eq!(
            fills(&s)[0].swatch,
            a,
            "Paint Unpainted leaves painted zones alone"
        );

        s.paint_mode = PaintMode::Repaint;
        s.paint_at(0, 1, 50.0, 50.0, 1.0);
        assert_eq!(fills(&s)[0].swatch, b);
        s.undo();
        assert_eq!(fills(&s)[0].swatch, a, "repaint undoes");

        s.paint_mode = PaintMode::Unpaint;
        s.paint_at(0, 1, 50.0, 50.0, 1.0);
        assert!(fills(&s).is_empty());
    }

    #[test]
    fn protected_colours_are_left_alone_while_respected() {
        let mut s = Session::new_scene();
        let ink = s.swatch;
        square_drawing(&mut s);
        let (a, b) = (s.doc.palette.swatches[1].id, s.doc.palette.swatches[2].id);
        s.paint_mode = PaintMode::Paint;
        s.swatch = a;
        s.paint_at(0, 1, 50.0, 50.0, 1.0);
        s.toggle_protect_swatch(a);
        s.toggle_protect_swatch(ink);
        assert!(s.doc.palette.get(a).unwrap().protected);

        s.swatch = b;
        s.paint_at(0, 1, 50.0, 50.0, 1.0);
        assert_eq!(fills(&s)[0].swatch, a, "Paint skips a protected zone");
        s.paint_mode = PaintMode::Unpaint;
        s.paint_at(0, 1, 50.0, 50.0, 1.0);
        assert_eq!(fills(&s).len(), 1, "Unpaint skips a protected zone");
        s.paint_mode = PaintMode::Paint;
        s.paint_at(0, 1, 10.0, 50.0, 1.0);
        let d = s.doc.exposed(0, 1).unwrap();
        assert_eq!(s.doc.drawings[&d].strokes[0].swatch, ink, "protected line");

        s.toggle_respect_protected();
        s.paint_at(0, 1, 50.0, 50.0, 1.0);
        assert_eq!(fills(&s)[0].swatch, b, "not respected: repaints");

        s.undo();
        s.undo();
        assert!(!s.doc.palette.get(ink).unwrap().protected, "protect undoes");
    }

    #[test]
    fn close_gap_makes_an_open_shape_paintable() {
        let mut s = Session::new_scene();
        let swatch = s.swatch;
        let open = Stroke {
            id: s.doc.ids.fresh(),
            kind: StrokeKind::Brush,
            art: Default::default(),
            swatch,
            min_width: 2.0,
            max_width: 2.0,
            points: [(10.0, 10.0), (90.0, 10.0), (90.0, 90.0), (10.0, 90.0)]
                .iter()
                .map(|&(x, y)| Point {
                    x,
                    y,
                    pressure: 1.0,
                })
                .collect(),
        };
        s.commit_stroke(0, 1, open);
        s.paint_at(0, 1, 50.0, 50.0, 1.0);
        assert!(fills(&s).is_empty(), "an open shape can't be painted");
        // A rough stroke near the gap on the left side.
        let rough = [
            Point {
                x: 14.0,
                y: 20.0,
                pressure: 1.0,
            },
            Point {
                x: 14.0,
                y: 80.0,
                pressure: 1.0,
            },
        ];
        s.close_gap(0, 1, &rough, 40.0);
        s.paint_at(0, 1, 50.0, 50.0, 1.0);
        assert_eq!(fills(&s).len(), 1, "after Close Gap it can");
    }

    #[test]
    fn transform_tool_move_keys_the_peg_in_animate_mode() {
        let mut s = Session::new_scene();
        s.animate = true;
        s.frame = 5;
        s.adjust_peg(0, |v| v.x += 30.0);
        s.end_gesture();
        let p = s.doc.peg(0).unwrap();
        assert!(p.has_key(5));
        assert_eq!(p.x.at(5), 30.0);
        s.undo();
        assert_eq!(s.doc.peg(0).unwrap().x.at(5), 0.0);
    }
}
