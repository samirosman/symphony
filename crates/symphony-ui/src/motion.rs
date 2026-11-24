//! Symphony's motion vocabulary (DESIGN §9): one named motion per kind of
//! action, used everywhere, so the same action always moves the same way.
//!
//! | Motion    | For                                            | Time   | Curve          |
//! |-----------|------------------------------------------------|--------|----------------|
//! | `Appear`  | something opening (a fade): popups, hints      | 140 ms | ease-out cubic |
//! | `Dismiss` | something closing                              | 110 ms | ease-in quad   |
//! | `Expand`  | something growing or shrinking: lists, sizes   | 180 ms | ease-out quart |
//! | `Select`  | a selection or state changing: tabs, toggles   | 150 ms | ease-in-out    |
//! | `Settle`  | something landing in place: drops, snaps       | 220 ms | ease-out back  |
//!
//! With interface animations off (or the OS "reduce motion" setting) every
//! motion finishes at once. Motion never delays input (D9.2): values move
//! towards where they're going, and input acts on the final state.

use egui::{self, Id};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Appear,
    Dismiss,
    Expand,
    Select,
    Settle,
}

impl Motion {
    pub fn seconds(self) -> f64 {
        match self {
            Motion::Appear => 0.14,
            Motion::Dismiss => 0.11,
            Motion::Expand => 0.18,
            Motion::Select => 0.15,
            Motion::Settle => 0.22,
        }
    }

    /// Eased progress for linear time `t` in 0..=1.
    pub fn ease(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Motion::Appear => 1.0 - (1.0 - t).powi(3),
            Motion::Dismiss => t * t,
            Motion::Expand => 1.0 - (1.0 - t).powi(4),
            Motion::Select => 0.5 - 0.5 * (std::f32::consts::PI * t).cos(),
            Motion::Settle => {
                // Ease-out with a small overshoot, so a drop lands softly.
                let (c1, c3) = (1.2, 2.2);
                1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
            }
        }
    }
}

/// Is motion on for this context?
pub fn enabled(ctx: &egui::Context) -> bool {
    ctx.global_style().animation_time > 0.0
}

#[derive(Clone, Copy)]
struct Track {
    from: f32,
    to: f32,
    start: f64,
    motion: Motion,
}

/// Move a value towards `target` with `motion`, starting a new move from
/// wherever it is whenever the target changes. Returns the current value.
pub fn value(ctx: &egui::Context, id: Id, target: f32, motion: Motion) -> f32 {
    if !enabled(ctx) {
        ctx.data_mut(|d| d.remove::<Track>(id));
        return target;
    }
    let now = ctx.input(|i| i.time);
    let current = |t: &Track| {
        let p = ((now - t.start) / t.motion.seconds()) as f32;
        t.from + (t.to - t.from) * t.motion.ease(p)
    };
    let track = ctx.data_mut(|d| {
        let old = d.get_temp::<Track>(id);
        let track = match old {
            Some(t) if t.to == target => t,
            Some(t) => Track {
                from: current(&t),
                to: target,
                start: now,
                motion,
            },
            // First sight: start at the target (nothing to animate from).
            None => Track {
                from: target,
                to: target,
                start: now,
                motion,
            },
        };
        d.insert_temp(id, track);
        track
    });
    if now - track.start < track.motion.seconds() {
        ctx.request_repaint();
        current(&track)
    } else {
        track.to
    }
}

/// A rectangle that slides to `target` when it moves (Select motion): a
/// selection highlight glides from the old choice to the new one instead
/// of jumping. Returns where to draw it this frame.
pub fn slide(ctx: &egui::Context, id: Id, target: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(
            value(ctx, id.with("x0"), target.min.x, Motion::Select),
            value(ctx, id.with("y0"), target.min.y, Motion::Select),
        ),
        egui::pos2(
            value(ctx, id.with("x1"), target.max.x, Motion::Select),
            value(ctx, id.with("y1"), target.max.y, Motion::Select),
        ),
    )
}

/// Put a value at `to` with no movement, so a later `value` moves on from
/// there (something the pointer is carrying: when it's let go, it must
/// not glide in from where it was before).
pub fn jump(ctx: &egui::Context, id: Id, to: f32) {
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| {
        d.insert_temp(
            id,
            Track {
                from: to,
                to,
                start: now,
                motion: Motion::Select,
            },
        )
    });
}

/// Start a value at `from` and move it to `to`, e.g. for something that
/// appears: `start(ctx, id, 0.0)` the frame it opens.
pub fn start(ctx: &egui::Context, id: Id, from: f32) {
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| {
        d.insert_temp(
            id,
            Track {
                from,
                to: from,
                start: now,
                motion: Motion::Appear,
            },
        )
    });
}

/// Presence of something that opens and closes: 0 hidden .. 1 shown,
/// appearing with `Appear` and leaving with `Dismiss`. Draw it while the
/// result is above zero.
pub fn presence(ctx: &egui::Context, id: Id, shown: bool) -> f32 {
    let target = if shown { 1.0 } else { 0.0 };
    let motion = if shown {
        Motion::Appear
    } else {
        Motion::Dismiss
    };
    // Something seen for the first time while shown should still appear.
    if shown && ctx.data(|d| d.get_temp::<Track>(id).is_none()) {
        start(ctx, id, 0.0);
    }
    value(ctx, id, target, motion)
}

/// How a present thing looks at presence `p`. Things that open and close
/// only fade (Tristan's call): full size, in place, at every moment.
pub struct Look {
    pub opacity: f32,
    pub scale: f32,
    pub drop: f32,
}

pub fn look(p: f32) -> Look {
    Look {
        opacity: p.clamp(0.0, 1.0),
        scale: 1.0,
        drop: 0.0,
    }
}

/// Mix two colours for a `Select` change (`t` 0 = `a`, 1 = `b`).
pub fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let (a, b) = (a.to_srgba_unmultiplied(), b.to_srgba_unmultiplied());
    let c = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8;
    egui::Color32::from_rgba_unmultiplied(c(0), c(1), c(2), c(3))
}

#[cfg(test)]
mod tests {
    /// Something carried by the pointer and let go where it is doesn't
    /// glide in from where it started (a dropped node used to swoop).
    #[test]
    fn a_jumped_value_lands_without_moving() {
        let ctx = egui::Context::default();
        let id = Id::unique("carried");
        let mut out = ctx.run_ui(Default::default(), |_| {
            assert_eq!(value(&ctx, id, 0.0, Motion::Select), 0.0);
        });
        out.textures_delta.clear();
        let mut out = ctx.run_ui(Default::default(), |_| {
            // Carried to 100, then let go there.
            jump(&ctx, id, 100.0);
            assert_eq!(value(&ctx, id, 100.0, Motion::Select), 100.0);
        });
        out.textures_delta.clear();
    }

    use super::*;

    #[test]
    fn every_motion_starts_at_zero_and_ends_at_one() {
        for m in [
            Motion::Appear,
            Motion::Dismiss,
            Motion::Expand,
            Motion::Select,
            Motion::Settle,
        ] {
            assert!(m.ease(0.0).abs() < 1e-5, "{m:?}");
            assert!((m.ease(1.0) - 1.0).abs() < 1e-5, "{m:?}");
            assert!(m.seconds() <= 0.24, "D9.1: short");
        }
        // Settle overshoots a little before landing.
        assert!((0..100).any(|i| Motion::Settle.ease(i as f32 / 100.0) > 1.0));
    }
}
