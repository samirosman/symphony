//! Timeline and Xsheet: two views over the same exposure data.
//! Click a cell to go to that frame and select that layer (D8.9).
//! Double-click a layer name to rename it; right-click for layer actions.
//! Double-click a sound track to open the Sound editor.
//! The amber playhead and frame tag are Symphony's signature (D3.9).

use eframe::egui::{self, Align2, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};
use symphony_ui::icons::{self, Icon};
use symphony_ui::{theme, widgets};

use symphony_core::doc::SoundClip;
use symphony_core::id::Id;
use symphony_core::sound;

use crate::keymap::{self, Action, Profile};
use crate::session::{InsertExposure, Session};
use crate::sound::SoundState;
use crate::views::sound_editor::SoundEditor;
use symphony_core::graph::{NodeId, NodeKind};

#[derive(Default)]
pub struct TimingState {
    renaming: Option<(usize, String)>,
    menu_row: Option<usize>,
    /// The node a right-click menu is for (a layer's drawing node too).
    menu_node: Option<NodeId>,
    show_unanimated_nodes: bool,
    /// Hide peg rows, so the Timeline reads as a plain layer stack.
    /// Saved with the app's workspace settings.
    pub hide_pegs: bool,
    /// Parameters column: drawing name being typed, per row.
    swap_text: Option<(usize, String)>,
    /// Parameters column drag: row, starting drawing index, pixels dragged.
    swap_drag: Option<(usize, usize, f32)>,
    /// Sound track being renamed, and the name typed so far.
    renaming_sound: Option<(Id, String)>,
    /// Sound track whose context menu is open.
    sound_menu: Option<Id>,
    /// A sound track being moved or trimmed.
    sound_drag: Option<SoundDrag>,
    /// The Sound editor, when open.
    pub sound_editor: Option<SoundEditor>,
    /// A layer being dragged to a new place in the stack (its row).
    layer_drag: Option<usize>,
    /// The width of a frame in the Timeline, once Ctrl+wheel has changed
    /// it.
    frame_w: Option<f32>,
    scrubbing: bool,
    /// Layer order last frame (columns), and layers that just arrived or
    /// moved with when, so their row glows briefly.
    last_order: Vec<symphony_core::id::Id>,
    arrived: Vec<(symphony_core::id::Id, f64)>,
}

/// Which part of a sound block a drag holds.
#[derive(Clone, Copy, PartialEq)]
enum SoundPart {
    /// The middle: moves the track.
    Body,
    /// The left edge: trims the start.
    Start,
    /// The right edge: trims the end.
    End,
}

struct SoundDrag {
    part: SoundPart,
    /// Pointer x where the drag started.
    x: f32,
    /// The track as it was then; each step is worked out from it.
    original: SoundClip,
}

/// Pixels on each side of a sound block's edge that grab the edge.
const SOUND_EDGE: f32 = 6.0;

/// Width of the Timeline's Parameters field (the exposed drawing's name).
const PARAM_W: f32 = 48.0;
/// Pixels of drag per drawing when scrubbing the Parameters field.
const PARAM_STEP: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineAction {
    SetKey,
    DeleteKey,
    InsertNew,
    Duplicate,
    Extend,
    InsertInbetween,
    DeleteLayer,
}

impl TimelineAction {
    pub const ALL: [Self; 7] = [
        Self::SetKey,
        Self::DeleteKey,
        Self::InsertNew,
        Self::Duplicate,
        Self::Extend,
        Self::InsertInbetween,
        Self::DeleteLayer,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::SetKey => "Set Keyframe",
            Self::DeleteKey => "Delete Keyframe",
            Self::InsertNew => "Insert New Drawing Frame",
            Self::Duplicate => "Duplicate Drawing Frame",
            Self::Extend => "Extend Exposure",
            Self::InsertInbetween => "Insert In-between Frame",
            Self::DeleteLayer => "Delete Layer",
        }
    }

    pub fn keywords(self) -> &'static str {
        match self {
            Self::SetKey => "add create key pose animation timeline",
            Self::DeleteKey => "remove key pose animation timeline",
            Self::InsertNew => "add new frame drawing blank empty cel cell",
            Self::Duplicate => "copy clone duplicate frame exposure independent cel",
            Self::Extend => "hold extend increase exposure length one frame",
            Self::InsertInbetween => "insert new frame all layers hold timing scene",
            Self::DeleteLayer => "remove layer timeline",
        }
    }

    fn enabled(self, s: &Session) -> bool {
        match self {
            Self::SetKey => !s.doc.layers.is_empty() || s.selected_node.is_some(),
            Self::DeleteKey => super::node::selected_has_key(s),
            Self::InsertNew => s.can_insert_exposure(),
            Self::Duplicate | Self::Extend => {
                s.can_insert_exposure() && s.doc.exposed(s.selected, s.frame).is_some()
            }
            Self::InsertInbetween => s.length() < 100_000,
            Self::DeleteLayer => s.layer_count() > 1,
        }
    }

    pub fn run(self, s: &mut Session) {
        match self {
            Self::SetKey => super::node::set_selected_key(s, false),
            Self::DeleteKey => super::node::set_selected_key(s, true),
            Self::InsertNew => s.insert_exposure(InsertExposure::New),
            Self::Duplicate => s.insert_exposure(InsertExposure::Duplicate),
            Self::Extend => s.change_exposure(1),
            Self::InsertInbetween => s.insert_inbetweens(1),
            Self::DeleteLayer => s.delete_layer(s.selected),
        }
    }

    pub fn menu(self, ui: &mut egui::Ui, s: &mut Session) {
        let button = egui::Button::new(self.label());
        let button = if self == Self::SetKey {
            button.shortcut_text("K in Timeline")
        } else {
            button
        };
        if ui.add_enabled(self.enabled(s), button).clicked() {
            self.run(s);
            ui.close();
        }
    }
}

fn frame_menu(ui: &mut egui::Ui, s: &mut Session) {
    for action in [
        TimelineAction::InsertNew,
        TimelineAction::Duplicate,
        TimelineAction::Extend,
    ] {
        action.menu(ui, s);
    }
    ui.menu_button("Reuse Existing Drawing", |ui| {
        let drawings = s.element_drawings(s.selected);
        if drawings.is_empty() {
            ui.label("This layer has no drawings yet.");
        }
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .show(ui, |ui| {
                for (id, name) in drawings {
                    if ui
                        .add_enabled(s.can_insert_exposure(), egui::Button::new(name))
                        .clicked()
                    {
                        s.insert_exposure(InsertExposure::Reuse(id));
                        ui.close();
                    }
                }
            });
    });
    ui.separator();
    TimelineAction::InsertInbetween.menu(ui, s);
    TimelineAction::DeleteKey.menu(ui, s);
    ui.separator();
    if ui
        .button("Clear exposure")
        .on_hover_text("Empty this cell; the drawing stays in the layer's drawings.")
        .clicked()
    {
        s.clear_cell();
        ui.close();
    }
    if ui
        .button("Remove frame")
        .on_hover_text("Take out this cell and pull the rest of the layer one frame earlier.")
        .clicked()
    {
        s.remove_frame();
        ui.close();
    }
}

/// View header: transport, timing fields and layer buttons (D3.2).
pub fn header(ui: &mut egui::Ui, s: &mut Session, profile: Profile) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(theme::CARD_PAD, 4))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let available = ui.available_width();
                let icon = if s.playing { Icon::Pause } else { Icon::Play };
                if icons::button(
                    ui,
                    icon,
                    s.playing,
                    true,
                    &keymap::tooltip(profile, Action::PlayPause),
                )
                .clicked()
                {
                    s.toggle_playback();
                }
                ui.spacing_mut().item_spacing.x = 6.0;
                let len = s.length();
                // Keep scene settings in the menu when the panel is narrow.
                let compact = available < 535.0;
                ui.add(
                    egui::DragValue::new(&mut s.frame)
                        .range(1..=len)
                        .prefix("Frame "),
                );
                let keyed = super::node::selected_has_key(s);
                let (icon, action, tooltip) = if keyed {
                    (
                        Icon::RemoveKeyframe,
                        TimelineAction::DeleteKey,
                        "Remove keyframe",
                    )
                } else {
                    (
                        Icon::AddKeyframe,
                        TimelineAction::SetKey,
                        "Set keyframe (K in Timeline)",
                    )
                };
                if icons::button(ui, icon, keyed, action.enabled(s), tooltip).clicked() {
                    action.run(s);
                }
                ui.menu_button("Frames", |ui| {
                    frame_menu(ui, s);
                    ui.separator();
                    ui.menu_button("Timing options", |ui| timing_options(ui, s));
                });
                if !compact {
                    scene_timing(ui, s);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if icons::button(
                            ui,
                            Icon::Trash,
                            false,
                            s.layer_count() > 1,
                            "Delete layer",
                        )
                        .clicked()
                        {
                            s.delete_layer(s.selected);
                        }
                        if icons::button(
                            ui,
                            Icon::Plus,
                            false,
                            true,
                            &keymap::tooltip(profile, Action::AddLayer),
                        )
                        .clicked()
                        {
                            s.add_layer();
                        }
                    });
                }
            });
        });
}

