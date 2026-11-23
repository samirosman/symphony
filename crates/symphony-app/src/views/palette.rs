//! The search box (Tab): a field centred on the mouse that shows results as
//! you type. Ranking and the plain-language rules live in `search.rs`. It
//! only fades in and out (the shared Appear and Dismiss motions, opacity
//! only): no growing or sliding.

use eframe::egui::{self, Align2, Id, Pos2, Rect, Vec2};
use symphony_ui::motion;
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
    scroll_offset: f32,
    first_frame: bool,
    /// Closing: fading out, no longer taking input.
    pub closing: bool,
}

impl PaletteState {
    pub fn new(at: Pos2, here: Option<View>) -> Self {
        PaletteState {
            query: String::new(),
            at,
            here,
            selected: 0,
            scroll_offset: 0.0,
            first_frame: true,
            closing: false,
        }
    }
}

fn placeholder(here: Option<View>) -> &'static str {
    match here {
        Some(View::Node | View::NodeLibrary) => "Search nodes and effects…",
        Some(View::Drawing) => "Search drawing tools and actions…",
        Some(View::Multiplane) => "Search camera, depth, and staging…",
        Some(View::Camera) => "Search camera, playback, and views…",
        Some(View::Timeline | View::Xsheet) => "Search frames, layers, and timing…",
        Some(View::Colour) => "Search colours and palettes…",
        Some(View::Function) => "Search keyframes and timing…",
        Some(View::ToolProperties | View::LayerProperties) => {
            "Search tools, properties, and settings…"
        }
        Some(View::Library) => "Search the library and views…",
        None => "Search tools, actions, and settings…",
    }
}

const WIDTH: f32 = 420.0;
const ROWS: usize = 9;
const ROW_H: f32 = 34.0;
const FIELD_H: f32 = 30.0;

pub enum Outcome {
    Open,
    /// Fading out; keep calling until `Closed`.
    Closing,
    Closed,
    Chosen(Command, String),
}

fn presence_id() -> Id {
    Id::unique("symphony-search-presence")
}

fn reveal_offset(selected: usize, offset: f32, height: f32) -> f32 {
    let row_top = selected as f32 * ROW_H;
    let row_bottom = row_top + ROW_H;
    if row_top < offset {
        row_top
    } else if row_bottom > offset + height {
        (row_bottom - height).max(0.0)
    } else {
        offset
    }
}

