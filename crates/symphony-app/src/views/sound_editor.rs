//! The Sound editor: a floating window over one sound track, opened by a
//! double-click on the track in the Timeline (Harmony's Sound Element
//! Editor). It shows the whole file with a large waveform, so the start and
//! end of the part that plays can be set finer than the Timeline allows.
//!
//! Edits apply at once, each one undoable, so there is nothing to confirm
//! (D8.4). The window doesn't block the rest of the app (D8.3).

use eframe::egui::{self, Align2, Pos2, Rect, Sense, Stroke, Vec2};
use symphony_core::doc::{Media, SoundClip};
use symphony_core::id::Id;
use symphony_ui::{theme, widgets};

use crate::session::Session;
use crate::sound::SoundState;

/// Height of the waveform, not counting its frame ruler.
const WAVE_H: f32 = 160.0;
/// Height of the frame ruler over the waveform.
const RULER_H: f32 = 20.0;
/// Pixels on each side of a trim edge that grab it.
const EDGE: f32 = 6.0;
/// Height of the overview bar under the waveform.
/// The minimum pen target (D6.2).
const OVERVIEW_H: f32 = 24.0;
/// Most zoom: the view shows this many scene frames.
const FRAMES_IN_VIEW_MIN: f64 = 25.0;

/// Which trim edge a drag holds.
#[derive(Clone, Copy, PartialEq)]
enum Edge {
    Start,
    End,
}

/// What a drag in the overview bar holds, as on a Timeline sound block.
#[derive(Clone, Copy)]
enum ViewDrag {
    /// The left edge of the view box: zooms, the right edge stays.
    Start,
    /// The right edge: zooms, the left edge stays.
    End,
    /// The middle, held this many seconds right of the view's left edge.
    Body(f64),
}

pub struct SoundEditor {
    id: Id,
    name: String,
    /// Seconds of the file in view; `None` shows the whole file.
    visible: Option<f64>,
    /// A drag in the overview bar.
    view_drag: Option<ViewDrag>,
    /// File time at the left edge of the view, in seconds.
    left: f64,
    /// Trim drags snap to whole scene frames, as in the Timeline.
    snap: bool,
    /// The edge being dragged, and the track as it was when the drag began.
    drag: Option<(Edge, SoundClip)>,
}

impl SoundEditor {
    pub fn new(track: &SoundClip) -> SoundEditor {
        SoundEditor {
            id: track.id,
            name: track.name.clone(),
            visible: None,
            view_drag: None,
            left: 0.0,
            snap: true,
            drag: None,
        }
    }
}

/// Show the editor, if one is open. Closing it, or the track going away
/// (deleted, or undone), ends it.
pub fn show(
    ctx: &egui::Context,
    editor: &mut Option<SoundEditor>,
    s: &mut Session,
    sounds: &mut SoundState,
) {
    let Some(ed) = editor.as_mut() else {
        return;
    };
    let Some(track) = s.doc.sounds.iter().find(|t| t.id == ed.id).cloned() else {
        sounds.stop_preview();
        *editor = None;
        return;
    };
    let Some(file) = s.doc.media.get(&track.media).cloned() else {
        *editor = None;
        return;
    };
    let mut open = true;
    let mut close = false;
    egui::Window::new("Sound editor")
        .id(egui::Id::unique("sound-editor"))
        .open(&mut open)
        .collapsible(false)
        .resizable([true, false])
        .default_width(720.0)
        .show(ctx, |ui| {
            close = body(ui, ed, s, sounds, &track, &file);
        });
    if !open || close {
        if ed.drag.take().is_some() {
            s.end_gesture();
        }
        sounds.stop_preview();
        *editor = None;
    } else if sounds.previewing() {
        ctx.request_repaint();
    }
}