fn timing_options(ui: &mut egui::Ui, s: &mut Session) {
    ui.checkbox(&mut s.animate, "Auto-key")
        .on_hover_text("Record animated property edits at the current frame.");
    ui.separator();
    let len = s.length();
    for (label, frame) in [
        ("First frame", 1),
        ("Previous frame", s.frame.saturating_sub(1).max(1)),
        ("Next frame", (s.frame + 1).min(len)),
        ("Last frame", len),
    ] {
        if ui.button(label).clicked() {
            s.frame = frame;
            ui.close();
        }
    }
    ui.separator();
    scene_timing(ui, s);
    ui.separator();
    if ui.button("Add layer").clicked() {
        s.add_layer();
        ui.close();
    }
    if ui
        .add_enabled(s.layer_count() > 1, egui::Button::new("Delete layer"))
        .clicked()
    {
        s.delete_layer(s.selected);
        ui.close();
    }
}

fn scene_timing(ui: &mut egui::Ui, s: &mut Session) {
    let mut length = s.length();
    let response = ui.add(
        egui::DragValue::new(&mut length)
            .range(1..=100_000)
            .prefix("Length "),
    );
    if response.changed() {
        s.set_length(length);
        s.frame = s.frame.min(length);
    }
    if response.drag_stopped() || response.lost_focus() {
        s.end_gesture();
    }
    let mut fps = s.doc.settings.fps;
    widgets::frame_rate(ui, "timeline-fps", &mut fps);
    s.set_frame_rate(fps);
}

/// Keyframe: amber diamond (amber = time, plus a shape, D4.5).
fn key_diamond(painter: &egui::Painter, c: Pos2, r: f32) {
    let pts = vec![
        c + Vec2::new(0.0, -r),
        c + Vec2::new(r, 0.0),
        c + Vec2::new(0.0, r),
        c + Vec2::new(-r, 0.0),
    ];
    painter.add(egui::Shape::convex_polygon(
        pts,
        theme::ACCENT_TIME,
        Stroke::new(1.0, theme::BG_PANEL),
    ));
}

/// The amber frame tag on a ruler (D3.9).
pub(crate) fn frame_tag(painter: &egui::Painter, centre: Pos2, frame: u32) {
    let text = frame.to_string();
    let w = 10.0 + 7.0 * text.len() as f32;
    let r = Rect::from_center_size(centre, Vec2::new(w, 16.0));
    painter.rect_filled(r, theme::RADIUS_CONTROL as f32, theme::ACCENT_TIME);
    painter.text(
        r.center(),
        Align2::CENTER_CENTER,
        text,
        theme::numbers(),
        theme::BG_BASE,
    );
}

/// The Timeline ruler's beats for a frame rate: [small step, half second,
/// second], in frames. 24 fps: 6, 12, 24; 30 fps: 5, 15, 30; 25 fps: 5,
/// 25 (no whole half), 25; 12 fps: 3, 6, 12.
fn ruler_steps(fps: f32) -> [u32; 3] {
    let second = (fps.round() as u32).max(1);
    let half = if second.is_multiple_of(2) {
        second / 2
    } else {
        second
    };
    let small = [4, 6, 5, 3, 2]
        .into_iter()
        .find(|d| second.is_multiple_of(*d) && second / d >= 3)
        .map_or(half, |d| second / d);
    [small.max(1), half.max(1), second]
}

/// Indent per level of the node hierarchy, in the Timeline's name column.
const INDENT: f32 = 12.0;

/// One Timeline row: a layer, a node track (index into the shown node
/// tracks) or a sound clip.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Slot {
    Layer(usize),
    Node(usize),
    Sound(usize),
}

fn kind_of(s: &Session, id: NodeId) -> Option<&NodeKind> {
    s.doc.graph.node(id).map(|n| &n.kind)
}

/// The pegs above a layer's drawing (outermost first) and the effects
/// after it, in order.
fn layer_chain(s: &Session, layer: usize) -> (Vec<NodeId>, Vec<NodeId>) {
    let graph = &s.doc.graph;
    let node = s.doc.layers[layer].node;
    let mut pegs = Vec::new();
    let mut cur = node;
    while let Some(up) = graph.input(cur, 0) {
        if matches!(
            kind_of(s, up),
            Some(
                NodeKind::Peg(_)
                    | NodeKind::MeshWarp(_)
                    | NodeKind::Skeleton(_)
                    | NodeKind::Bone(_)
            )
        ) && !pegs.contains(&up)
        {
            pegs.push(up);
            cur = up;
        } else {
            break;
        }
    }
    pegs.reverse();
    let mut effects = Vec::new();
    let mut cur = node;
    while let Some(next) = graph
        .links()
        .iter()
        .find(|l| {
            l.from == cur && l.port == 0 && matches!(kind_of(s, l.to), Some(NodeKind::Effect(_)))
        })
        .map(|l| l.to)
    {
        if effects.contains(&next) || effects.len() > 32 {
            break;
        }
        effects.push(next);
        cur = next;
    }
    (pegs, effects)
}

/// Nodes that are part of some layer's hierarchy.
fn attached_nodes(s: &Session) -> std::collections::HashSet<NodeId> {
    (0..s.layer_count())
        .flat_map(|i| {
            let (pegs, effects) = layer_chain(s, i);
            pegs.into_iter().chain(effects)
        })
        .collect()
}

/// Timeline rows in hierarchy order: for each layer, its pegs above it
/// (outermost first, indented like parents), the layer, then its effects;
/// then sound; then the remaining nodes. A peg shared by several layers
/// shows once, above the first.
fn hierarchy_rows(s: &Session, tracks: &[NodeId], sounds: usize) -> Vec<(Slot, usize)> {
    let index = |id: NodeId| tracks.iter().position(|&x| x == id);
    let mut placed = std::collections::HashSet::new();
    let mut out = Vec::new();
    for i in 0..s.layer_count() {
        let (pegs, effects) = layer_chain(s, i);
        let mut depth = 0;
        for peg in pegs {
            // A hidden peg row doesn't indent the rows under it.
            if let Some(j) = index(peg) {
                if placed.insert(peg) {
                    out.push((Slot::Node(j), depth));
                }
                depth += 1;
            }
        }
        out.push((Slot::Layer(i), depth));
        for fx in effects {
            if let Some(j) = index(fx) {
                if placed.insert(fx) {
                    out.push((Slot::Node(j), depth + 1));
                }
            }
        }
    }
    out.extend((0..sounds).map(|k| (Slot::Sound(k), 0)));
    for (j, id) in tracks.iter().enumerate() {
        if !placed.contains(id) {
            out.push((Slot::Node(j), 0));
        }
    }
    out
}

/// "Add node" from a Timeline row: the new node goes in the nearest place
/// in the hierarchy that fits (a peg above, an effect after), so the
/// Timeline can work like a layer stack or like nodes.
fn add_node_menu(ui: &mut egui::Ui, s: &mut Session, target: NodeId) {
    ui.menu_button("Add node", |ui| {
        if ui.button("Drawing layer").clicked() {
            s.add_layer();
            ui.close();
        }
        ui.separator();
        for kind in super::node::addable_nodes() {
            if ui.button(super::node::kind_label(&kind)).clicked() {
                super::node::insert_near(s, target, kind);
                ui.close();
            }
        }
    });
}

fn layer_menu(ui: &mut egui::Ui, s: &mut Session, t: &mut TimingState, row: usize) {
    if ui.button("Rename layer").clicked() {
        t.renaming = Some((row, s.layer_name(row).to_owned()));
        ui.close();
    }
    let vis = if s.layer_visible(row) {
        "Hide layer"
    } else {
        "Show layer"
    };
    if ui.button(vis).clicked() {
        s.toggle_visible(row);
        ui.close();
    }
    let lock = if s.layer_locked(row) {
        "Unlock layer"
    } else {
        "Lock layer"
    };
    if ui.button(lock).clicked() {
        s.toggle_locked(row);
        ui.close();
    }
    ui.separator();
    if ui
        .add_enabled(s.layer_count() > 1, egui::Button::new("Delete layer"))
        .clicked()
    {
        s.delete_layer(row);
        ui.close();
    }
}