/// Draw the palette. `entries` is everything searchable; `uses` how often
/// each key was chosen.
pub fn ui(
    ctx: &egui::Context,
    st: &mut PaletteState,
    entries: &[Entry],
    uses: &dyn Fn(&str) -> u32,
) -> Outcome {
    let presence_id = presence_id();
    if st.first_frame {
        // Every opening fades in from nothing.
        motion::start(ctx, presence_id, 0.0);
    }
    let shown = motion::presence(ctx, presence_id, !st.closing);
    if st.closing && shown <= 0.0 {
        return Outcome::Closed;
    }

    // Results only once something is typed: phrase commands ("go to
    // frame 12") first, then ranked entries.
    let typed = !st.query.trim().is_empty();
    let mut results: Vec<Entry> = Vec::new();
    // How many results are strong: phrase commands, and matches scoring at
    // least half the best. The box is sized to these; weaker matches are
    // still there below, by scrolling or the arrow keys.
    let mut strong = 0;
    if typed {
        results.extend(search::phrase(&st.query));
        let hits = search::rank(entries, &st.query, st.here, uses);
        let best = hits.first().map_or(0.0, |h| h.score);
        strong = results.len() + hits.iter().take_while(|h| h.score >= best * 0.5).count();
        results.extend(hits.iter().map(|h| entries[h.index].clone()));
    }
    st.selected = st.selected.min(results.len().saturating_sub(1));

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
    // Closing keeps drawing this frame too: returning early here left one
    // blank frame before the fade-out, which looked like fading twice.
    if !st.closing && (esc || tab) {
        st.closing = true;
    }
    if up {
        st.selected = st.selected.saturating_sub(1);
    }
    if down && st.selected + 1 < results.len() {
        st.selected += 1;
    }

    // The results area grows to fit (Expand), from nothing while empty.
    let screen = ctx.content_rect();
    let max_list_h = (screen.height() - FIELD_H - 36.0).max(ROW_H);
    let rows = if typed { strong.clamp(1, ROWS) } else { 0 };
    let target_h = (rows as f32 * ROW_H).min(max_list_h);
    // Results show at their full height at once; only opacity animates.
    let list_h = target_h;

    // Centred on the mouse horizontally, just below it, kept in the window;
    // it simply fades in and out (Appear and Dismiss, opacity only).
    let opacity = shown;
    let box_w = WIDTH + 16.0;
    let full_h = FIELD_H + 20.0 + target_h;
    let x =
        (st.at.x - box_w / 2.0).clamp(screen.left() + 8.0, (screen.right() - box_w - 8.0).max(8.0));
    let y = (st.at.y + 12.0).clamp(
        screen.top() + 8.0,
        (screen.bottom() - full_h - 8.0).max(8.0),
    );
    let pos = Pos2::new(x, y);

    let mut chosen: Option<usize> = None;
    let area = egui::Area::new(Id::unique("symphony-search"))
        // Above everything else, floating windows and overlays included.
        .order(egui::Order::Tooltip)
        .fixed_pos(pos)
        .fade_in(false)
        .interactable(!st.closing)
        .show(ctx, |ui| {
            ui.multiply_opacity(opacity);
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
                            .hint_text(placeholder(st.here))
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
                    if list_h < 0.5 {
                        return;
                    }
                    ui.add_space(4.0);
                    if !typed {
                        ui.add_space(list_h);
                        return;
                    }
                    if results.is_empty() {
                        ui.label(
                            egui::RichText::new(
                                "Nothing matches. Try other words, like \"rubber\" or \"fill\".",
                            )
                            .font(theme::dense())
                            .color(theme::TEXT_SECONDARY),
                        );
                        return;
                    }
                    let mut scroll = egui::ScrollArea::vertical()
                        .id_salt("search-results")
                        .max_height(list_h)
                        .auto_shrink([false, false]);
                    if edit.changed() {
                        scroll = scroll.vertical_scroll_offset(0.0);
                    } else if up || down {
                        scroll = scroll.vertical_scroll_offset(reveal_offset(
                            st.selected,
                            st.scroll_offset,
                            list_h,
                        ));
                    }
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let output = scroll.show_rows(ui, ROW_H - 2.0, results.len(), |ui, range| {
                        for i in range {
                            let e = &results[i];
                            let sel = i == st.selected;
                            let (rect, row) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), ROW_H - 2.0),
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
                    st.scroll_offset = output.state.clamped_offset().y;
                    let view = output.inner_rect;
                    let fade = 9.0_f32.min(view.height() * 0.5);
                    if st.scroll_offset > 0.5 {
                        let strip = Rect::from_min_size(view.min, Vec2::new(view.width(), fade));
                        theme::fade_strip(ui.painter(), strip, theme::BG_RAISED, true);
                    }
                    if st.scroll_offset + view.height() + 0.5 < output.content_size.y {
                        let min = Pos2::new(view.left(), view.bottom() - fade);
                        let strip = Rect::from_min_max(min, view.max);
                        theme::fade_strip(ui.painter(), strip, theme::BG_RAISED, false);
                    }
                });
        });
    if st.closing {
        return Outcome::Closing;
    }
    if enter && !results.is_empty() {
        chosen = Some(st.selected);
    }
    if let Some(i) = chosen {
        let e = &results[i];
        return Outcome::Chosen(e.command.clone(), e.key.clone());
    }
    // A click anywhere else closes it.
    let clicked_outside = ctx.input(|i| i.pointer.any_pressed())
        && ctx
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| !area.response.rect.contains(p));
    if clicked_outside {
        st.closing = true;
        return Outcome::Closing;
    }
    Outcome::Open
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    #[test]
    fn search_scrolls_and_reaches_results_beyond_the_first_page() {
        let mut state = PaletteState::new(Pos2::new(390.0, 54.0), Some(View::Timeline));
        state.query = "Drawing".into();
        let entries: Vec<_> = (1..=32)
            .map(|index| {
                Entry::new(
                    &format!("Drawing entry {index:02}"),
                    search::Kind::Action,
                    Command::GoToFrame(index),
                    &format!("drawing.{index}"),
                )
            })
            .collect();
        let mut harness = Harness::builder()
            .with_size(Vec2::new(800.0, 620.0))
            .with_context_setup(|ctx| {
                theme::apply(ctx);
                theme::set_motion(ctx, false);
            })
            .build_ui_state(
                |ui, (state, entries)| {
                    super::ui(ui.ctx(), state, entries, &|_| 0);
                },
                (state, entries),
            );
        harness.run_steps(3);
        for _ in 0..13 {
            harness.input_mut().events.push(egui::Event::Key {
                key: egui::Key::ArrowDown,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            harness.run_steps(1);
            harness.input_mut().events.push(egui::Event::Key {
                key: egui::Key::ArrowDown,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            harness.run_steps(1);
        }
        assert_eq!(harness.state().0.selected, 13);
        let before = harness.state().0.scroll_offset;
        assert!(before > 0.0);
        harness.input_mut().events.extend([
            egui::Event::PointerMoved(Pos2::new(380.0, 205.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, -120.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        harness.run_steps(5);
        assert!(harness.state().0.scroll_offset > before);
    }
}
