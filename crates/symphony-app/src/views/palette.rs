//! The search box (Tab): opens next to the mouse and finds anything in the
//! app. Ranking and the plain-language rules live in `search.rs`.

use eframe::egui::{self, Align2, Pos2, Vec2};
use symphony_ui::{theme, widgets};

use crate::search::{self, Command, Entry};
use crate::workspace::View;

pub struct PaletteState {
    pub query: String,
    /// Where it opened (the mouse), in screen points.
    pub at: Pos2,
    /// The view the mouse was over, for ranking.
    pub here: Option<View>,
    selected: usize,
    first_frame: bool,
    /// When it opened, and when it started closing (for the animation).
    opened_at: Option<f64>,
    pub closing_at: Option<f64>,
}

impl PaletteState {
    pub fn new(at: Pos2, here: Option<View>) -> Self {
        PaletteState {
            query: String::new(),
            at,
            here,
            selected: 0,
            first_frame: true,
            opened_at: None,
            closing_at: None,
        }
    }
}

const WIDTH: f32 = 420.0;
const ROWS: usize = 9;

pub enum Outcome {
    Open,
    /// Fading out; keep calling until `Closed`.
    Closing,
    Closed,
    Chosen(Command, String),
}

/// Open and close animation length, seconds.
const ANIMATE: f64 = 0.14;

/// Draw the palette. `entries` is everything searchable; `uses` how often
/// each key was chosen.
pub fn ui(
    ctx: &egui::Context,
    st: &mut PaletteState,
    entries: &[Entry],
    uses: &dyn Fn(&str) -> u32,
) -> Outcome {
    // Phrase commands ("go to frame 12") first, then ranked entries.
    let phrases = search::phrase(&st.query);
    let hits = search::rank(entries, &st.query, st.here, uses);
    let mut results: Vec<&Entry> = phrases.iter().collect();
    results.extend(hits.iter().map(|h| &entries[h.index]));
    results.truncate(ROWS);
    st.selected = st.selected.min(results.len().saturating_sub(1));

    let now = ctx.input(|i| i.time);
    let opened = *st.opened_at.get_or_insert(now);
    let animated = ctx.global_style().animation_time > 0.0;
    // Fade and grow in; fade and shrink out (instant when animations are off).
    let progress = |since: f64| {
        if animated {
            ((now - since) / ANIMATE).clamp(0.0, 1.0) as f32
        } else {
            1.0
        }
    };
    let ease = |t: f32| 1.0 - (1.0 - t).powi(3);
    let shown = match st.closing_at {
        Some(c) => 1.0 - ease(progress(c)),
        None => ease(progress(opened)),
    };
    if st.closing_at.is_some() && shown <= 0.0 {
        return Outcome::Closed;
    }
    if shown < 1.0 {
        ctx.request_repaint();
    }
    let closing = st.closing_at.is_some();
    // Tab toggles it closed again; Esc closes it too.
    let (up, down, enter, esc, tab) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
            !st.first_frame && i.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
        )
    });
    if !closing && (esc || tab) {
        st.closing_at = Some(now);
        return Outcome::Closing;
    }
    if up {
        st.selected = st.selected.saturating_sub(1);
    }
    if down && st.selected + 1 < results.len() {
        st.selected += 1;
    }

    // Centred on the mouse horizontally, just below it, kept in the window.
    let screen = ctx.content_rect();
    let height = 52.0 + results.len().max(1) as f32 * 34.0;
    let box_w = WIDTH + 16.0;
    let pos = Pos2::new(
        (st.at.x - box_w / 2.0).clamp(screen.left() + 8.0, (screen.right() - box_w - 8.0).max(8.0)),
        (st.at.y + 12.0).clamp(
            screen.top() + 8.0,
            (screen.bottom() - height - 8.0).max(8.0),
        ),
    );
    // Grow from slightly smaller towards the mouse, and fade.
    let scale = 0.96 + 0.04 * shown;
    let pos = Pos2::new(
        st.at.x + (pos.x - st.at.x) * scale,
        st.at.y + (pos.y - st.at.y) * scale,
    );
    let mut chosen: Option<usize> = None;
    let area = egui::Area::new(egui::Id::unique("symphony-search"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .fade_in(false)
        .interactable(!closing)
        .show(ctx, |ui| {
            ui.multiply_opacity(shown);
            egui::Frame::new()
                .fill(theme::BG_RAISED)
                .stroke(egui::Stroke::new(1.0, theme::LINE_STRONG))
                .corner_radius(theme::RADIUS_FLOATING)
                .shadow(ui.visuals().popup_shadow)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut st.query)
                            .hint_text("Search tools, actions, settings… or try \"go to frame 12\"")
                            .desired_width(f32::INFINITY)
                            .font(theme::body()),
                    );
                    if st.first_frame {
                        edit.request_focus();
                        st.first_frame = false;
                    }
                    if edit.changed() {
                        st.selected = 0;
                    }
                    ui.add_space(4.0);
                    if results.is_empty() {
                        ui.label(
                            egui::RichText::new(
                                "Nothing matches. Try other words, like \"rubber\" or \"fill\".",
                            )
                            .font(theme::dense())
                            .color(theme::TEXT_SECONDARY),
                        );
                    }
                    for (i, e) in results.iter().enumerate() {
                        let sel = i == st.selected;
                        let (rect, row) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 32.0),
                            egui::Sense::click(),
                        );
                        let fill = if sel {
                            theme::ACCENT_SELECT_FILL
                        } else if row.hovered() {
                            theme::BG_HOVER
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        let p = ui.painter();
                        p.rect_filled(rect, theme::RADIUS_CONTROL as f32, fill);
                        let fg = if sel {
                            theme::ON_ACCENT
                        } else {
                            theme::TEXT_PRIMARY
                        };
                        let fg2 = if sel {
                            theme::ON_ACCENT
                        } else {
                            theme::TEXT_SECONDARY
                        };
                        let right = rect.right() - 8.0;
                        let shortcut_w = if e.shortcut.is_empty() { 0.0 } else { 90.0 };
                        widgets::fitted_text(
                            ui,
                            p,
                            Pos2::new(rect.left() + 10.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            &e.title,
                            theme::body(),
                            fg,
                            rect.width() - 90.0 - shortcut_w,
                        );
                        p.text(
                            Pos2::new(right - shortcut_w, rect.center().y),
                            Align2::RIGHT_CENTER,
                            e.kind.label(),
                            theme::dense(),
                            fg2,
                        );
                        if !e.shortcut.is_empty() {
                            p.text(
                                Pos2::new(right, rect.center().y),
                                Align2::RIGHT_CENTER,
                                &e.shortcut,
                                theme::dense(),
                                fg2,
                            );
                        }
                        if row.clicked() {
                            chosen = Some(i);
                        }
                        if row.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) {
                            st.selected = i;
                        }
                    }
                });
        });
    if closing {
        return Outcome::Closing;
    }
    if enter && !results.is_empty() {
        chosen = Some(st.selected);
    }
    if let Some(i) = chosen {
        let e = results[i];
        return Outcome::Chosen(e.command.clone(), e.key.clone());
    }
    // A click anywhere else closes it.
    let clicked_outside = ctx.input(|i| i.pointer.any_pressed())
        && ctx
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| !area.response.rect.contains(p));
    if clicked_outside {
        st.closing_at = Some(now);
        return Outcome::Closing;
    }
    Outcome::Open
}