fn sound_menu(ui: &mut egui::Ui, s: &mut Session, t: &mut TimingState, id: Id) {
    let Some(track) = s.doc.sounds.iter().find(|t| t.id == id) else {
        return;
    };
    let (name, on) = (track.name.clone(), !track.muted);
    if ui.button("Open sound editor").clicked() {
        t.sound_editor = Some(SoundEditor::new(track));
        ui.close();
    }
    if ui.button("Rename sound").clicked() {
        t.renaming_sound = Some((id, name));
        ui.close();
    }
    if ui
        .button(if on { "Mute sound" } else { "Unmute sound" })
        .clicked()
    {
        s.toggle_sound(id);
        ui.close();
    }
    ui.separator();
    if ui.button("Delete sound").clicked() {
        s.delete_sound(id);
        ui.close();
    }
}

/// [`rename_field`] for a sound track's name.
fn sound_rename_field(ui: &mut egui::Ui, s: &mut Session, t: &mut TimingState, rect: Rect) {
    let Some((id, text)) = t.renaming_sound.as_mut() else {
        return;
    };
    let id = *id;
    let resp = ui.put(rect, egui::TextEdit::singleline(text).font(theme::body()));
    resp.request_focus();
    let (enter, esc) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if esc {
        t.renaming_sound = None;
    } else if enter || resp.lost_focus() {
        let name = text.clone();
        t.renaming_sound = None;
        s.rename_sound(id, &name);
    }
}

/// A sound track's waveform over `block`, which starts at the track's
/// first frame. One line per pixel column, from the lowest to the highest
/// sample in that slice of time.
fn waveform(
    painter: &egui::Painter,
    block: Rect,
    peaks: &symphony_audio::Peaks,
    gain: f32,
    offset: f64,
    seconds_per_pixel: f64,
    colour: egui::Color32,
) {
    let mid = block.center().y;
    let half = block.height() / 2.0 - 1.0;
    let mut x = block.left().ceil();
    while x < block.right() {
        let from = offset + f64::from(x - block.left()) * seconds_per_pixel;
        let (lo, hi) = peaks.range(from, from + seconds_per_pixel);
        let (lo, hi) = ((lo * gain).max(-1.0), (hi * gain).min(1.0));
        if hi > lo {
            painter.line_segment(
                [
                    Pos2::new(x + 0.5, mid - hi * half),
                    Pos2::new(x + 0.5, mid - lo * half + 1.0),
                ],
                Stroke::new(1.0, colour),
            );
        }
        x += 1.0;
    }
}

/// Inline rename field over a layer name. Enter or clicking away keeps it,
/// Escape cancels (D8.6).
/// Harmony's Timeline Parameters column for a drawing layer: the drawing
/// exposed at the current frame. Drag sideways to cycle through the
/// layer's drawings, or double-click to type a drawing's name.
/// A middle drag scrolls the Timeline or Xsheet instead of acting on
/// `resp`; see [`widgets::middle_pan`].
fn middle_scrolls(ui: &egui::Ui, resp: egui::Response) -> egui::Response {
    let (resp, pan) = widgets::middle_pan(ui, resp);
    if let Some(delta) = pan {
        ui.scroll_with_delta_animation(delta, egui::style::ScrollAnimation::none());
    }
    resp
}

