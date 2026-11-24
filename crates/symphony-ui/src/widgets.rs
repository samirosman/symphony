//! Symphony's own small controls, where egui's defaults break the design
//! rules. Unchecked checkboxes and unselected radios were invisible on our
//! surfaces, so these draw a `line.control` edge (3:1 or more, D4.4).

use egui::{self, Pos2, Rect, Response, Sense, Stroke, Vec2};

use crate::theme;

const BOX: f32 = 14.0;

/// Keep a slider's rail focused after pointer adjustment so its configured
/// step is immediately available from Left and Right.
pub fn precise_slider(ui: &mut egui::Ui, slider: egui::Slider<'_>) -> Response {
    let response = ui.add(slider);
    if response.drag_started() || response.drag_stopped() {
        response.request_focus();
    }
    response
}

/// The commit action in a dialog or workflow.
pub fn primary_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> Response {
    let enabled = enabled && ui.is_enabled();
    let colour = if enabled {
        theme::ON_ACCENT
    } else {
        theme::TEXT_DISABLED
    };
    let button = egui::Button::new(egui::RichText::new(label).color(colour))
        .min_size(Vec2::new(0.0, theme::ROW))
        .fill(if enabled {
            theme::ACCENT_SELECT_FILL
        } else {
            theme::BG_RAISED
        });
    let response = ui.add_enabled(enabled, button);
    theme::focus_ring(ui, &response);
    response
}

pub const FRAME_RATES: [f32; 4] = [12.0, 24.0, 30.0, 48.0];

/// The rates a scene may have, presets or custom.
pub const FRAME_RATE_RANGE: std::ops::RangeInclusive<f32> = 1.0..=120.0;

/// Common film and video rates, and Custom... for any other. An imported
/// scene's custom rate stays visible until the artist selects another.
pub fn frame_rate(ui: &mut egui::Ui, id: &str, fps: &mut f32) {
    let id = egui::Id::unique(id);
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{fps} fps"))
        .show_ui(ui, |ui| {
            for rate in FRAME_RATES {
                ui.selectable_value(fps, rate, format!("{rate:.0} fps"));
            }
            ui.separator();
            if ui.button("Custom...").clicked() {
                ask_custom_frame_rate(ui.ctx(), id, *fps);
            }
        });
    custom_frame_rate(ui.ctx(), id, fps);
}

/// Open the Custom frame rate dialog that belongs to `id`, starting at
/// `current`. [`custom_frame_rate`] shows it.
pub fn ask_custom_frame_rate(ctx: &egui::Context, id: egui::Id, current: f32) {
    ctx.data_mut(|d| d.insert_temp(id.with("custom"), current));
}

/// While the dialog of `id` is open, show it. Set Frame Rate writes the
/// value to `fps` and returns `true`; Cancel, Escape or a click outside
/// close it unchanged. Call it every frame.
pub fn custom_frame_rate(ctx: &egui::Context, id: egui::Id, fps: &mut f32) -> bool {
    let key = id.with("custom");
    let Some(mut value) = ctx.data(|d| d.get_temp::<f32>(key)) else {
        return false;
    };
    let mut done = None;
    let modal = egui::Modal::new(key).show(ctx, |ui| {
        ui.heading("Custom frame rate");
        ui.add_space(8.0);
        let field = ui.add(
            egui::DragValue::new(&mut value)
                .range(FRAME_RATE_RANGE)
                .speed(0.1)
                .max_decimals(3)
                .suffix(" fps"),
        );
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if primary_button(ui, "Set frame rate", true).clicked() || enter {
                done = Some(true);
            }
            if ui.button("Cancel").clicked() {
                done = Some(false);
            }
        });
    });
    if done.is_none() && modal.should_close() {
        done = Some(false);
    }
    match done {
        Some(set) => {
            ctx.data_mut(|d| d.remove::<f32>(key));
            if set {
                *fps = value.clamp(*FRAME_RATE_RANGE.start(), *FRAME_RATE_RANGE.end());
            }
            set
        }
        None => {
            ctx.data_mut(|d| d.insert_temp(key, value));
            false
        }
    }
}

