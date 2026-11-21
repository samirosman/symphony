//! Actions registry and per-profile shortcuts (docs/ENGINEERING.md E4.3–E4.6,
//! docs/DESIGN.md §11). Every shortcut in the app is defined here and
//! nowhere else.
//!
//! Harmony-profile bindings come from Harmony's published default
//! shortcuts. They're marked `verified: false` until each one is checked
//! against the Harmony 27 list (E4.4). The Symphony profile starts as a
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
    PrevFrame,
    NextFrame,
    PlayPause,
    Undo,
    MaximiseView,
    ToolSelect,
    ToolTransform,
    ToolBrush,
    ToolPencil,
    ToolEraser,
    ToolPaint,
    ToolDropper,
    ToolHand,
    ToolZoom,
}

impl Action {
    pub const ALL: [Action; 14] = [
        Action::PrevFrame,
        Action::NextFrame,
        Action::PlayPause,
        Action::Undo,
        Action::MaximiseView,
        Action::ToolSelect,
        Action::ToolTransform,
        Action::ToolBrush,
        Action::ToolPencil,
        Action::ToolEraser,
        Action::ToolPaint,
        Action::ToolDropper,
        Action::ToolHand,
        Action::ToolZoom,
    ];

    /// Stable ID, used for settings and scripting.
    pub fn id(self) -> &'static str {
        match self {
            Action::PrevFrame => "frame.previous",
            Action::NextFrame => "frame.next",
            Action::PlayPause => "playback.toggle",
            Action::Undo => "edit.undo",
            Action::MaximiseView => "view.maximise",
            Action::ToolSelect => "tool.select",
            Action::ToolTransform => "tool.transform",
            Action::ToolBrush => "tool.brush",
            Action::ToolPencil => "tool.pencil",
            Action::ToolEraser => "tool.eraser",
            Action::ToolPaint => "tool.paint",
            Action::ToolDropper => "tool.dropper",
            Action::ToolHand => "tool.hand",
            Action::ToolZoom => "tool.zoom",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Action::PrevFrame => "Previous frame",
            Action::NextFrame => "Next frame",
            Action::PlayPause => "Play / pause",
            Action::Undo => "Undo",
            Action::MaximiseView => "Maximise view",
            Action::ToolSelect => "Select",
            Action::ToolTransform => "Transform",
            Action::ToolBrush => "Brush",
            Action::ToolPencil => "Pencil",
            Action::ToolEraser => "Eraser",
            Action::ToolPaint => "Paint",
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
        verified: false,
    })
}

const ALT: Modifiers = Modifiers::ALT;
const SHIFT: Modifiers = Modifiers::SHIFT;
const NONE: Modifiers = Modifiers::NONE;
const CMD: Modifiers = Modifiers::COMMAND;

fn harmony(action: Action) -> Option<Binding> {
    match action {
        Action::PrevFrame => b(Key::Comma, NONE),
        Action::NextFrame => b(Key::Period, NONE),
        Action::PlayPause => b(Key::Enter, NONE),
        Action::Undo => b(Key::Z, CMD),
        Action::MaximiseView => b(Key::Backtick, NONE),
        Action::ToolSelect => b(Key::S, ALT),
        Action::ToolTransform => b(Key::T, SHIFT),
        Action::ToolBrush => b(Key::B, ALT),
        Action::ToolPencil => b(Key::Slash, ALT),
        Action::ToolEraser => b(Key::E, ALT),
        Action::ToolPaint => b(Key::I, ALT),
        Action::ToolDropper => b(Key::D, ALT),
        Action::ToolHand => None,
        Action::ToolZoom => b(Key::Z, ALT),
    }
}

/// Symphony profile: Harmony's bindings unless an improvement is logged
/// in DESIGN.md §11.1. There are none yet.
fn symphony(action: Action) -> Option<Binding> {
    harmony(action)
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
    ctx.input(|i| i.key_pressed(bind.key) && i.modifiers.matches_exact(bind.mods))
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
