//! The search box (Tab): pops up as just a field centred on the mouse, then
//! grows to show results as you type. Ranking and the plain-language rules
//! live in `search.rs`; its motion is the app's shared vocabulary
//! (`symphony_ui::motion`): Appear and Dismiss for opening and closing,
//! Expand for the results growing.

use eframe::egui::{self, Align2, Id, Pos2, Rect, Vec2};
use symphony_ui::motion::{self, Motion};
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

fn ids() -> (Id, Id) {
    (
        Id::unique("symphony-search-presence"),
        Id::unique("symphony-search-height"),
    )
}

/// Draw the palette. `entries` is everything searchable; `uses` how often
/// each key was chosen.
pub fn ui(
    ctx: &egui::Context,
    st: &mut PaletteState,
    entries: &[Entry],
    uses: &dyn Fn(&str) -> u32,
) -> Outcome {
    let (presence_id, height_id) = ids();
    if st.first_frame {
        // Every opening starts from nothing, with no results showing.
        motion::start(ctx, presence_id, 0.0);
        motion::start(ctx, height_id, 0.0);
    }
    let shown = motion::presence(ctx, presence_id, !st.closing);
    if st.closing && shown <= 0.0 {
        return Outcome::Closed;
    }

    // Results only once something is typed: phrase commands ("go to
    // frame 12") first, then ranked entries.
    let typed = !st.query.trim().is_empty();
    let mut results: Vec<Entry> = Vec::new();
    if typed {
        results.extend(search::phrase(&st.query));
        let hits = search::rank(entries, &st.query, st.here, uses);
        results.extend(hits.iter().map(|h| entries[h.index].clone()));
        results.truncate(ROWS);
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
    if !st.closing && (esc || tab) {
        st.closing = true;
        return Outcome::Closing;
    }
    if up {
        st.selected = st.selected.saturating_sub(1);
    }
    if down && st.selected + 1 < results.len() {
        st.selected += 1;
    }

    // The results area grows to fit (Expand), from nothing while empty.
    let rows = if typed { results.len().max(1) } else { 0 };
    let list_h = motion::value(ctx, height_id, rows as f32 * ROW_H, Motion::Expand);

    // Centred on the mouse horizontally, just below it, kept in the window;
    // it appears with the shared Appear look (fade, grow, rise).
    let look = motion::look(shown);
    let screen = ctx.content_rect();
    let box_w = WIDTH + 16.0;
    let full_h = FIELD_H + 16.0 + rows as f32 * ROW_H;
    let x =
        (st.at.x - box_w / 2.0).clamp(screen.left() + 8.0, (screen.right() - box_w - 8.0).max(8.0));
    let y = (st.at.y + 12.0).clamp(
        screen.top() + 8.0,
        (screen.bottom() - full_h - 8.0).max(8.0),
    );
    let pos = Pos2::new(st.at.x + (x - st.at.x) * look.scale, y + look.drop);

    let mut chosen: Option<usize> = None;
    let area = egui::Area::new(Id::unique("symphony-search"))
        // Above everything else, floating windows and overlays included.
        .order(egui::Order::Tooltip)
        .fixed_pos(pos)
        .fade_in(false)
        .interactable(!st.closing)
        .show(ctx, |ui| {
            ui.multiply_opacity(look.opacity);
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
                    // The list, clipped to its animated height.
                    ui.add_space(4.0);
                    let (area_rect, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), list_h),
                        egui::Sense::hover(),
                    );
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_size(
                                area_rect.min,
                                Vec2::new(area_rect.width(), rows as f32 * ROW_H),
                            ))
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    child.set_clip_rect(area_rect.intersect(child.clip_rect()));
                    let ui = &mut child;
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