fn row(ui: &mut egui::Ui, label: &str) -> (Response, Rect) {
    let colour = if ui.is_enabled() {
        theme::TEXT_PRIMARY
    } else {
        theme::TEXT_DISABLED
    };
    let font = theme::body();
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, colour);
    let size = Vec2::new(
        BOX + 14.0 + galley.size().x,
        BOX.max(galley.size().y).max(theme::TOOLBAR_BUTTON),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let mark = Rect::from_center_size(
        Pos2::new(rect.left() + 3.0 + BOX / 2.0, rect.center().y),
        Vec2::splat(BOX),
    );
    let text_pos = Pos2::new(mark.right() + 8.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_pos, galley, colour);
    theme::focus_ring(ui, &response);
    (response, mark)
}

/// A checkbox: cel-blue fill with a white check when on, a visible edge when off.
pub fn checkbox(ui: &mut egui::Ui, value: &mut bool, label: &str) -> Response {
    let (mut response, mark) = row(ui, label);
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let p = ui.painter();
    if *value {
        p.rect_filled(
            mark,
            theme::RADIUS_CONTROL as f32,
            if response.enabled() {
                theme::ACCENT_SELECT_FILL
            } else {
                theme::BG_RAISED
            },
        );
        let pts = [
            Pos2::new(mark.left() + 3.5, mark.center().y),
            Pos2::new(mark.left() + 6.0, mark.bottom() - 3.5),
            Pos2::new(mark.right() - 3.0, mark.top() + 3.5),
        ];
        p.add(egui::Shape::line(
            pts.to_vec(),
            Stroke::new(
                1.8,
                if response.enabled() {
                    theme::ON_ACCENT
                } else {
                    theme::TEXT_DISABLED
                },
            ),
        ));
    } else {
        p.rect_filled(mark, theme::RADIUS_CONTROL as f32, theme::BG_INPUT);
        let edge = if !response.enabled() {
            theme::TEXT_DISABLED
        } else if response.hovered() {
            theme::TEXT_SECONDARY
        } else {
            theme::LINE_CONTROL
        };
        p.rect_stroke(
            mark,
            theme::RADIUS_CONTROL as f32,
            Stroke::new(1.0, edge),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, response.enabled(), *value, label)
    });
    response
}

/// One option of a choice (a radio button). Returns the response; the
/// caller sets the value when it's clicked.
pub fn choice(ui: &mut egui::Ui, selected: bool, label: &str) -> Response {
    let (response, mark) = row(ui, label);
    let p = ui.painter();
    let r = BOX / 2.0;
    if selected {
        let (fill, dot) = if response.enabled() {
            (theme::ACCENT_SELECT_FILL, theme::ON_ACCENT)
        } else {
            (theme::BG_RAISED, theme::TEXT_DISABLED)
        };
        p.circle_filled(mark.center(), r, fill);
        p.circle_filled(mark.center(), r * 0.4, dot);
    } else {
        p.circle_filled(mark.center(), r, theme::BG_INPUT);
        let edge = if !response.enabled() {
            theme::TEXT_DISABLED
        } else if response.hovered() {
            theme::TEXT_SECONDARY
        } else {
            theme::LINE_CONTROL
        };
        p.circle_stroke(mark.center(), r - 0.5, Stroke::new(1.0, edge));
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::RadioButton, response.enabled(), selected, label)
    });
    response
}

/// `choice` bound to a value.
pub fn radio_value<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    current: &mut T,
    value: T,
    label: &str,
) -> Response {
    let mut r = choice(ui, *current == value, label);
    if r.clicked() && *current != value {
        *current = value;
        r.mark_changed();
    }
    r
}

/// Paint text cut to `max_width` with an ellipsis (DESIGN D5.5). `pos` is
/// placed by `align`, as in `Painter::text`. Returns the drawn rectangle
/// and whether the text was cut.
#[allow(clippy::too_many_arguments)]
pub fn fit_text(
    ui: &egui::Ui,
    painter: &egui::Painter,
    pos: Pos2,
    align: egui::Align2,
    text: &str,
    font: egui::FontId,
    colour: egui::Color32,
    max_width: f32,
) -> (Rect, bool) {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat::simple(font, colour),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(8.0));
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let rect = align.anchor_size(pos, galley.size());
    let elided = galley.elided;
    painter.galley(rect.min, galley, colour);
    (rect, elided)
}