fn parameters_field(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    s: &mut Session,
    t: &mut TimingState,
    row: usize,
    rect: Rect,
) {
    let frame = s.frame;
    if let Some((_, text)) = t.swap_text.as_mut().filter(|(r, _)| *r == row) {
        let resp = ui.put(rect, egui::TextEdit::singleline(text).font(theme::dense()));
        resp.request_focus();
        let (enter, esc) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if esc {
            t.swap_text = None;
        } else if enter || resp.lost_focus() {
            let wanted = text.trim().to_lowercase();
            t.swap_text = None;
            let found = s
                .element_drawings(row)
                .into_iter()
                .find(|(_, n)| n.to_lowercase() == wanted);
            match found {
                Some((id, _)) => {
                    s.swap_drawing(row, frame, id);
                    s.end_gesture();
                }
                None if !wanted.is_empty() => {
                    s.notice = Some(format!("This layer has no drawing called {wanted}"));
                }
                None => {}
            }
        }
        return;
    }
    let exposed = s.doc.exposed(row, frame);
    let name = exposed
        .and_then(|d| s.doc.drawings.get(&d))
        .map(|d| d.name.clone());
    painter.rect_filled(rect, theme::RADIUS_CONTROL as f32, theme::BG_INPUT);
    widgets::fitted_text(
        ui,
        painter,
        rect.center(),
        Align2::CENTER_CENTER,
        name.as_deref().unwrap_or(""),
        theme::dense(),
        theme::TEXT_PRIMARY,
        rect.width() - 6.0,
    );
    if exposed.is_none() {
        return;
    }
    let resp = ui
        .interact(
            rect,
            ui.scope_id().with(("swap", row)),
            Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text(
            "Drawing at this frame. Drag sideways to swap it, or double-click to type a name.",
        );
    let resp = middle_scrolls(ui, resp);
    if resp.drag_started() || resp.clicked() {
        s.select_graph_node(s.doc.layers[row].node, false);
    }
    if resp.double_clicked() {
        t.swap_text = Some((row, name.unwrap_or_default()));
        return;
    }
    let list = s.element_drawings(row);
    if resp.drag_started() {
        let start = list
            .iter()
            .position(|(id, _)| Some(*id) == exposed)
            .unwrap_or(0);
        t.swap_drag = Some((row, start, 0.0));
    }
    if resp.dragged() {
        if let Some((_, start, moved)) = t.swap_drag.as_mut().filter(|(r, ..)| *r == row) {
            *moved += resp.drag_delta().x;
            let steps = (*moved / PARAM_STEP).round() as isize;
            let i = start
                .saturating_add_signed(steps)
                .min(list.len().saturating_sub(1));
            if let Some((id, _)) = list.get(i) {
                s.swap_drawing(row, frame, *id);
            }
        }
    }
    if resp.drag_stopped() {
        t.swap_drag = None;
        s.end_gesture();
    }
}

fn rename_field(ui: &mut egui::Ui, s: &mut Session, t: &mut TimingState, rect: Rect) {
    let Some((row, text)) = t.renaming.as_mut() else {
        return;
    };
    let row = *row;
    let resp = ui.put(rect, egui::TextEdit::singleline(text).font(theme::body()));
    resp.request_focus();
    let (enter, esc) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if esc {
        t.renaming = None;
    } else if enter || resp.lost_focus() {
        let name = text.clone();
        t.renaming = None;
        s.rename_layer(row, &name);
    }
}

fn reveal_track(ui: &egui::Ui, row: Rect, reveal: bool) {
    let visible = ui.clip_rect();
    if reveal && (row.top() < visible.top() || row.bottom() > visible.bottom()) {
        let x = visible.center().x;
        ui.scroll_to_rect(
            Rect::from_min_max(Pos2::new(x, row.top()), Pos2::new(x, row.bottom())),
            None,
        );
    }
}

pub fn timeline(
    ui: &mut egui::Ui,
    s: &mut Session,
    t: &mut TimingState,
    sounds: &SoundState,
    profile: Profile,
) {
    header(ui, s, profile);
    const HEAD: f32 = 176.0;
    const RULER: f32 = 28.0;
    // The lock switch, just left of the Parameters field.
    const LOCK: std::ops::Range<f32> = HEAD - PARAM_W - 28.0..HEAD - PARAM_W - 6.0;
    // Ctrl+wheel zooms the frames, from a whole scene at a glance to
    // wide cells.
    const FRAME_W: std::ops::RangeInclusive<f32> = 2.0..=48.0;
    let cw = t.frame_w.unwrap_or(theme::FRAME_CELL);
    let rh = theme::ROW;
    // Sound tracks come after the drawing layers, as in Harmony.
    let layers = s.layer_count();
    let rows = layers;
    let len = s.length();
    let reveal = s.reveal_node_track.take();
    let visible_node = super::node::selected_node(s);
    let mut node_tracks = Vec::new();
    let mut has_unanimated_nodes = false;
    for (id, node) in s.doc.graph.nodes() {
        use symphony_core::graph::NodeKind;
        if matches!(node.kind, NodeKind::Drawing { .. }) {
            continue;
        }
        let mut frames = std::collections::BTreeSet::new();
        frames.extend(
            s.doc
                .graph
                .keys(id)
                .into_iter()
                .flat_map(|keys| keys.keys().copied()),
        );
        match &node.kind {
            NodeKind::Peg(peg) | NodeKind::Skeleton(peg) => {
                for value in [&peg.x, &peg.y, &peg.z, &peg.rotation]
                    .into_iter()
                    .chain(peg.scale_x.iter())
                    .chain(peg.scale_y.iter())
                {
                    frames.extend(value.keys().iter().map(|key| key.frame));
                }
            }
            NodeKind::Bone(bone) => {
                for value in bone.values() {
                    frames.extend(value.keys().iter().map(|key| key.frame));
                }
            }
            NodeKind::MeshWarp(warp) => {
                for value in [&warp.x, &warp.y, &warp.width, &warp.height]
                    .into_iter()
                    .chain(warp.points.iter().flatten())
                {
                    frames.extend(value.keys().iter().map(|key| key.frame));
                }
            }
            NodeKind::Display => {
                let motion = &s.doc.camera_motion;
                for value in [&s.doc.camera_fov, &motion.x, &motion.y, &motion.dolly] {
                    frames.extend(value.keys().iter().map(|key| key.frame));
                }
            }
            _ => {}
        }
        has_unanimated_nodes |= frames.is_empty();
        node_tracks.push((id, super::node::node_name(node), frames));
    }
    ui.horizontal(|ui| {
        let mut show_pegs = !t.hide_pegs;
        ui.toggle_value(&mut show_pegs, "Show pegs")
            .on_hover_text("Show each layer's pegs above it.");
        t.hide_pegs = !show_pegs;
        if has_unanimated_nodes {
            ui.toggle_value(&mut t.show_unanimated_nodes, "Show all node tracks")
                .on_hover_text("Animated nodes and the selected node are always shown.");
        }
    });
    // Pegs and effects that belong to a layer always show: they're part
    // of the layer's hierarchy. The filter is for loose nodes.
    let attached = attached_nodes(s);
    node_tracks.retain(|(id, _, frames)| {
        if t.hide_pegs && matches!(kind_of(s, *id), Some(NodeKind::Peg(_))) {
            return false;
        }
        t.show_unanimated_nodes
            || !frames.is_empty()
            || Some(*id) == visible_node
            || attached.contains(id)
    });
    // Own labels because timeline interactions can mutate the document.
    let node_tracks: Vec<_> = node_tracks
        .into_iter()
        .map(|(id, name, frames)| (id, name.to_owned(), frames))
        .collect();
    let sound_rows = s.doc.sounds.len();
    let track_count = rows + node_tracks.len() + sound_rows;
    // Where each row goes, following the node hierarchy.
    let track_ids: Vec<NodeId> = node_tracks.iter().map(|(id, _, _)| *id).collect();
    let layout = hierarchy_rows(s, &track_ids, sound_rows);
    let mut layer_slot = vec![0usize; rows];
    let mut layer_depth = vec![0usize; rows];
    let mut node_slot = vec![0usize; node_tracks.len()];
    let mut node_depth = vec![0usize; node_tracks.len()];
    let mut sound_slot = vec![0usize; sound_rows];
    for (slot, &(row, depth)) in layout.iter().enumerate() {
        match row {
            Slot::Layer(i) => {
                layer_slot[i] = slot;
                layer_depth[i] = depth;
            }
            Slot::Node(j) => {
                node_slot[j] = slot;
                node_depth[j] = depth;
            }
            Slot::Sound(k) => sound_slot[k] = slot,
        }
    }
    let width = HEAD + len as f32 * cw + 16.0;

    let out = egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let size = Vec2::new(
                width.max(ui.available_width()),
                (RULER + track_count as f32 * rh + 8.0).max(ui.available_height()),
            );
            let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
            // The middle button scrolls the view and never edits it.
            let resp = middle_scrolls(ui, resp);
            let o = resp.rect.min;
            let frame_x = |f: u32| o.x + HEAD + (f - 1) as f32 * cw;
            let rows_bottom = o.y + RULER + track_count as f32 * rh;
            let zoom = ui.input(|i| i.zoom_delta());
            if let Some(p) = ui.ctx().pointer_hover_pos().filter(|p| {
                zoom != 1.0 && ui.clip_rect().contains(*p) && ui.rect_contains_pointer(resp.rect)
            }) {
                let wide = (cw * zoom).clamp(*FRAME_W.start(), *FRAME_W.end());
                // The frame under the pointer stays under it.
                let along = (p.x - o.x - HEAD).max(0.0);
                ui.scroll_with_delta_animation(
                    Vec2::new(-along * (wide / cw - 1.0), 0.0),
                    egui::style::ScrollAnimation::none(),
                );
                t.frame_w = Some(wide);
                ui.ctx().request_repaint();
            }
            // The ruler stays at the top of the view while the rows
            // scroll under it.
            let ruler_y = ui.clip_rect().top().max(o.y);

            // Beats follow the frame rate: the second, its half and a
            // small step (24 fps: 24, 12, 6; 30 fps: 30, 15, 5). Lines and
            // numbers sit on the frame's centre, like the playhead.
            let [small, half, second] = ruler_steps(s.doc.settings.fps);
            // Number every small step if there's room, else every half.
            let label_step = if small as f32 * cw >= 22.0 {
                small
            } else if half as f32 * cw >= 22.0 {
                half
            } else {
                second
            };
            let beat = |f: u32| {
                if f.is_multiple_of(second) {
                    3
                } else if f.is_multiple_of(half) {
                    2
                } else if f.is_multiple_of(small) {
                    1
                } else {
                    0
                }
            };
            for f in (1..=len).filter(|&f| beat(f) > 0) {
                let x = frame_x(f) + cw / 2.0;
                let alpha = [0.0, 0.35, 0.6, 1.0][beat(f)];
                painter.line_segment(
                    [Pos2::new(x, o.y + RULER), Pos2::new(x, rows_bottom)],
                    Stroke::new(1.0, theme::LINE_STRONG.gamma_multiply(alpha)),
                );
            }

            for i in 0..rows {
                let y = o.y + RULER + layer_slot[i] as f32 * rh;
                let indent = layer_depth[i] as f32 * INDENT;
                let node = s.doc.layers[i].node;
                let sel = visible_node == Some(node);
                let visible = s.layer_visible(i);
                let row = Rect::from_min_size(
                    Pos2::new(o.x + 4.0, y + 1.0),
                    Vec2::new(width - 8.0, rh - 2.0),
                );
                reveal_track(ui, row, reveal == Some(node));
                if sel {
                    painter.rect_filled(row, theme::RADIUS_CONTROL as f32, theme::BG_RAISED);
                    let edge = Rect::from_min_size(row.min, Vec2::new(3.0, row.height()));
                    painter.rect_filled(
                        edge,
                        CornerRadius {
                            nw: 2,
                            sw: 2,
                            ne: 0,
                            se: 0,
                        },
                        theme::ACCENT_SELECT,
                    );
                }
                let eye = if visible { Icon::Eye } else { Icon::EyeOff };
                let eye_rect = Rect::from_center_size(
                    Pos2::new(o.x + 22.0 + indent, y + rh / 2.0),
                    Vec2::splat(16.0),
                );
                icons::paint(
                    &painter,
                    eye_rect,
                    eye,
                    if visible {
                        theme::TEXT_SECONDARY
                    } else {
                        theme::TEXT_DISABLED
                    },
                );
                // Locked: a closed padlock and the name in red, as in Harmony.
                let locked = s.layer_locked(i);
                let lock_rect = Rect::from_center_size(
                    Pos2::new(o.x + (LOCK.start + LOCK.end) / 2.0, y + rh / 2.0),
                    Vec2::splat(16.0),
                );
                icons::paint(
                    &painter,
                    lock_rect,
                    if locked { Icon::Lock } else { Icon::Unlock },
                    if locked {
                        theme::TEXT_SECONDARY
                    } else {
                        theme::TEXT_DISABLED
                    },
                );
                if resp
                    .hover_pos()
                    .is_some_and(|p| LOCK.contains(&(p.x - o.x)) && (y..y + rh).contains(&p.y))
                {
                    resp.clone()
                        .on_hover_text_at_pointer(keymap::tooltip(profile, Action::ToggleLock));
                }
                if !matches!(t.renaming, Some((r, _)) if r == i) {
                    let name_col = if locked {
                        theme::STATE_LOCKED
                    } else if visible {
                        theme::TEXT_PRIMARY
                    } else {
                        theme::TEXT_SECONDARY
                    };
                    widgets::fitted_text(
                        ui,
                        &painter,
                        Pos2::new(o.x + 40.0 + indent, y + rh / 2.0),
                        Align2::LEFT_CENTER,
                        s.layer_name(i),
                        theme::body(),
                        name_col,
                        HEAD - 68.0 - PARAM_W - indent,
                    );
                }
                parameters_field(
                    ui,
                    &painter,
                    s,
                    t,
                    i,
                    Rect::from_min_size(
                        Pos2::new(o.x + HEAD - PARAM_W - 6.0, y + 4.0),
                        Vec2::new(PARAM_W, rh - 8.0),
                    ),
                );

                if let Some(col) = s.doc.column(i) {
                    let mut f = 1;
                    while f <= len {
                        let Some(d) = col.get(f) else {
                            f += 1;
                            continue;
                        };
                        let start = f;
                        while f <= len && col.get(f) == Some(d) {
                            f += 1;
                        }
                        let block = Rect::from_min_max(
                            Pos2::new(frame_x(start) + 1.0, y + 5.0),
                            Pos2::new(frame_x(f) - 1.0, y + rh - 5.0),
                        );
                        let fill = if sel {
                            theme::LINE_STRONG
                        } else {
                            theme::BG_HOVER
                        };
                        painter.rect_filled(block, theme::RADIUS_CONTROL as f32, fill);
                        let name = s
                            .doc
                            .drawings
                            .get(&d)
                            .map(|d| d.name.as_str())
                            .unwrap_or("?");
                        // The drawing name must stay readable: step it past the
                        // playhead and any keyframe diamond it would sit under.
                        let text_w = 7.0 * name.len() as f32;
                        let mut obstacles: Vec<f32> = (start..f)
                            .filter(|fr| s.doc.graph.has_key(node, *fr) || *fr == s.frame)
                            .map(|fr| frame_x(fr) + cw / 2.0)
                            .collect();
                        obstacles.sort_by(f32::total_cmp);
                        let mut x = block.left() + 5.0;
                        for ob in obstacles {
                            if ob + 6.0 > x - 2.0 && ob - 6.0 < x + text_w {
                                x = ob + 7.0;
                            }
                        }
                        if x + text_w < block.right() - 2.0 {
                            painter.text(
                                Pos2::new(x, block.center().y),
                                Align2::LEFT_CENTER,
                                name,
                                theme::dense(),
                                theme::TEXT_PRIMARY,
                            );
                        }
                    }
                }
                for f in s
                    .doc
                    .graph
                    .keys(node)
                    .into_iter()
                    .flat_map(|keys| keys.keys().copied())
                    .filter(|f| (1..=len).contains(f))
                {
                    key_diamond(
                        &painter,
                        Pos2::new(frame_x(f) + cw / 2.0, y + rh / 2.0),
                        4.5,
                    );
                }
            }

            for (index, (node, name, frames)) in node_tracks.iter().enumerate() {
                let y = o.y + RULER + node_slot[index] as f32 * rh;
                let indent = node_depth[index] as f32 * INDENT;
                let row = Rect::from_min_size(
                    Pos2::new(o.x + 4.0, y + 1.0),
                    Vec2::new(width - 8.0, rh - 2.0),
                );
                reveal_track(ui, row, reveal == Some(*node));
                if visible_node == Some(*node) {
                    painter.rect_filled(row, theme::RADIUS_CONTROL, theme::BG_RAISED);
                }
                widgets::fitted_text(
                    ui,
                    &painter,
                    Pos2::new(o.x + 18.0 + indent, y + rh / 2.0),
                    Align2::LEFT_CENTER,
                    name,
                    theme::body(),
                    theme::TEXT_SECONDARY,
                    HEAD - 30.0 - indent,
                );
                for f in frames.iter().copied().filter(|f| (1..=len).contains(f)) {
                    key_diamond(
                        &painter,
                        Pos2::new(frame_x(f) + cw / 2.0, y + rh / 2.0),
                        4.5,
                    );
                }
            }

            let fps = s.doc.settings.fps.max(1.0);
            // Each sound block, for moving and trimming.
            let mut blocks: Vec<(Id, Rect)> = Vec::new();
            for (j, track) in s.doc.sounds.iter().enumerate() {
                let y = o.y + RULER + sound_slot[j] as f32 * rh;
                let on = !track.muted;
                let eye_rect =
                    Rect::from_center_size(Pos2::new(o.x + 22.0, y + rh / 2.0), Vec2::splat(16.0));
                // Mute uses the layer's show/hide icon until the icon set
                // has a speaker.
                icons::paint(
                    &painter,
                    eye_rect,
                    if on { Icon::Eye } else { Icon::EyeOff },
                    if on {
                        theme::TEXT_SECONDARY
                    } else {
                        theme::TEXT_DISABLED
                    },
                );
                if !matches!(&t.renaming_sound, Some((id, _)) if *id == track.id) {
                    widgets::fitted_text(
                        ui,
                        &painter,
                        Pos2::new(o.x + 40.0, y + rh / 2.0),
                        Align2::LEFT_CENTER,
                        &track.name,
                        theme::body(),
                        if on {
                            theme::TEXT_PRIMARY
                        } else {
                            theme::TEXT_SECONDARY
                        },
                        HEAD - 56.0,
                    );
                }
                let Some(file) = s.doc.media.get(&track.media) else {
                    continue;
                };
                let first = track.start.max(1);
                if first > len {
                    continue;
                }
                // A sound longer than the scene stops at the scene end.
                let last = sound::last_frame(track, file, fps).min(len);
                let block = Rect::from_min_max(
                    Pos2::new(frame_x(first) + 1.0, y + 5.0),
                    Pos2::new(frame_x(last + 1) - 1.0, y + rh - 5.0),
                );
                painter.rect_filled(block, theme::RADIUS_CONTROL as f32, theme::BG_HOVER);
                blocks.push((track.id, block));
                if let Some((peaks, gain)) = sounds.peaks(track.media) {
                    let colour = if on {
                        theme::TEXT_SECONDARY
                    } else {
                        theme::TEXT_DISABLED
                    };
                    let seconds_per_pixel = 1.0 / (f64::from(fps) * f64::from(cw));
                    // Inset so the waveform stays inside the rounded block.
                    waveform(
                        &painter,
                        block.shrink2(Vec2::new(1.0, 2.0)),
                        peaks,
                        gain,
                        track.offset(file),
                        seconds_per_pixel,
                        colour,
                    );
                }
            }

            // Drag a sound block's middle to move it, an edge to trim it.
            // With Shift, a drag anywhere trims the nearer edge.
            let shift = ui.input(|i| i.modifiers.shift);
            let hit = |p: Pos2| {
                blocks.iter().find_map(|(id, r)| {
                    r.expand2(Vec2::new(SOUND_EDGE / 2.0, 0.0))
                        .contains(p)
                        .then(|| {
                            let part = if p.x - r.left() <= SOUND_EDGE {
                                SoundPart::Start
                            } else if r.right() - p.x <= SOUND_EDGE {
                                SoundPart::End
                            } else if shift && p.x < r.center().x {
                                SoundPart::Start
                            } else if shift {
                                SoundPart::End
                            } else {
                                SoundPart::Body
                            };
                            (*id, part)
                        })
                })
            };
            // Where the press began, not where the pointer is when egui
            // calls it a drag (a few pixels on, often past an edge's zone).
            let pressed_on = ui
                .input(|i| i.pointer.press_origin())
                .filter(|_| resp.is_pointer_button_down_on() || resp.drag_stopped());
            let on_block = pressed_on
                .filter(|p| p.y >= ruler_y + RULER)
                .and_then(hit)
                .is_some();
            if resp.drag_started() {
                if let Some((p, (id, part))) = pressed_on
                    .filter(|p| p.y >= ruler_y + RULER)
                    .and_then(|p| hit(p).map(|h| (p, h)))
                {
                    if let Some(original) = s.doc.sounds.iter().find(|t| t.id == id).cloned() {
                        t.sound_drag = Some(SoundDrag {
                            part,
                            x: p.x,
                            original,
                        });
                    }
                }
            }
            let cursor = match &t.sound_drag {
                Some(d) => Some(d.part),
                None => resp.hover_pos().and_then(hit).map(|(_, part)| part),
            };
            match cursor {
                Some(SoundPart::Body) if t.sound_drag.is_some() => {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing)
                }
                Some(SoundPart::Body) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
                Some(_) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
                None => {}
            }
            let dragging_sound = t.sound_drag.is_some();
            if let Some(d) = &t.sound_drag {
                if let (Some(p), Some(file)) = (
                    resp.interact_pointer_pos(),
                    s.doc.media.get(&d.original.media),
                ) {
                    let frames = ((p.x - d.x) / cw).round() as i64;
                    let o = &d.original;
                    let (after, name) = match d.part {
                        SoundPart::Body => (
                            // Never past the scene end, where the block would go away.
                            o.moved_to(
                                (i64::from(o.start) + frames).clamp(1, i64::from(len)) as u32
                            ),
                            "Move sound",
                        ),
                        SoundPart::Start => (o.trimmed_start(file, fps, frames), "Trim sound"),
                        SoundPart::End => (o.trimmed_end(file, fps, frames), "Trim sound"),
                    };
                    s.drag_sound(after, name);
                }
                if resp.drag_stopped() {
                    s.end_gesture();
                    t.sound_drag = None;
                }
            }

            // A layer that was just added or moved glows briefly, so the
            // eye finds it instead of rows simply jumping.
            let now = ui.input(|i| i.time);
            let order: Vec<symphony_core::id::Id> = s.doc.layers.iter().map(|l| l.column).collect();
            if !t.last_order.is_empty() && order != t.last_order {
                for (i, id) in order.iter().enumerate() {
                    if t.last_order.get(i) != Some(id) && !t.last_order.is_empty() {
                        let moved_or_new = t.last_order.iter().position(|c| c == id) != Some(i);
                        if moved_or_new && s.selected == i {
                            t.arrived.retain(|(c, _)| c != id);
                            t.arrived.push((*id, now));
                        }
                    }
                }
            }
            t.last_order = order.clone();
            t.arrived.retain(|(_, at)| now - at < 0.8);
            for (id, at) in &t.arrived {
                if let Some(row) = order.iter().position(|c| c == id) {
                    let k = 1.0 - ((now - at) / 0.8) as f32;
                    let y = o.y + RULER + layer_slot[row] as f32 * rh;
                    painter.rect_filled(
                        Rect::from_min_size(
                            Pos2::new(o.x + 4.0, y + 1.0),
                            Vec2::new(width - 8.0, rh - 2.0),
                        ),
                        theme::RADIUS_CONTROL,
                        theme::ACCENT_SELECT.gamma_multiply(0.22 * k * k),
                    );
                    ui.ctx().request_repaint();
                }
            }
            // The ruler, over the rows that have scrolled under it.
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(ui.clip_rect().left(), ruler_y),
                    Vec2::new(ui.clip_rect().width(), RULER),
                ),
                0.0,
                theme::BG_PANEL,
            );
            for f in 1..=len {
                let x = frame_x(f) + cw / 2.0;
                let level = beat(f);
                // Frames too narrow to tell apart keep only the beats.
                if level == 0 && cw < 5.0 {
                    continue;
                }
                let top = match level {
                    0 => 22.0,
                    1 => 18.0,
                    _ => 15.0,
                };
                painter.line_segment(
                    [
                        Pos2::new(x, ruler_y + top),
                        Pos2::new(x, ruler_y + RULER - 2.0),
                    ],
                    Stroke::new(1.0, theme::LINE_STRONG),
                );
                if f == 1 || f % label_step == 0 {
                    painter.text(
                        Pos2::new(x, ruler_y + 4.0),
                        Align2::CENTER_TOP,
                        f.to_string(),
                        theme::numbers(),
                        if level == 3 {
                            theme::TEXT_PRIMARY
                        } else {
                            theme::TEXT_SECONDARY
                        },
                    );
                }
            }
            // The playback range, as a band along the ruler.
            if let Some((a, b)) = s.play_range {
                let (a, b) = (a.clamp(1, len), b.clamp(1, len));
                let band = Rect::from_min_max(
                    Pos2::new(frame_x(a.min(b)), ruler_y + 2.0),
                    Pos2::new(frame_x(a.max(b)) + cw, ruler_y + RULER - 2.0),
                );
                painter.rect_filled(
                    band,
                    theme::RADIUS_CONTROL,
                    theme::ACCENT_TIME.gamma_multiply(0.16),
                );
            }
            // Determine the gesture from the press, even when its current
            // pointer has crossed into the headers or past the last frame.
            // A drag that starts on a sound block moves or trims the sound,
            // so it never scrubs.
            if resp.drag_started_by(egui::PointerButton::Primary) {
                t.scrubbing = !on_block
                    && ui
                        .input(|i| i.pointer.press_origin())
                        .is_some_and(|p| p.x >= o.x + HEAD && p.y >= o.y);
            }
            let scrub = t.scrubbing && (resp.dragged() || resp.drag_stopped());
            if let Some(p) = resp.interact_pointer_pos().filter(|p| {
                scrub || (resp.clicked_by(egui::PointerButton::Primary) && p.x >= o.x + HEAD)
            }) {
                let frame =
                    (((p.x - o.x - HEAD) / cw).floor() as i64 + 1).clamp(1, i64::from(len)) as u32;
                s.frame = frame;
            }
            if resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
                t.scrubbing = false;
            }
            // A click jumps straight to the frame (only that frame is ever
            // shown or rendered); just the line glides there (Select
            // motion). Dragging and playing keep it exactly on the frame:
            // scrubbing must feel snappy, not trail the mouse.
            let target = s.frame.min(len) as f32;
            let playhead = egui::Id::unique("timeline-playhead");
            let shown = if s.playing || scrub || s.frame == 1 {
                symphony_ui::motion::start(ui.ctx(), playhead, target);
                target
            } else {
                symphony_ui::motion::value(
                    ui.ctx(),
                    playhead,
                    target,
                    symphony_ui::motion::Motion::Select,
                )
            };
            let whole = (shown.floor() as u32).clamp(1, len);
            let cx = frame_x(whole)
                + (frame_x((whole + 1).min(len)) - frame_x(whole)) * shown.fract()
                + cw / 2.0;
            painter.line_segment(
                [
                    Pos2::new(cx, ruler_y + RULER - 4.0),
                    Pos2::new(cx, rows_bottom),
                ],
                Stroke::new(1.5, theme::ACCENT_TIME),
            );
            frame_tag(&painter, Pos2::new(cx, ruler_y + RULER - 12.0), s.frame);

            let slot_at = |p: Pos2| -> Option<Slot> {
                let r = ((p.y - o.y - RULER) / rh).floor();
                // Not a row where the ruler covers it.
                (r >= 0.0 && p.y >= ruler_y + RULER)
                    .then(|| layout.get(r as usize).map(|&(row, _)| row))
                    .flatten()
            };
            let row_at = |p: Pos2| -> Option<usize> {
                match slot_at(p) {
                    Some(Slot::Layer(i)) => Some(i),
                    _ => None,
                }
            };
            // Drag a layer by its name to move it in the stack; a line
            // shows where it will land.
            if resp.drag_started_by(egui::PointerButton::Primary) && !t.scrubbing {
                let origin = ui.input(|i| i.pointer.press_origin());
                if let (Some(p), Some(row)) = (origin, origin.and_then(row_at)) {
                    let x = p.x - o.x;
                    // Layer rows only: sound rows sit below the layers.
                    if row < layers && (32.0..LOCK.start).contains(&x) {
                        t.layer_drag = Some(row);
                    }
                }
            }
            if let (Some(from), Some(p)) = (t.layer_drag, resp.interact_pointer_pos()) {
                // The gap in the layer stack: layers whose row centre is
                // above the pointer come before it.
                let at = (p.y - o.y - RULER) / rh;
                let gap = (0..rows)
                    .filter(|&i| (layer_slot[i] as f32 + 0.5) < at)
                    .count();
                let line_slot = if gap < rows {
                    layer_slot[gap]
                } else {
                    layer_slot.iter().max().map_or(0, |m| m + 1)
                };
                let y = o.y + RULER + line_slot as f32 * rh;
                painter.line_segment(
                    [Pos2::new(o.x + 8.0, y), Pos2::new(o.x + width - 8.0, y)],
                    Stroke::new(2.0, theme::ACCENT_SELECT),
                );
                if resp.drag_stopped() {
                    t.layer_drag = None;
                    let to = if gap > from { gap - 1 } else { gap };
                    s.move_layer(from, to.min(layers.saturating_sub(1)));
                }
            }
            if !resp.dragged() && !resp.drag_stopped() {
                t.layer_drag = None;
            }
            // A press on a sound block picks nothing until it turns out to
            // be a click.
            let picks = !dragging_sound && (!on_block || resp.clicked()) && t.layer_drag.is_none();
            if let Some(p) = resp.interact_pointer_pos().filter(|_| picks) {
                let rel = p - o;
                if let Some(Slot::Sound(j)) = slot_at(p) {
                    let id = s.doc.sounds[j].id;
                    if rel.x < 32.0 && resp.clicked() {
                        s.toggle_sound(id);
                    }
                    // As in Harmony, a double-click opens the Sound editor,
                    // from the name or from the sound itself.
                    if resp.double_clicked() && rel.x >= 32.0 {
                        t.sound_editor = Some(SoundEditor::new(&s.doc.sounds[j]));
                    }
                } else if let Some(row) = row_at(p) {
                    if resp.clicked() || resp.drag_started() {
                        if rel.x < 32.0 && resp.clicked() {
                            s.toggle_visible(row);
                        } else if LOCK.contains(&rel.x) && resp.clicked() {
                            s.toggle_locked(row);
                        }
                        s.select_graph_node(s.doc.layers[row].node, false);
                    }
                    if resp.double_clicked() && rel.x < LOCK.start {
                        t.renaming = Some((row, s.layer_name(row).to_owned()));
                    }
                } else if resp.clicked() || resp.drag_started() {
                    if let Some(Slot::Node(j)) = slot_at(p) {
                        s.select_graph_node(node_tracks[j].0, false);
                    }
                }
            }
            // Right-click a layer or node: its menu, with Add node, so the
            // Timeline works as a layer stack or as nodes.
            if resp.secondary_clicked() {
                t.menu_row = None;
                t.menu_node = None;
                t.sound_menu = None;
                match resp.interact_pointer_pos().and_then(slot_at) {
                    Some(Slot::Layer(r)) => {
                        t.menu_row = Some(r);
                        t.menu_node = Some(s.doc.layers[r].node);
                    }
                    Some(Slot::Node(j)) => t.menu_node = Some(node_tracks[j].0),
                    Some(Slot::Sound(j)) => t.sound_menu = Some(s.doc.sounds[j].id),
                    None => {}
                }
                if let Some(node) = t.menu_node {
                    s.select_graph_node(node, false);
                }
            }
            if let Some(id) = t.sound_menu {
                resp.context_menu(|ui| sound_menu(ui, s, t, id));
            } else if t.menu_node.is_some() {
                resp.context_menu(|ui| {
                    if let Some(node) = t.menu_node {
                        add_node_menu(ui, s, node);
                    }
                    if let Some(row) = t.menu_row {
                        ui.separator();
                        layer_menu(ui, s, t, row);
                    } else if let Some(node) = t.menu_node {
                        // A peg, Mesh Warp or effect row.
                        ui.separator();
                        let why = super::node::undeletable(s, node);
                        let b = ui.add_enabled(why.is_none(), egui::Button::new("Delete node"));
                        let b = match why {
                            Some(w) => b.on_disabled_hover_text(w),
                            None => b,
                        };
                        if b.clicked() {
                            super::node::delete_nodes(s, &[node]);
                            ui.close();
                        }
                    }
                });
            }

            let renaming = t
                .renaming_sound
                .as_ref()
                .and_then(|(id, _)| s.doc.sounds.iter().position(|t| t.id == *id));
            if let Some(j) = renaming {
                let y = o.y + RULER + sound_slot[j] as f32 * rh;
                sound_rename_field(
                    ui,
                    s,
                    t,
                    Rect::from_min_size(
                        Pos2::new(o.x + 36.0, y + 3.0),
                        Vec2::new(HEAD - 52.0, rh - 6.0),
                    ),
                );
            }
            if let Some((row, _)) = t.renaming {
                let y = o.y + RULER + layer_slot.get(row).copied().unwrap_or(row) as f32 * rh;
                rename_field(
                    ui,
                    s,
                    t,
                    Rect::from_min_size(
                        Pos2::new(o.x + 36.0, y + 3.0),
                        Vec2::new(HEAD - 52.0 - PARAM_W, rh - 6.0),
                    ),
                );
            }
        });
    // More tracks below (or above): a soft fade at that edge says so.
    let view = out.inner_rect;
    let (offset, content) = (out.state.clamped_offset().y, out.content_size.y);
    let fade = 24.0_f32.min(view.height() * 0.25);
    if offset + view.height() < content - 1.0 {
        theme::fade_strip(
            ui.painter(),
            Rect::from_min_max(Pos2::new(view.left(), view.bottom() - fade), view.max),
            theme::GLASS_PANEL,
            false,
        );
    }
}

