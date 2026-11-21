//! Symphony's own icon set (docs/DESIGN.md §7). Drawn in code on a 16×16
//! grid with 1.5-unit round strokes, so icons stay sharp at any UI scale.
//! No emoji, no Unicode symbols, no icon fonts (D7.1).

use std::f32::consts::PI;

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Vec2};

use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Select,
    Transform,
    Brush,
    Pencil,
    Eraser,
    Paint,
    Dropper,
    Hand,
    Zoom,
    FirstFrame,
    PrevFrame,
    Play,
    Pause,
    NextFrame,
    LastFrame,
    Eye,
    EyeOff,
}

impl Icon {
    pub const ALL: [Icon; 17] = [
        Icon::Select,
        Icon::Transform,
        Icon::Brush,
        Icon::Pencil,
        Icon::Eraser,
        Icon::Paint,
        Icon::Dropper,
        Icon::Hand,
        Icon::Zoom,
        Icon::FirstFrame,
        Icon::PrevFrame,
        Icon::Play,
        Icon::Pause,
        Icon::NextFrame,
        Icon::LastFrame,
        Icon::Eye,
        Icon::EyeOff,
    ];
}

/// Maps 16×16 grid units into a screen rect.
struct Grid {
    origin: Pos2,
    unit: f32,
    colour: Color32,
}

impl Grid {
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + Vec2::new(x, y) * self.unit
    }
    fn pts(&self, xy: &[(f32, f32)]) -> Vec<Pos2> {
        xy.iter().map(|&(x, y)| self.p(x, y)).collect()
    }
    fn stroke(&self) -> Stroke {
        Stroke::new(1.5 * self.unit, self.colour)
    }
    fn line(&self, painter: &Painter, xy: &[(f32, f32)]) {
        painter.add(Shape::line(self.pts(xy), self.stroke()));
    }
    fn closed(&self, painter: &Painter, xy: &[(f32, f32)]) {
        painter.add(Shape::closed_line(self.pts(xy), self.stroke()));
    }
    fn fill(&self, painter: &Painter, xy: &[(f32, f32)]) {
        painter.add(Shape::convex_polygon(
            self.pts(xy),
            self.colour,
            Stroke::NONE,
        ));
    }
    fn circle(&self, painter: &Painter, x: f32, y: f32, r: f32) {
        painter.circle_stroke(self.p(x, y), r * self.unit, self.stroke());
    }
    fn dot(&self, painter: &Painter, x: f32, y: f32, r: f32) {
        painter.circle_filled(self.p(x, y), r * self.unit, self.colour);
    }
}

fn eye_outline(g: &Grid, painter: &Painter) {
    let n = 12;
    let mut pts = Vec::with_capacity(2 * n + 2);
    for i in 0..=n {
        let t = i as f32 / n as f32;
        pts.push((2.0 + 12.0 * t, 8.0 - 4.2 * (PI * t).sin()));
    }
    for i in (1..n).rev() {
        let t = i as f32 / n as f32;
        pts.push((2.0 + 12.0 * t, 8.0 + 4.2 * (PI * t).sin()));
    }
    g.closed(painter, &pts);
    g.circle(painter, 8.0, 8.0, 2.0);
}