/// The window's contents. `true` when Close was pressed.
fn body(
    ui: &mut egui::Ui,
    ed: &mut SoundEditor,
    s: &mut Session,
    sounds: &mut SoundState,
    track: &SoundClip,
    file: &Media,
) -> bool {
    let fps = s.doc.settings.fps.max(1.0);
    ui.horizontal(|ui| {
        ui.label("Name");
        let resp = ui.add(egui::TextEdit::singleline(&mut ed.name).desired_width(240.0));
        if resp.lost_focus() {
            if ed.name.trim().is_empty() {
                ed.name = track.name.clone();
            } else {
                s.rename_sound(track.id, &ed.name);
            }
        } else if !resp.has_focus() && ed.name != track.name {
            // Renamed elsewhere, or by undo.
            ed.name = track.name.clone();
        }
        let mut on = !track.muted;
        if widgets::checkbox(ui, &mut on, "Play in scene").changed() {
            s.toggle_sound(track.id);
        }
    });
    ui.add_space(8.0);

    waveform_view(ui, ed, s, sounds, track, file, fps);

    ui.horizontal(|ui| {
        widgets::checkbox(ui, &mut ed.snap, "Snap to frames");
    });
    ui.add_space(8.0);

    // The same values as numbers, to type or drag (D8.8).
    let rate = f64::from(file.rate.max(1));
    let last = symphony_core::sound::last_frame(track, file, fps);
    egui::Grid::new("sound-editor-fields")
        .num_columns(4)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            ui.label("Start frame");
            let mut start = track.start;
            if ui
                .add(egui::DragValue::new(&mut start).range(1..=u32::MAX / 2))
                .changed()
            {
                field_edit(s, track.moved_to(start), "Move sound");
            }
            ui.label("End frame");
            let mut end = last;
            if ui
                .add(egui::DragValue::new(&mut end).range(track.start..=u32::MAX / 2))
                .changed()
            {
                let frames = i64::from(end) - i64::from(last);
                field_edit(s, track.trimmed_end(file, fps, frames), "Trim sound");
            }
            ui.end_row();

            ui.label("Trim start");
            let mut secs = track.trim_start as f64 / rate;
            if ui
                .add(
                    egui::DragValue::new(&mut secs)
                        .range(0.0..=file.sound_seconds())
                        .speed(0.001)
                        .fixed_decimals(3)
                        .suffix(" s"),
                )
                .changed()
            {
                let samples = (secs * rate).round() as u64;
                field_edit(s, track.with_trim_start(file, fps, samples), "Trim sound");
            }
            ui.label("Trim end");
            let mut secs = track.trim_end as f64 / rate;
            if ui
                .add(
                    egui::DragValue::new(&mut secs)
                        .range(0.0..=file.sound_seconds())
                        .speed(0.001)
                        .fixed_decimals(3)
                        .suffix(" s"),
                )
                .changed()
            {
                let samples = (secs * rate).round() as u64;
                field_edit(s, track.with_trim_end(file, fps, samples), "Trim sound");
            }
            ui.end_row();
        });
    let frames = last - track.start + 1;
    ui.label(
        egui::RichText::new(format!(
            "Plays {frames} frames ({:.3} s) of {:.3} s",
            track.seconds(file),
            file.sound_seconds()
        ))
        .font(theme::dense())
        .color(theme::TEXT_SECONDARY),
    );
    if !sounds.enabled() {
        ui.label(
            egui::RichText::new("Sound is off. Turn on Play > Enable Sound to hear it.")
                .font(theme::dense())
                .color(theme::TEXT_SECONDARY),
        );
    }
    ui.add_space(8.0);

    let mut close = false;
    ui.horizontal(|ui| {
        if sounds.previewing() {
            if ui.button("Stop").clicked() {
                sounds.stop_preview();
            }
        } else if ui.button("Play sound").clicked() {
            sounds.preview(s, track.id);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close = widgets::primary_button(ui, "Close", true).clicked();
        });
    });
    // Escape closes it while the pointer is over it (D8.6).
    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
    close || (esc && ui.ui_contains_pointer())
}

/// One change from a number field, as its own undo step.
fn field_edit(s: &mut Session, after: SoundClip, name: &str) {
    s.drag_sound(after, name);
    s.end_gesture();
}

/// Seconds in view at the most zoom: 25 scene frames, or the whole file
/// when it is shorter.
fn least_visible(seconds: f64, fps: f32) -> f64 {
    (FRAMES_IN_VIEW_MIN / f64::from(fps.max(1.0))).min(seconds)
}

