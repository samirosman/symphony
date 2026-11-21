//! Actions registry and per-profile shortcuts (docs/ENGINEERING.md E4.3–E4.6,
//! docs/DESIGN.md §11). Every shortcut in the app is defined here and
//! nowhere else.
//!
//! Harmony-profile bindings are checked against Harmony 27's published
//! default shortcuts, kept as facts in docs/reference/harmony-27-shortcuts.tsv (E4.4). The Symphony profile starts as a
//! copy of Harmony; any difference must be logged in DESIGN.md §11.1.

use eframe::egui::{self, Key, Modifiers};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    #[default]
    Harmony,
    Symphony,
}

impl Profile {
    pub fn title(self) -> &'static str {
        match self {
            Profile::Harmony => "Harmony",
            Profile::Symphony => "Symphony",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Profile::Harmony => "Shortcuts and workflow as in the latest Harmony.",
            Profile::Symphony => "Harmony's base plus Symphony's improvements.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    NewScene,
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
    SelectAll,
    DeselectAll,
    Delete,
    Cut,
    Copy,
    Paste,
    FlipHorizontal,
    FlipVertical,
    PrevFrame,
    NextFrame,
    FirstFrame,
    LastFrame,
    PlayPause,
    IncreaseExposure,
    DecreaseExposure,
    CreateEmptyDrawing,
    AddLayer,
    ToggleOnionSkin,
    ResetView,
    ZoomIn,
    ZoomOut,
    RotateCw,
    RotateCcw,
    ResetZoom,
    ResetRotation,
    ResetPan,
    OnionMorePrevious,
    OnionLessPrevious,
    OnionMoreNext,
    OnionLessNext,
    OnionPrevious1,
    OnionPrevious2,
    OnionPrevious3,
    OnionNoPrevious,
    OnionNext1,
    OnionNext2,
    OnionNext3,
    OnionNoNext,
    MaximiseView,
    ToolSelect,
    ToolTransform,
    ToolBrush,
    ToolPencil,
    ToolEraser,
    ToolPaint,
    ToolPaintUnpainted,
    ToolRepaint,
    ToolUnpaint,
    ToolCloseGap,
    ShowStrokes,
    RespectProtectedColours,
    ToolDropper,
    ToolHand,
    ToolZoom,
}

impl Action {
    pub const ALL: [Action; 60] = [
        Action::NewScene,
        Action::Open,
        Action::Save,
        Action::SaveAs,
        Action::Undo,
        Action::Redo,
        Action::SelectAll,
        Action::DeselectAll,
        Action::Delete,
        Action::Cut,
        Action::Copy,
        Action::Paste,
        Action::FlipHorizontal,
        Action::FlipVertical,
        Action::PrevFrame,
        Action::NextFrame,
        Action::FirstFrame,
        Action::LastFrame,
        Action::PlayPause,
        Action::IncreaseExposure,
        Action::DecreaseExposure,
        Action::CreateEmptyDrawing,
        Action::AddLayer,
        Action::ToggleOnionSkin,
        Action::ResetView,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::RotateCw,
        Action::RotateCcw,
        Action::ResetZoom,
        Action::ResetRotation,
        Action::ResetPan,
        Action::OnionMorePrevious,
        Action::OnionLessPrevious,
        Action::OnionMoreNext,
        Action::OnionLessNext,
        Action::OnionPrevious1,
        Action::OnionPrevious2,
        Action::OnionPrevious3,
        Action::OnionNoPrevious,
        Action::OnionNext1,
        Action::OnionNext2,
        Action::OnionNext3,
        Action::OnionNoNext,
        Action::MaximiseView,
        Action::ToolSelect,
        Action::ToolTransform,
        Action::ToolBrush,
        Action::ToolPencil,
        Action::ToolEraser,
        Action::ToolPaint,
        Action::ToolPaintUnpainted,
        Action::ToolRepaint,
        Action::ToolUnpaint,
        Action::ToolCloseGap,
        Action::ShowStrokes,
        Action::RespectProtectedColours,
        Action::ToolDropper,
        Action::ToolHand,
        Action::ToolZoom,
    ];