/// Paint `icon` filling `rect` (square) in `colour`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, colour: Color32) {
    let side = rect.width().min(rect.height());
    let g = Grid {
        origin: rect.center() - Vec2::splat(side / 2.0),
        unit: side / 16.0,
        colour,
    };
    let p = painter;
    match icon {
        Icon::Select => g.closed(
            p,
            &[
                (4.0, 2.5),
                (4.0, 13.0),
                (6.8, 10.4),
                (9.0, 14.0),
                (10.8, 13.0),
                (8.7, 9.5),
                (12.5, 9.2),
            ],
        ),
        Icon::Transform => {
            g.closed(p, &[(4.0, 4.0), (12.0, 4.0), (12.0, 12.0), (4.0, 12.0)]);
            for (x, y) in [(4.0, 4.0), (12.0, 4.0), (12.0, 12.0), (4.0, 12.0)] {
                g.fill(
                    p,
                    &[
                        (x - 1.6, y - 1.6),
                        (x + 1.6, y - 1.6),
                        (x + 1.6, y + 1.6),
                        (x - 1.6, y + 1.6),
                    ],
                );
            }
        }
        Icon::Brush => {
            g.line(p, &[(13.5, 2.5), (8.0, 8.0)]);
            g.fill(
                p,
                &[(9.2, 9.2), (6.8, 6.8), (4.6, 8.6), (3.0, 13.0), (7.4, 11.4)],
            );
        }
        Icon::Pencil => {
            g.closed(
                p,
                &[
                    (10.8, 2.6),
                    (13.4, 5.2),
                    (5.2, 13.4),
                    (2.6, 13.4),
                    (2.6, 10.8),
                ],
            );
            g.line(p, &[(9.2, 4.2), (11.8, 6.8)]);
        }
        Icon::Eraser => {
            g.closed(p, &[(9.0, 2.6), (13.4, 7.0), (8.0, 12.4), (3.6, 8.0)]);
            g.line(p, &[(6.3, 5.3), (10.7, 9.7)]);
            g.line(p, &[(6.5, 13.5), (13.5, 13.5)]);
        }
        Icon::Paint => {
            g.closed(p, &[(2.8, 7.6), (7.6, 2.8), (12.4, 7.6), (7.6, 12.4)]);
            g.line(p, &[(2.8, 7.6), (12.4, 7.6)]);
            g.fill(p, &[(13.2, 10.0), (14.4, 12.4), (13.2, 13.6), (12.0, 12.4)]);
        }
        Icon::Dropper => {
            g.line(p, &[(3.0, 13.0), (9.6, 6.4)]);
            g.line(p, &[(8.0, 4.8), (11.2, 8.0)]);
            g.dot(p, 11.6, 4.4, 2.2);
        }
        Icon::Hand => {
            g.line(p, &[(8.0, 2.5), (8.0, 13.5)]);
            g.line(p, &[(2.5, 8.0), (13.5, 8.0)]);
            for pts in [
                [(6.2, 4.3), (8.0, 2.5), (9.8, 4.3)],
                [(6.2, 11.7), (8.0, 13.5), (9.8, 11.7)],
                [(4.3, 6.2), (2.5, 8.0), (4.3, 9.8)],
                [(11.7, 6.2), (13.5, 8.0), (11.7, 9.8)],
            ] {
                g.line(p, &pts);
            }
        }
        Icon::Zoom => {
            g.circle(p, 7.0, 7.0, 4.2);
            g.line(p, &[(10.2, 10.2), (13.6, 13.6)]);
        }
        Icon::FirstFrame => {
            g.line(p, &[(3.5, 3.5), (3.5, 12.5)]);
            g.fill(p, &[(12.5, 3.5), (12.5, 12.5), (5.5, 8.0)]);
        }
        Icon::PrevFrame => g.fill(p, &[(11.0, 3.5), (11.0, 12.5), (4.0, 8.0)]),
        Icon::Play => g.fill(p, &[(4.5, 2.8), (4.5, 13.2), (13.0, 8.0)]),
        Icon::Pause => {
            g.fill(p, &[(4.0, 3.0), (6.8, 3.0), (6.8, 13.0), (4.0, 13.0)]);
            g.fill(p, &[(9.2, 3.0), (12.0, 3.0), (12.0, 13.0), (9.2, 13.0)]);
        }
        Icon::NextFrame => g.fill(p, &[(5.0, 3.5), (5.0, 12.5), (12.0, 8.0)]),
        Icon::LastFrame => {
            g.fill(p, &[(3.5, 3.5), (3.5, 12.5), (10.5, 8.0)]);
            g.line(p, &[(12.5, 3.5), (12.5, 12.5)]);
        }
        Icon::Eye => eye_outline(&g, p),
        Icon::EyeOff => {
            eye_outline(&g, p);
            g.line(p, &[(3.0, 13.0), (13.0, 3.0)]);
        }
    }
}

/// A toolbar icon button (D6.2: 24 px target, 16 px icon). `active` shows
/// the on state (D7.3). Disabled buttons still show their tooltip.
pub fn button(
    ui: &mut egui::Ui,
    icon: Icon,
    active: bool,
    enabled: bool,
    tooltip: &str,
) -> Response {
    sized_button(
        ui,
        icon,
        theme::TOOLBAR_BUTTON,
        16.0,
        active,
        enabled,
        tooltip,
    )
}

/// A Tools-toolbar button (D6.2: 32 px pen target, 20 px icon).
pub fn tool_button(
    ui: &mut egui::Ui,
    icon: Icon,
    active: bool,
    enabled: bool,
    tooltip: &str,
) -> Response {
    sized_button(
        ui,
        icon,
        theme::TOOL_BUTTON,
        theme::TOOL_ICON,
        active,
        enabled,
        tooltip,
    )
}

fn sized_button(
    ui: &mut egui::Ui,
    icon: Icon,
    size: f32,
    icon_size: f32,
    active: bool,
    enabled: bool,
    tooltip: &str,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), sense);
    let painter = ui.painter();
    let (bg, fg) = if !enabled {
        (None, theme::TEXT_DISABLED)
    } else if active {
        (Some(theme::ACCENT_SELECT_FILL), theme::ON_ACCENT)
    } else if response.hovered() {
        (Some(theme::BG_HOVER), theme::TEXT_PRIMARY)
    } else {
        (None, theme::TEXT_PRIMARY)
    };
    if let Some(bg) = bg {
        painter.rect_filled(rect, theme::RADIUS_CONTROL as f32, bg);
    }
    paint(
        painter,
        Rect::from_center_size(rect.center(), Vec2::splat(icon_size)),
        icon,
        fg,
    );
    response.on_hover_text(tooltip)
}