pub fn xsheet(ui: &mut egui::Ui, s: &mut Session, t: &mut TimingState, profile: Profile) {
    header(ui, s, profile);
    const NUM: f32 = 48.0;
    const CW: f32 = 72.0;
    const HEAD: f32 = 28.0;
    let rh = theme::XSHEET_ROW;
    let cols = s.layer_count();
    let len = s.length();
    let width = NUM + cols as f32 * CW + 8.0;

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let size = Vec2::new(
                width.max(ui.available_width()),
                HEAD + len as f32 * rh + 8.0,
            );
            let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
            // The middle button scrolls the view and never edits it.
            let resp = middle_scrolls(ui, resp);
            let o = resp.rect.min;
            let row_y = |f: u32| o.y + HEAD + (f - 1) as f32 * rh;

            for f in 1..=len {
                let y = row_y(f);
                if f % 2 == 0 {
                    painter.rect_filled(
                        Rect::from_min_size(
                            Pos2::new(o.x + NUM, y),
                            Vec2::new(cols as f32 * CW, rh),
                        ),
                        0.0,
                        theme::BG_RAISED,
                    );
                }
                if f % 8 == 0 {
                    // Paper exposure sheets mark every 8 frames; we keep that rhythm.
                    painter.line_segment(
                        [
                            Pos2::new(o.x + NUM, y + rh),
                            Pos2::new(o.x + NUM + cols as f32 * CW, y + rh),
                        ],
                        Stroke::new(1.0, theme::LINE_STRONG),
                    );
                }
                if f != s.frame {
                    painter.text(
                        Pos2::new(o.x + NUM - 10.0, y + rh / 2.0),
                        Align2::RIGHT_CENTER,
                        f.to_string(),
                        theme::numbers(),
                        theme::TEXT_SECONDARY,
                    );
                }
            }

            for i in 0..cols {
                let x = o.x + NUM + i as f32 * CW;
                let sel = i == s.selected;
                let head = Rect::from_min_size(
                    Pos2::new(x + 2.0, o.y + 2.0),
                    Vec2::new(CW - 4.0, HEAD - 6.0),
                );
                painter.rect_filled(
                    head,
                    theme::RADIUS_CONTROL as f32,
                    if sel {
                        theme::ACCENT_SELECT_FILL
                    } else {
                        theme::BG_RAISED
                    },
                );
                if !matches!(t.renaming, Some((r, _)) if r == i) {
                    let text = if sel {
                        theme::ON_ACCENT
                    } else if s.layer_visible(i) {
                        theme::TEXT_PRIMARY
                    } else {
                        theme::TEXT_SECONDARY
                    };
                    widgets::fitted_text(
                        ui,
                        &painter,
                        head.center(),
                        Align2::CENTER_CENTER,
                        s.layer_name(i),
                        theme::body(),
                        text,
                        head.width() - 8.0,
                    );
                }
                painter.line_segment(
                    [Pos2::new(x, o.y + HEAD), Pos2::new(x, o.y + size.y)],
                    Stroke::new(1.0, theme::LINE_SUBTLE),
                );

                let Some(col) = s.doc.column(i) else { continue };
                for f in 1..=len {
                    let Some(d) = col.get(f) else { continue };
                    let y = row_y(f);
                    if f > 1 && col.get(f - 1) == Some(d) {
                        // Held cell: a vertical line, like a paper exposure sheet.
                        let cx = x + CW / 2.0;
                        painter.line_segment(
                            [Pos2::new(cx, y), Pos2::new(cx, y + rh)],
                            Stroke::new(1.5, theme::TEXT_SECONDARY),
                        );
                    } else {
                        let name = s
                            .doc
                            .drawings
                            .get(&d)
                            .map(|d| d.name.as_str())
                            .unwrap_or("?");
                        painter.text(
                            Pos2::new(x + CW / 2.0, y + rh / 2.0),
                            Align2::CENTER_CENTER,
                            name,
                            theme::numbers(),
                            theme::TEXT_PRIMARY,
                        );
                    }
                }
            }

            let y = row_y(s.frame.min(len)) + rh / 2.0;
            painter.line_segment(
                [
                    Pos2::new(o.x + NUM, y),
                    Pos2::new(o.x + NUM + cols as f32 * CW, y),
                ],
                Stroke::new(1.0, theme::ACCENT_TIME.gamma_multiply(0.6)),
            );
            frame_tag(&painter, Pos2::new(o.x + NUM / 2.0 + 2.0, y), s.frame);

            let col_at = |p: Pos2| -> Option<usize> {
                let c = ((p.x - o.x - NUM) / CW).floor();
                (c >= 0.0 && (c as usize) < cols).then_some(c as usize)
            };
            if let Some(p) = resp.interact_pointer_pos() {
                let rel = p - o;
                if rel.y >= HEAD {
                    s.frame = (((rel.y - HEAD) / rh) as u32 + 1).clamp(1, len);
                }
                if let Some(c) = col_at(p) {
                    if resp.clicked() || resp.drag_started() {
                        s.select_graph_node(s.doc.layers[c].node, false);
                    }
                    if resp.double_clicked() && rel.y < HEAD {
                        t.renaming = Some((c, s.layer_name(c).to_owned()));
                    }
                }
            }
            if resp.secondary_clicked() {
                t.menu_row = resp.interact_pointer_pos().and_then(col_at);
                if let Some(c) = t.menu_row {
                    s.select_graph_node(s.doc.layers[c].node, false);
                }
            }
            if let Some(c) = t.menu_row {
                resp.context_menu(|ui| layer_menu(ui, s, t, c));
            }
            if let Some((c, _)) = t.renaming {
                let x = o.x + NUM + c as f32 * CW;
                rename_field(
                    ui,
                    s,
                    t,
                    Rect::from_min_size(
                        Pos2::new(x + 2.0, o.y + 2.0),
                        Vec2::new(CW - 4.0, HEAD - 6.0),
                    ),
                );
            }
        });
}