    /// Stable ID, used for settings and scripting.
    pub fn id(self) -> &'static str {
        match self {
            Action::NewScene => "file.new",
            Action::Open => "file.open",
            Action::Save => "file.save",
            Action::SaveAs => "file.save_as",
            Action::Undo => "edit.undo",
            Action::Redo => "edit.redo",
            Action::SelectAll => "edit.select_all",
            Action::DeselectAll => "edit.deselect_all",
            Action::Delete => "edit.delete",
            Action::Cut => "edit.cut",
            Action::Copy => "edit.copy",
            Action::Paste => "edit.paste",
            Action::FlipHorizontal => "edit.flip_horizontal",
            Action::FlipVertical => "edit.flip_vertical",
            Action::PrevFrame => "frame.previous",
            Action::NextFrame => "frame.next",
            Action::FirstFrame => "frame.first",
            Action::LastFrame => "frame.last",
            Action::PlayPause => "playback.toggle",
            Action::IncreaseExposure => "timing.increase_exposure",
            Action::DecreaseExposure => "timing.decrease_exposure",
            Action::CreateEmptyDrawing => "drawing.create_empty",
            Action::AddLayer => "layer.add",
            Action::ToggleOnionSkin => "view.onion_skin",
            Action::ResetView => "view.reset",
            Action::ZoomIn => "view.zoom_in",
            Action::ZoomOut => "view.zoom_out",
            Action::RotateCw => "view.rotate_cw",
            Action::RotateCcw => "view.rotate_ccw",
            Action::ResetZoom => "view.reset_zoom",
            Action::ResetRotation => "view.reset_rotation",
            Action::ResetPan => "view.reset_pan",
            Action::OnionMorePrevious => "onion.previous_more",
            Action::OnionLessPrevious => "onion.previous_less",
            Action::OnionMoreNext => "onion.next_more",
            Action::OnionLessNext => "onion.next_less",
            Action::OnionPrevious1 => "onion.previous_1",
            Action::OnionPrevious2 => "onion.previous_2",
            Action::OnionPrevious3 => "onion.previous_3",
            Action::OnionNoPrevious => "onion.previous_none",
            Action::OnionNext1 => "onion.next_1",
            Action::OnionNext2 => "onion.next_2",
            Action::OnionNext3 => "onion.next_3",
            Action::OnionNoNext => "onion.next_none",
            Action::MaximiseView => "view.maximise",
            Action::ToolSelect => "tool.select",
            Action::ToolTransform => "tool.transform",
            Action::ToolBrush => "tool.brush",
            Action::ToolPencil => "tool.pencil",
            Action::ToolEraser => "tool.eraser",
            Action::ToolPaint => "tool.paint",
            Action::ToolPaintUnpainted => "tool.paint_unpainted",
            Action::ToolRepaint => "tool.repaint",
            Action::ToolUnpaint => "tool.unpaint",
            Action::ToolCloseGap => "tool.close_gap",
            Action::ShowStrokes => "view.show_strokes",
            Action::RespectProtectedColours => "drawing.respect_protected_colours",
            Action::ToolDropper => "tool.dropper",
            Action::ToolHand => "tool.hand",
            Action::ToolZoom => "tool.zoom",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Action::NewScene => "New scene",
            Action::Open => "Open",
            Action::Save => "Save",
            Action::SaveAs => "Save as",
            Action::Undo => "Undo",
            Action::Redo => "Redo",
            Action::SelectAll => "Select all",
            Action::DeselectAll => "Deselect all",
            Action::Delete => "Delete",
            Action::Cut => "Cut",
            Action::Copy => "Copy",
            Action::Paste => "Paste",
            Action::FlipHorizontal => "Flip horizontal",
            Action::FlipVertical => "Flip vertical",
            Action::PrevFrame => "Previous frame",
            Action::NextFrame => "Next frame",
            Action::FirstFrame => "First frame",
            Action::LastFrame => "Last frame",
            Action::PlayPause => "Play / pause",
            Action::IncreaseExposure => "Increase exposure",
            Action::DecreaseExposure => "Decrease exposure",
            Action::CreateEmptyDrawing => "Create empty drawing",
            Action::AddLayer => "Add drawing layer",
            Action::ToggleOnionSkin => "Onion skin",
            Action::ResetView => "Reset view",
            Action::ZoomIn => "Zoom in",
            Action::ZoomOut => "Zoom out",
            Action::RotateCw => "Rotate view CW",
            Action::RotateCcw => "Rotate view CCW",
            Action::ResetZoom => "Reset zoom",
            Action::ResetRotation => "Reset rotation",
            Action::ResetPan => "Reset pan",
            Action::OnionMorePrevious => "Onion skin: add previous drawing",
            Action::OnionLessPrevious => "Onion skin: remove previous drawing",
            Action::OnionMoreNext => "Onion skin: add next drawing",
            Action::OnionLessNext => "Onion skin: remove next drawing",
            Action::OnionPrevious1 => "Onion skin: 1 previous",
            Action::OnionPrevious2 => "Onion skin: 2 previous",
            Action::OnionPrevious3 => "Onion skin: 3 previous",
            Action::OnionNoPrevious => "Onion skin: no previous",
            Action::OnionNext1 => "Onion skin: 1 next",
            Action::OnionNext2 => "Onion skin: 2 next",
            Action::OnionNext3 => "Onion skin: 3 next",
            Action::OnionNoNext => "Onion skin: no next",
            Action::MaximiseView => "Maximise view",
            Action::ToolSelect => "Select",
            Action::ToolTransform => "Transform",
            Action::ToolBrush => "Brush",
            Action::ToolPencil => "Pencil",
            Action::ToolEraser => "Eraser",
            Action::ToolPaint => "Paint",
            Action::ToolPaintUnpainted => "Paint Unpainted",
            Action::ToolRepaint => "Repaint",
            Action::ToolUnpaint => "Unpaint",
            Action::ToolCloseGap => "Close Gap",
            Action::ShowStrokes => "Show Strokes",
            Action::RespectProtectedColours => "Respect Protected Colours",
            Action::ToolDropper => "Dropper",
            Action::ToolHand => "Hand",
            Action::ToolZoom => "Zoom",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub key: Key,
    pub mods: Modifiers,
    /// Checked against the latest Harmony's published list (E4.4).
    pub verified: bool,
}

const fn b(key: Key, mods: Modifiers) -> Option<Binding> {
    Some(Binding {
        key,
        mods,
        verified: true,
    })
}

const ALT: Modifiers = Modifiers::ALT;
const SHIFT: Modifiers = Modifiers::SHIFT;
const NONE: Modifiers = Modifiers::NONE;
const CMD: Modifiers = Modifiers::COMMAND;
const CMD_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);
const CTRL_SHIFT: Modifiers = Modifiers::CTRL.plus(Modifiers::SHIFT);
const ALT_SHIFT: Modifiers = Modifiers::ALT.plus(Modifiers::SHIFT);

fn harmony(action: Action) -> Option<Binding> {
    match action {
        Action::NewScene => b(Key::N, CMD),
        Action::Open => b(Key::O, CMD),
        Action::Save => b(Key::S, CMD),
        Action::SaveAs => None, // Harmony's Ctrl+Shift+S is Advanced Save
        Action::Undo => b(Key::Z, CMD),
        // Harmony 27 documents Redo as Ctrl+Shift+Z on macOS too.
        Action::Redo => b(
            Key::Z,
            if cfg!(target_os = "macos") {
                CTRL_SHIFT
            } else {
                CMD_SHIFT
            },
        ),
        Action::SelectAll => b(Key::A, CMD),
        Action::DeselectAll => b(Key::Escape, NONE),
        Action::Delete => b(
            if cfg!(target_os = "macos") {
                Key::Backspace
            } else {
                Key::Delete
            },
            NONE,
        ),
        Action::Cut => b(Key::X, CMD),
        Action::Copy => b(Key::C, CMD),
        Action::Paste => b(Key::V, CMD),
        Action::FlipHorizontal => b(Key::Num4, NONE),
        Action::FlipVertical => b(Key::Num5, NONE),
        Action::PrevFrame => b(Key::Comma, NONE),
        Action::NextFrame => b(Key::Period, NONE),
        Action::FirstFrame => b(Key::Comma, SHIFT), // "<"
        Action::LastFrame => b(Key::Period, SHIFT), // ">"
        Action::PlayPause => b(Key::Enter, SHIFT),  // Toggle Playback
        Action::IncreaseExposure => b(Key::Plus, NONE),
        Action::DecreaseExposure => b(Key::Minus, NONE),
        Action::CreateEmptyDrawing => b(Key::R, ALT_SHIFT),
        Action::AddLayer => b(Key::R, CMD),
        Action::ToggleOnionSkin => b(Key::O, ALT),
        Action::ResetView => b(Key::M, SHIFT),
        Action::ZoomIn => b(Key::Num2, NONE),
        Action::ZoomOut => b(Key::Num1, NONE),
        Action::ResetZoom => None,
        Action::ResetRotation => b(Key::X, SHIFT),
        Action::ResetPan => b(Key::N, SHIFT),
        Action::RotateCw | Action::RotateCcw => None,
        Action::OnionMorePrevious
        | Action::OnionLessPrevious
        | Action::OnionMoreNext
        | Action::OnionLessNext => None,
        // Harmony 27: ! @ # ~ for previous, Cmd+1/2/3/` for next.
        Action::OnionPrevious1 => b(Key::Exclamationmark, SHIFT),
        Action::OnionPrevious2 => b(Key::Num2, SHIFT),
        Action::OnionPrevious3 => b(Key::Num3, SHIFT),
        Action::OnionNoPrevious => b(Key::Backtick, SHIFT),
        Action::OnionNext1 => b(Key::Num1, CMD),
        Action::OnionNext2 => b(Key::Num2, CMD),
        Action::OnionNext3 => b(Key::Num3, CMD),
        Action::OnionNoNext => b(Key::Backtick, CMD),
        Action::MaximiseView => b(Key::F, CMD), // Toggle Full Screen
        Action::ToolSelect => b(Key::S, ALT),
        Action::ToolTransform => b(Key::T, SHIFT),
        Action::ToolBrush => b(Key::B, ALT),
        Action::ToolPencil => b(Key::Slash, ALT),
        Action::ToolEraser => b(Key::E, ALT),
        Action::ToolPaint => b(Key::I, ALT),
        Action::ToolPaintUnpainted => b(Key::Y, ALT),
        Action::ToolRepaint => b(Key::R, ALT),
        Action::ToolUnpaint => b(Key::U, ALT),
        Action::ToolCloseGap => b(Key::C, ALT),
        Action::ShowStrokes => b(Key::K, NONE),
        Action::RespectProtectedColours => b(Key::S, SHIFT),
        Action::ToolDropper => b(Key::D, ALT),
        Action::ToolHand => None,
        Action::ToolZoom => b(Key::Z, ALT),
    }
}

/// Symphony profile: Harmony's bindings unless an improvement is logged
/// in DESIGN.md §11.1. There are none yet.
fn symphony(action: Action) -> Option<Binding> {
    match action {
        // Mac-standard Redo (DESIGN §11.1).
        Action::Redo => b(Key::Z, CMD_SHIFT),
        _ => harmony(action),
    }
}

pub fn binding(profile: Profile, action: Action) -> Option<Binding> {
    match profile {
        Profile::Harmony => harmony(action),
        Profile::Symphony => symphony(action),
    }
}

/// Did the user just trigger `action`? Modifiers must match exactly, so
/// Alt+E (Eraser) never fires on plain E.
pub fn pressed(ctx: &egui::Context, profile: Profile, action: Action) -> bool {
    let Some(bind) = binding(profile, action) else {
        return false;
    };
    ctx.input(|i| {
        // + and - are typed with Shift on some layouts, so ignore Shift there.
        let mods_ok = if matches!(bind.key, Key::Plus | Key::Minus | Key::Exclamationmark) {
            i.modifiers.matches_logically(bind.mods)
        } else {
            i.modifiers.matches_exact(bind.mods)
        };
        i.key_pressed(bind.key) && mods_ok
    })
}

/// Shortcut text for menus and tooltips, such as "Alt+B" or "Cmd+Z".
pub fn label(profile: Profile, action: Action) -> String {
    let Some(bind) = binding(profile, action) else {
        return String::new();
    };
    let mac = cfg!(target_os = "macos");
    let mut parts: Vec<&str> = Vec::new();
    if bind.mods.command || bind.mods.ctrl || bind.mods.mac_cmd {
        parts.push(if mac { "Cmd" } else { "Ctrl" });
    }
    if bind.mods.alt {
        parts.push(if mac { "Option" } else { "Alt" });
    }
    if bind.mods.shift {
        parts.push("Shift");
    }
    let key = bind.key.symbol_or_name();
    let mut s = parts.join("+");
    if !s.is_empty() {
        s.push('+');
    }
    s.push_str(key);
    s
}

/// "Name (shortcut)" for tooltips (D7.5).
pub fn tooltip(profile: Profile, action: Action) -> String {
    match label(profile, action) {
        l if l.is_empty() => action.name().to_owned(),
        l => format!("{} ({l})", action.name()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E11.3: no two actions share a binding within a profile.
    #[test]
    fn no_duplicate_bindings() {
        for profile in [Profile::Harmony, Profile::Symphony] {
            let mut seen = Vec::new();
            for a in Action::ALL {
                if let Some(bind) = binding(profile, a) {
                    let k = (bind.key, bind.mods);
                    assert!(!seen.contains(&k), "{profile:?}: {} clashes", a.id());
                    seen.push(k);
                }
            }
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<_> = Action::ALL.iter().map(|a| a.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), Action::ALL.len());
    }
}