/// The file at the editor's zoom, the part that plays lit and the rest
/// dimmed, with a ruler of the scene frames each part lands on. Drag an
/// edge to trim. Zoomed in, the wheel or the overview bar below moves
/// along the file.
fn waveform_view(
    ui: &mut egui::Ui,
    ed: &mut SoundEditor,
    s: &mut Session,
    sounds: &SoundState,
    track: &SoundClip,
    file: &Media,
    fps: f32,
) {
    let view_w = ui.available_width().max(1.0);
    let seconds = file.sound_seconds().max(1e-6);
    let visible = ed
        .visible
        .unwrap_or(seconds)
        .clamp(least_visible(seconds, fps), seconds);
    let spp = visible / f64::from(view_w);
    let offset = track.offset(file);
    let played = track.seconds(file);
    let size = Vec2::new(view_w, RULER_H + WAVE_H);
    let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    if resp.hovered() {
        let d = ui.input(|i| i.smooth_scroll_delta);
        ed.left -= f64::from(d.x + d.y) * spp;
    }
    ed.left = ed.left.clamp(0.0, (seconds - visible).max(0.0));
    let left = ed.left;
    let painter = painter.with_clip_rect(resp.rect);
    let o = resp.rect.min;
    let x_of = |t: f64| o.x + ((t - left) / spp) as f32;
    let t_of = |x: f32| (left + f64::from(x - o.x) * spp).clamp(0.0, seconds);
    let wave = Rect::from_min_size(Pos2::new(o.x, o.y + RULER_H), Vec2::new(view_w, WAVE_H));
    painter.rect_filled(wave, theme::RADIUS_CONTROL as f32, theme::BG_INPUT);
    let (x0, x1) = (x_of(offset), x_of(offset + played));
    let lit = Rect::from_min_max(Pos2::new(x0, wave.top()), Pos2::new(x1, wave.bottom()));
    painter.rect_filled(lit, 0.0, theme::BG_HOVER);
    draw_wave(&painter, sounds, track.media, wave, left, spp, x0, x1);
    ruler(&painter, track, file, fps, o, view_w, left, spp);
    for x in [x0, x1] {
        painter.line_segment(
            [
                Pos2::new(x, o.y + RULER_H / 2.0),
                Pos2::new(x, wave.bottom()),
            ],
            Stroke::new(2.0, theme::ACCENT_TIME),
        );
    }
    if let Some(t) = sounds.preview_time() {
        let into = t - f64::from(track.start.max(1) - 1) / f64::from(fps);
        let x = x_of(offset + into);
        painter.line_segment(
            [Pos2::new(x, wave.top()), Pos2::new(x, wave.bottom())],
            Stroke::new(1.5, theme::ACCENT_SELECT),
        );
    }

    // Drag an edge to trim it.
    let near = |x: f32| {
        if (x - x0).abs() <= EDGE {
            Some(Edge::Start)
        } else if (x - x1).abs() <= EDGE {
            Some(Edge::End)
        } else {
            None
        }
    };
    let pressed = ui.input(|i| i.pointer.press_origin());
    if resp.drag_started() {
        if let Some(edge) = pressed.and_then(|p| near(p.x)) {
            ed.drag = Some((edge, track.clone()));
        }
    }
    if ed.drag.is_some() || resp.hover_pos().and_then(|p| near(p.x)).is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if let (Some((edge, before)), Some(p)) = (&ed.drag, resp.interact_pointer_pos()) {
        let after = trim_to(before, file, fps, *edge, t_of(p.x), ed.snap);
        s.drag_sound(after, "Trim sound");
    }
    if resp.drag_stopped() && ed.drag.take().is_some() {
        s.end_gesture();
    }

    ui.add_space(4.0);
    overview(
        ui,
        ed,
        seconds,
        visible,
        least_visible(seconds, fps),
        offset,
        played,
    );
}

