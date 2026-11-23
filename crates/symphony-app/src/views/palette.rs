//! The search box (Tab): a field centred on the mouse that shows results as
//! you type. Ranking and the plain-language rules live in `search.rs`. It
//! only fades in and out (the shared Appear and Dismiss motions, opacity
//! only): no growing or sliding.

use eframe::egui::{self, Align2, Id, Pos2, Rect, Vec2};
use symphony_ui::motion;
use symphony_ui::{theme, widgets};

use crate::keymap::{self, Action, Profile};
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
    /// Waiting for a new shortcut for this action (from the right-click
    /// menu), and the result's title for the prompt.
    recording: Option<(Action, String)>,
    /// A short note after a menu choice ("Alt+K now runs Mirror view").
    note: Option<String>,
}

/// What a result's right-click menu asked for.
enum Pick {
    Run,
    Record(Action),
    Unbind(Action),
    Reset(Action),
    Copy,
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
            recording: None,
            note: None,
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
    profile: Profile,
) -> Outcome {
    // Recording a new shortcut: the next key press (with its modifiers)
    // becomes the shortcut, and it doesn't type or close the box. Esc
    // cancels.
    if let Some((action, title)) = st.recording.clone() {
        let pressed = ctx.input_mut(|i| {
            let found = i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            });
            i.events
                .retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)));
            found
        });
        if let Some((key, mods)) = pressed {
            st.recording = None;
            if key == egui::Key::Escape {
                st.note = Some("Shortcut unchanged".into());
            } else {
                let taken = keymap::remap(profile, action, key, mods);
                let label = keymap::label(profile, action);
                let mut note = format!("{label} now runs {title}");
                if !taken.is_empty() {
                    let names: Vec<&str> = taken.iter().map(|a| a.name()).collect();
                    note.push_str(&format!(" (taken from {})", names.join(", ")));
                }
                st.note = Some(note);
            }
        }
    }
    if st.first_frame {
        ctx.move_to_top(egui::LayerId::new(
            egui::Order::Foreground,
            Id::unique("symphony-search"),
        ));
    }
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
    let mut pick: Option<(usize, Pick)> = None;
    let area = egui::Area::new(Id::unique("symphony-search"))
        // Above panels and floating windows (Foreground, raised when it
        // opens), with its own right-click menus able to open above it.
        .order(egui::Order::Foreground)
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
                        st.note = None;
                    }
                    if let Some((_, title)) = &st.recording {
                        ui.label(
                            egui::RichText::new(format!(
                                "Press the new shortcut for {title}. Esc cancels."
                            ))
                            .font(theme::dense())
                            .color(theme::ACCENT_SELECT),
                        );
                    } else if let Some(note) = &st.note {
                        ui.label(
                            egui::RichText::new(note)
                                .font(theme::dense())
                                .color(theme::TEXT_SECONDARY),
                        );
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
                            row.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::Role::Button, true, &e.title)
                            });
                            // Right-click: run it, change or remove its
                            // shortcut, or copy its name.
                            row.context_menu(|ui| {
                                if ui.button("Run").clicked() {
                                    pick = Some((i, Pick::Run));
                                }
                                if let Command::Action(a) = e.command {
                                    ui.separator();
                                    let label = if e.shortcut.is_empty() {
                                        "Add shortcut…"
                                    } else {
                                        "Change shortcut…"
                                    };
                                    if ui.button(label).clicked() {
                                        pick = Some((i, Pick::Record(a)));
                                    }
                                    if !e.shortcut.is_empty()
                                        && ui.button("Remove shortcut").clicked()
                                    {
                                        pick = Some((i, Pick::Unbind(a)));
                                    }
                                    if keymap::is_remapped(profile, a)
                                        && ui
                                            .button(format!("Reset to {} default", profile.title()))
                                            .clicked()
                                    {
                                        pick = Some((i, Pick::Reset(a)));
                                    }
                                }
                                ui.separator();
                                if ui.button("Copy name").clicked() {
                                    pick = Some((i, Pick::Copy));
                                }
                            });
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
    if let Some((i, p)) = pick {
        match p {
            Pick::Run => chosen = Some(i),
            Pick::Record(a) => {
                st.recording = Some((a, results[i].title.clone()));
                st.note = None;
            }
            Pick::Unbind(a) => {
                keymap::unbind(profile, a);
                st.note = Some(format!("{} has no shortcut now", results[i].title));
            }
            Pick::Reset(a) => {
                keymap::reset(profile, a);
                let label = keymap::label(profile, a);
                st.note = Some(if label.is_empty() {
                    format!("{} is back to no shortcut", results[i].title)
                } else {
                    format!("{} is back to {label}", results[i].title)
                });
            }
            Pick::Copy => ctx.copy_text(results[i].title.clone()),
        }
    }
    // Enter while recording is the new shortcut, not "run".
    let enter = enter && st.recording.is_none();
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
    fn right_click_a_result_to_give_it_a_shortcut() {
        use egui_kittest::kittest::Queryable;
        let profile = Profile::Symphony;
        keymap::reset(profile, Action::MirrorView);
        let entries = vec![Entry::new(
            "Mirror view",
            search::Kind::Action,
            Command::Action(Action::MirrorView),
            "view.mirror",
        )];
        let mut state = PaletteState::new(Pos2::new(400.0, 40.0), None);
        state.query = "mirror".into();
        let mut harness = Harness::builder()
            .with_size(Vec2::new(800.0, 400.0))
            .with_context_setup(|ctx| {
                theme::apply(ctx);
                theme::set_motion(ctx, false);
            })
            .build_ui_state(
                |ui, (state, entries)| {
                    super::ui(ui.ctx(), state, entries, &|_| 0, Profile::Symphony);
                },
                (state, entries),
            );
        harness.run_steps(3);
        harness.get_by_label("Mirror view").click_secondary();
        harness.run_steps(2);
        harness.get_by_label("Add shortcut…").click();
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::ALT, egui::Key::K);
        harness.run_steps(2);
        let b = keymap::binding(profile, Action::MirrorView).expect("now bound");
        assert_eq!(b.key, egui::Key::K);
        assert!(b.mods.alt);
        assert!(keymap::is_remapped(profile, Action::MirrorView));
        keymap::reset(profile, Action::MirrorView);
        assert!(keymap::binding(profile, Action::MirrorView).is_none());
    }

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
                    super::ui(ui.ctx(), state, entries, &|_| 0, Profile::Harmony);
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