#[cfg(test)]
mod hierarchy_tests {
    use super::*;

    #[test]
    fn timeline_scrub_can_cross_into_headers_and_reach_frame_one() {
        let mut session = Session::new_scene();
        session.frame = 6;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(Vec2::new(800.0, 380.0))
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, (session, timing)| {
                    timeline(
                        ui,
                        session,
                        timing,
                        &SoundState::default(),
                        Profile::Symphony,
                    )
                },
                (session, TimingState::default()),
            );
        harness.run_steps(3);
        let press = Pos2::new(320.0, 96.0);
        let moved = Pos2::new(144.0, 96.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        harness
            .input_mut()
            .events
            .extend([egui::Event::PointerMoved(press), button(press, true)]);
        harness.run_steps(1);
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(moved));
        harness.run_steps(1);
        assert_eq!(harness.state().0.frame, 1);
        harness.input_mut().events.push(button(moved, false));
        harness.run_steps(1);
        assert_eq!(harness.state().0.frame, 1);
        assert!(!harness.state().1.scrubbing);
    }

    #[test]
    fn the_ruler_stays_on_top_when_the_rows_scroll() {
        let mut session = Session::new_scene();
        for _ in 0..30 {
            session.add_layer();
        }
        let mut harness = egui_kittest::Harness::builder()
            .with_size(Vec2::new(800.0, 380.0))
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, (session, timing)| {
                    timeline(
                        ui,
                        session,
                        timing,
                        &SoundState::default(),
                        Profile::Symphony,
                    )
                },
                (session, TimingState::default()),
            );
        harness.run_steps(3);
        let click = |harness: &mut egui_kittest::Harness<'_, (Session, TimingState)>, pos| {
            for pressed in [true, false] {
                harness.input_mut().events.extend([
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        pressed,
                        button: egui::PointerButton::Primary,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                harness.run_steps(1);
            }
        };
        // The top of the scrolling part: the first height where a click
        // on the frames moves the playhead.
        let top = (40..200)
            .map(|y| y as f32)
            .find(|&y| {
                harness.state_mut().0.frame = 1;
                click(&mut harness, Pos2::new(400.0, y));
                harness.state().0.frame != 1
            })
            .expect("the ruler");
        harness.input_mut().events.extend([
            egui::Event::PointerMoved(Pos2::new(400.0, 300.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, -240.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        harness.run_steps(8);
        // Rows are under the ruler now. A click on it still only moves
        // the playhead.
        let selected = harness.state().0.selected;
        for dy in [6.0, 14.0, 22.0] {
            harness.state_mut().0.frame = 1;
            click(&mut harness, Pos2::new(400.0, top + dy));
            assert_ne!(harness.state().0.frame, 1);
            assert_eq!(harness.state().0.selected, selected);
        }
        // Below the ruler, a click picks the row there.
        click(&mut harness, Pos2::new(400.0, top + 28.0 + 30.0));
        assert_ne!(harness.state().0.selected, selected);
    }

    #[test]
    fn ctrl_wheel_zooms_the_frames_of_the_timeline() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(Vec2::new(800.0, 380.0))
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, (session, timing)| {
                    timeline(
                        ui,
                        session,
                        timing,
                        &SoundState::default(),
                        Profile::Symphony,
                    )
                },
                (Session::new_scene(), TimingState::default()),
            );
        harness.run_steps(3);
        let at = Pos2::new(400.0, 200.0);
        let frame_at = |harness: &mut egui_kittest::Harness<'_, (Session, TimingState)>, pos| {
            for pressed in [true, false] {
                harness.input_mut().events.extend([
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        pressed,
                        button: egui::PointerButton::Primary,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                harness.run_steps(1);
            }
            harness.state().0.frame
        };
        let before = frame_at(&mut harness, at);
        // egui turns Ctrl+wheel into a zoom.
        harness
            .input_mut()
            .events
            .extend([egui::Event::PointerMoved(at), egui::Event::Zoom(2.0)]);
        harness.run_steps(3);
        assert_eq!(harness.state().1.frame_w, Some(2.0 * theme::FRAME_CELL));
        // The frame under the pointer stayed there, and frames are twice
        // as wide: four cells further on is two frames later.
        assert_eq!(frame_at(&mut harness, at), before);
        let further = at + Vec2::new(4.0 * theme::FRAME_CELL, 0.0);
        assert_eq!(frame_at(&mut harness, further), before + 2);
        // The zoom stops at its limits.
        for _ in 0..8 {
            harness
                .input_mut()
                .events
                .extend([egui::Event::PointerMoved(at), egui::Event::Zoom(0.25)]);
            harness.run_steps(1);
        }
        assert_eq!(harness.state().1.frame_w, Some(2.0));
    }

    #[test]
    fn ruler_beats_follow_the_frame_rate() {
        assert_eq!(ruler_steps(24.0), [6, 12, 24]);
        assert_eq!(ruler_steps(30.0), [5, 15, 30]);
        assert_eq!(ruler_steps(12.0), [3, 6, 12]);
        assert_eq!(ruler_steps(25.0), [5, 25, 25]);
        assert_eq!(ruler_steps(60.0), [15, 30, 60]);
    }
    use symphony_core::graph::Effect;

    /// Press, drag and release `button` across the Timeline's tracks;
    /// returns the frame it ends on.
    fn drag_on_timeline(button: egui::PointerButton) -> u32 {
        let mut h = egui_kittest::Harness::builder()
            .with_size(Vec2::new(900.0, 320.0))
            // The header's frame-rate box has no name yet; not this test's job.
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, (s, t, sounds)| timeline(ui, s, t, sounds, Profile::Symphony),
                (
                    Session::new_scene(),
                    TimingState::default(),
                    SoundState::default(),
                ),
            );
        h.state_mut().0.frame = 1;
        h.run_steps(2);
        let none = egui::Modifiers::NONE;
        let (from, to) = (Pos2::new(420.0, 200.0), Pos2::new(560.0, 220.0));
        let mut send = |pos: Pos2, pressed: Option<bool>| {
            h.input_mut().events.push(egui::Event::PointerMoved(pos));
            if let Some(pressed) = pressed {
                h.input_mut().events.push(egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: none,
                });
            }
            h.run_steps(1);
        };
        send(from, Some(true));
        for k in 1..=6 {
            send(from + (to - from) * (k as f32 / 6.0), None);
        }
        send(to, Some(false));
        h.state().0.frame
    }

    #[test]
    fn a_middle_drag_on_the_timeline_does_not_move_the_playhead() {
        assert!(
            drag_on_timeline(egui::PointerButton::Primary) > 1,
            "the primary button scrubs (checks the test reaches the tracks)"
        );
        assert_eq!(drag_on_timeline(egui::PointerButton::Middle), 1);
    }

    /// Nodes added from the Timeline go to the nearest fitting place, and
    /// the Timeline lists each layer's pegs right above it.
    #[test]
    fn added_nodes_join_the_hierarchy_and_the_timeline_follows_it() {
        let mut s = Session::new_scene();
        let layer = s.doc.layers[0];
        let old_peg = s
            .doc
            .graph
            .input(layer.node, 0)
            .expect("layers start with a peg");
        // A peg on the drawing: a new parent between it and its old peg.
        super::super::node::insert_near(&mut s, layer.node, NodeKind::Peg(Default::default()));
        let new_peg = s.doc.graph.input(layer.node, 0).unwrap();
        assert_ne!(new_peg, old_peg);
        assert_eq!(s.doc.graph.input(new_peg, 0), Some(old_peg));
        // An effect from the (old) peg: after that layer's drawing.
        let out_before = s
            .doc
            .graph
            .links()
            .iter()
            .find(|l| l.from == layer.node)
            .copied()
            .unwrap();
        super::super::node::insert_near(
            &mut s,
            old_peg,
            NodeKind::Effect(Effect::Blur {
                radius: 2.0,
                mode: symphony_core::graph::BlurMode::Box,
            }),
        );
        let fx = s
            .doc
            .graph
            .links()
            .iter()
            .find(|l| l.from == layer.node)
            .unwrap()
            .to;
        assert!(matches!(kind_of(&s, fx), Some(NodeKind::Effect(_))));
        assert!(s
            .doc
            .graph
            .links()
            .iter()
            .any(|l| l.from == fx && l.to == out_before.to));
        // The rows: old peg, new peg, then the layer, then its effect.
        let (pegs, effects) = layer_chain(&s, 0);
        assert_eq!(pegs, vec![old_peg, new_peg]);
        assert_eq!(effects, vec![fx]);
        let tracks = vec![fx, new_peg, old_peg];
        let rows = hierarchy_rows(&s, &tracks, 0);
        assert_eq!(
            &rows[..4],
            &[
                (Slot::Node(2), 0),
                (Slot::Node(1), 1),
                (Slot::Layer(0), 2),
                (Slot::Node(0), 3)
            ]
        );
        // With pegs hidden, the layer and its effect lose the indent.
        let rows = hierarchy_rows(&s, &[fx], 0);
        assert_eq!(&rows[..2], &[(Slot::Layer(0), 0), (Slot::Node(0), 1)]);
        // One undo each.
        s.undo();
        s.undo();
        assert_eq!(s.doc.graph.input(layer.node, 0), Some(old_peg));
    }
}