/// The whole file in a bar: the part that plays, and a box for the part
/// in view. Drag an edge of the box to zoom, its middle to move it; a
/// press outside the box moves the view there and holds it.
fn overview(
    ui: &mut egui::Ui,
    ed: &mut SoundEditor,
    seconds: f64,
    visible: f64,
    least: f64,
    offset: f64,
    played: f64,
) {
    let w = ui.available_width().max(1.0);
    let (resp, painter) = ui.allocate_painter(Vec2::new(w, OVERVIEW_H), Sense::click_and_drag());
    let r = resp.rect;
    let x_of = |t: f64| r.left() + (t / seconds) as f32 * w;
    let t_of = |x: f32| (f64::from((x - r.left()) / w) * seconds).clamp(0.0, seconds);
    painter.rect_filled(r, theme::RADIUS_CONTROL as f32, theme::BG_INPUT);
    let part = Rect::from_min_max(
        Pos2::new(x_of(offset), r.center().y - 1.0),
        Pos2::new(
            x_of(offset + played).max(x_of(offset) + 1.0),
            r.center().y + 1.0,
        ),
    );
    painter.rect_filled(part, 0.0, theme::ACCENT_TIME);
    let (v0, v1) = (x_of(ed.left), x_of(ed.left + visible));
    // Never thinner than two edges, so both stay in reach.
    let v1 = v1.max(v0 + 2.0 * EDGE);
    painter.rect_stroke(
        Rect::from_min_max(Pos2::new(v0, r.top()), Pos2::new(v1, r.bottom())),
        theme::RADIUS_CONTROL as f32,
        Stroke::new(1.5, theme::LINE_CONTROL),
        egui::StrokeKind::Inside,
    );

    let view_left = ed.left;
    let hit = |x: f32| {
        // The edge zones reach out of the box, so a thin box keeps a middle.
        if x >= v0 - EDGE && x <= v0 + EDGE.min((v1 - v0) / 3.0) {
            Some(ViewDrag::Start)
        } else if x >= v1 - EDGE.min((v1 - v0) / 3.0) && x <= v1 + EDGE {
            Some(ViewDrag::End)
        } else if x > v0 && x < v1 {
            Some(ViewDrag::Body(t_of(x) - view_left))
        } else {
            None
        }
    };
    let pressed = ui.input(|i| i.pointer.press_origin());
    if resp.is_pointer_button_down_on() && ed.view_drag.is_none() {
        if let Some(p) = pressed {
            ed.view_drag = Some(hit(p.x).unwrap_or_else(|| {
                // Outside the box: jump there, then hold the middle.
                ed.left = (t_of(p.x) - visible / 2.0).clamp(0.0, seconds - visible);
                ViewDrag::Body(visible / 2.0)
            }));
        }
    }
    let shown = ed
        .view_drag
        .or_else(|| resp.hover_pos().and_then(|p| hit(p.x)));
    match shown {
        Some(ViewDrag::Body(_)) if ed.view_drag.is_some() => {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing)
        }
        Some(ViewDrag::Body(_)) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
        Some(_) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
        None => {}
    }
    if let (Some(drag), Some(p)) = (ed.view_drag, resp.interact_pointer_pos()) {
        let t = t_of(p.x);
        let right = ed.left + visible;
        match drag {
            ViewDrag::Start => {
                let left = t.clamp(0.0, (right - least).max(0.0));
                ed.left = left;
                ed.visible = Some(right - left);
            }
            ViewDrag::End => {
                let end = t.clamp((ed.left + least).min(seconds), seconds);
                ed.visible = Some(end - ed.left);
            }
            ViewDrag::Body(grab) => {
                ed.left = (t - grab).clamp(0.0, (seconds - visible).max(0.0));
            }
        }
    }
    if !resp.is_pointer_button_down_on() {
        ed.view_drag = None;
    }
}

/// `before` with `edge` moved to file time `t`, in whole scene frames when
/// `snap`, else to the sample. The start frame stays where it is.
fn trim_to(
    before: &SoundClip,
    file: &Media,
    fps: f32,
    edge: Edge,
    t: f64,
    snap: bool,
) -> SoundClip {
    let rate = f64::from(file.rate.max(1));
    let per = rate / f64::from(fps.max(1.0));
    let samples = (t * rate).round().max(0.0) as u64;
    match (edge, snap) {
        (Edge::Start, true) => {
            let from = before.trim_start as f64;
            let frames = ((samples as f64 - from) / per).round();
            let snapped = (from + frames * per).round().max(0.0) as u64;
            before.with_trim_start(file, fps, snapped)
        }
        (Edge::Start, false) => before.with_trim_start(file, fps, samples),
        (Edge::End, true) => {
            let end = file.samples.saturating_sub(before.trim_end) as f64;
            let frames = ((samples as f64 - end) / per).round() as i64;
            before.trimmed_end(file, fps, frames)
        }
        (Edge::End, false) => before.with_trim_end(file, fps, file.samples.saturating_sub(samples)),
    }
}