/// [`fit_text`] plus the full text on hover when it was cut (D5.5). Use
/// this unless the surrounding widget's own tooltip already names it.
#[allow(clippy::too_many_arguments)]
pub fn fitted_text(
    ui: &egui::Ui,
    painter: &egui::Painter,
    pos: Pos2,
    align: egui::Align2,
    text: &str,
    font: egui::FontId,
    colour: egui::Color32,
    max_width: f32,
) -> Rect {
    let (rect, elided) = fit_text(ui, painter, pos, align, text, font, colour, max_width);
    if elided && ui.rect_contains_pointer(rect) {
        let rested = ui.input(|i| i.pointer.time_since_last_movement())
            >= ui.style().interaction.tooltip_delay;
        if rested {
            egui::Tooltip::always_open(
                ui.ctx().clone(),
                ui.layer_id(),
                ui.scope_id().with(("fitted-text", text)),
                egui::PopupAnchor::Pointer,
            )
            .show(|ui| ui.label(text));
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f32(
                    ui.style().interaction.tooltip_delay,
                ));
        }
    }
    rect
}

/// A row (or, `vertical`, a column) of choices. One highlight marks the
/// chosen option and slides to the new one when it changes (Select motion),
/// instead of jumping. Returns true when the choice changed.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    current: &mut T,
    options: &[(T, &str)],
    vertical: bool,
) -> bool {
    let id = ui.scope_id().with(("segmented", id_salt));
    let mut changed = false;
    // Size to the choices: a horizontal layout given the rest of a panel
    // would stretch to fill its height.
    let body = |ui: &mut egui::Ui| {
        if !vertical {
            ui.spacing_mut().item_spacing.x = 2.0;
        }
        // Track the highlight in the control's space so parent scrolling
        // moves the entire control and any in-flight transition together.
        let origin = ui.next_widget_position().to_vec2();
        let highlight = ui.painter().add(egui::Shape::Noop);
        let mut chosen = None;
        for &(value, label) in options {
            let selected = *current == value;
            let galley =
                ui.painter()
                    .layout_no_wrap(label.to_owned(), theme::body(), theme::TEXT_PRIMARY);
            let pad = Vec2::new(8.0, 4.0);
            let size = if vertical {
                Vec2::new(ui.available_width(), galley.size().y + pad.y * 2.0)
            } else {
                galley.size() + pad * 2.0
            };
            let enabled = ui.is_enabled();
            let (rect, response) = ui.allocate_exact_size(
                size,
                if enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            if response.hovered() && !selected && enabled {
                ui.painter()
                    .rect_filled(rect, theme::RADIUS_CONTROL, theme::BG_HOVER);
            }
            if selected {
                chosen = Some(rect);
            }
            let colour = if !enabled {
                theme::TEXT_DISABLED
            } else if selected {
                theme::ON_ACCENT
            } else {
                theme::TEXT_PRIMARY
            };
            let at = if vertical {
                Pos2::new(rect.left() + pad.x, rect.center().y - galley.size().y / 2.0)
            } else {
                rect.center() - galley.size() / 2.0
            };
            ui.painter().galley(at, galley, colour);
            theme::focus_ring(ui, &response);
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::RadioButton, enabled, selected, label)
            });
            if response.clicked() && !selected {
                *current = value;
                changed = true;
            }
        }
        if let Some(rect) = chosen {
            let r = crate::motion::slide(ui.ctx(), id, rect.translate(-origin)).translate(origin);
            let fill = if ui.is_enabled() {
                theme::ACCENT_SELECT_FILL
            } else {
                theme::BG_RAISED
            };
            ui.painter().set(
                highlight,
                egui::Shape::rect_filled(r, theme::RADIUS_CONTROL, fill),
            );
        }
    };
    if vertical {
        ui.vertical(body);
    } else {
        ui.horizontal(body);
    }
    changed
}

/// The middle button always pans (Harmony's "Middle Button Pans View").
/// When a middle press began on `resp`, returns how far the pointer moved
/// this frame (zero on the press and release frames) and a hover-only
/// stand-in for `resp`, so code that reads clicks, drags or the press
/// position sees nothing. Otherwise returns `resp` unchanged and `None`.
pub fn middle_pan(ui: &egui::Ui, resp: Response) -> (Response, Option<Vec2>) {
    let middle = egui::PointerButton::Middle;
    let (down, released, primary, delta) = ui.input(|i| {
        (
            i.pointer.button_down(middle),
            i.pointer.button_released(middle),
            i.pointer.primary_down(),
            i.pointer.delta(),
        )
    });
    let held = down && resp.is_pointer_button_down_on();
    let ended = released && (resp.drag_stopped() || resp.clicked_by(middle));
    if primary || !(held || ended) {
        return (resp, None);
    }
    let inert = ui.interact(resp.rect, resp.id.with("middle pan"), Sense::hover());
    (inert, Some(if held { delta } else { Vec2::ZERO }))
}
