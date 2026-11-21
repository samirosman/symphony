//! Symphony's own small controls, where egui's defaults break the design
//! rules. Unchecked checkboxes and unselected radios were invisible on our
//! surfaces, so these draw a `line.control` edge (3:1 or more, D4.4).

use egui::{self, Pos2, Rect, Response, Sense, Stroke, Vec2};

use crate::theme;

const BOX: f32 = 14.0;

fn row(ui: &mut egui::Ui, label: &str) -> (Response, Rect) {
    let font = theme::body();
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, theme::TEXT_PRIMARY);
    let size = Vec2::new(
        BOX + 8.0 + galley.size().x,
        BOX.max(galley.size().y).max(20.0),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let mark = Rect::from_center_size(
        Pos2::new(rect.left() + BOX / 2.0, rect.center().y),
        Vec2::splat(BOX),
    );
    let text_pos = Pos2::new(mark.right() + 8.0, rect.center().y - galley.size().y / 2.0);
    let colour = if ui.is_enabled() {
        theme::TEXT_PRIMARY
    } else {
        theme::TEXT_DISABLED
    };
    ui.painter().galley(text_pos, galley, colour);
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
            theme::ACCENT_SELECT_FILL,
        );
        let pts = [
            Pos2::new(mark.left() + 3.5, mark.center().y),
            Pos2::new(mark.left() + 6.0, mark.bottom() - 3.5),
            Pos2::new(mark.right() - 3.0, mark.top() + 3.5),
        ];
        p.add(egui::Shape::line(
            pts.to_vec(),
            Stroke::new(1.8, theme::ON_ACCENT),
        ));
    } else {
        p.rect_filled(mark, theme::RADIUS_CONTROL as f32, theme::BG_INPUT);
        let edge = if response.hovered() {
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
    response
}

/// One option of a choice (a radio button). Returns the response; the
/// caller sets the value when it's clicked.
pub fn choice(ui: &mut egui::Ui, selected: bool, label: &str) -> Response {
    let (response, mark) = row(ui, label);
    let p = ui.painter();
    let r = BOX / 2.0;
    if selected {
        p.circle_filled(mark.center(), r, theme::ACCENT_SELECT_FILL);
        p.circle_filled(mark.center(), r * 0.4, theme::ON_ACCENT);
    } else {
        p.circle_filled(mark.center(), r, theme::BG_INPUT);
        let edge = if response.hovered() {
            theme::TEXT_SECONDARY
        } else {
            theme::LINE_CONTROL
        };
        p.circle_stroke(mark.center(), r - 0.5, Stroke::new(1.0, edge));
    }
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