/// One line a pixel column, lowest to highest sample. Zoomed in far, the
/// samples themselves are read; further out, the stored peaks.
#[allow(clippy::too_many_arguments)]
fn draw_wave(
    painter: &egui::Painter,
    sounds: &SoundState,
    key: u64,
    wave: Rect,
    left: f64,
    spp: f64,
    lit0: f32,
    lit1: f32,
) {
    let Some((peaks, gain)) = sounds.peaks(key) else {
        painter.text(
            wave.center(),
            Align2::CENTER_CENTER,
            "Reading sound",
            theme::body(),
            theme::TEXT_SECONDARY,
        );
        return;
    };
    let pcm = sounds
        .samples(key)
        .filter(|(pcm, rate, _)| !pcm.is_empty() && spp * f64::from(*rate) <= 1024.0);
    let mid = wave.center().y;
    let half = wave.height() / 2.0 - 2.0;
    let mut x = wave.left().floor();
    while x < wave.right() {
        let t = left + f64::from(x - wave.left()) * spp;
        let (lo, hi) = match pcm {
            Some((pcm, rate, ch)) => {
                let ch = usize::from(ch.max(1));
                let a = (t * f64::from(rate)) as usize;
                let b = ((t + spp) * f64::from(rate)).ceil() as usize;
                let frames = pcm.len() / ch;
                let (a, b) = (a.min(frames), b.max(a + 1).min(frames));
                pcm[a * ch..b * ch]
                    .iter()
                    .fold((0.0f32, 0.0f32), |(l, h), &v| (l.min(v), h.max(v)))
            }
            None => peaks.range(t, t + spp),
        };
        let (lo, hi) = ((lo * gain).max(-1.0), (hi * gain).min(1.0));
        let colour = if x >= lit0 && x <= lit1 {
            theme::TEXT_PRIMARY
        } else {
            theme::TEXT_DISABLED
        };
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

/// Ticks at each scene frame the file would land on, numbered where there
/// is room. Frame `start` begins where the part that plays begins.
#[allow(clippy::too_many_arguments)]
fn ruler(
    painter: &egui::Painter,
    track: &SoundClip,
    file: &Media,
    fps: f32,
    o: Pos2,
    width: f32,
    left: f64,
    spp: f64,
) {
    let per_frame = 1.0 / f64::from(fps);
    let px_frame = (per_frame / spp) as f32;
    if px_frame < 2.0 {
        return;
    }
    // Number every 1, 5, 10 ... frames, at least 32 px apart.
    let every = [1u32, 5, 10, 25, 50, 100, 250, 500, 1000]
        .into_iter()
        .find(|&n| n as f32 * px_frame >= 32.0)
        .unwrap_or(1000);
    // File time where scene frame 1 would begin.
    let zero = track.offset(file) - f64::from(track.start.max(1) - 1) * per_frame;
    let first = ((left - zero) / per_frame).floor().max(0.0) as u32 + 1;
    let mut f = first;
    loop {
        let x = o.x + ((zero + f64::from(f - 1) * per_frame - left) / spp) as f32;
        if x > o.x + width {
            break;
        }
        let numbered = f.is_multiple_of(every);
        let h = if numbered { 6.0 } else { 3.0 };
        painter.line_segment(
            [Pos2::new(x, o.y + RULER_H - h), Pos2::new(x, o.y + RULER_H)],
            Stroke::new(1.0, theme::LINE_STRONG),
        );
        if numbered {
            painter.text(
                Pos2::new(x + 2.0, o.y + RULER_H - 7.0),
                Align2::LEFT_BOTTOM,
                f.to_string(),
                theme::dense(),
                theme::TEXT_SECONDARY,
            );
        }
        f += 1;
    }
}
