//! Symphony's own icon set (docs/DESIGN.md §7). Drawn in code on a 16×16
//! grid with 1.5-unit round strokes, so icons stay sharp at any UI scale.
//! No emoji, no Unicode symbols, no icon fonts (D7.1).

use std::f32::consts::PI;

use egui::{self, Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Vec2};

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
    Onion,
    Fit,
    RotateLeft,
    RotateRight,
    Plus,
    Trash,
    ZoomOut,
    Forbidden,
    Minus,
    CloseGap,
}

impl Icon {
    pub const ALL: [Icon; 27] = [
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
        Icon::Onion,
        Icon::Fit,
        Icon::RotateLeft,
        Icon::RotateRight,
        Icon::Plus,
        Icon::Trash,
        Icon::ZoomOut,
        Icon::Forbidden,
        Icon::Minus,
        Icon::CloseGap,
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
            // A tapered handle, a short ferrule and a loaded tuft curling left.
            g.fill(
                p,
                &[
                    (7.1, 7.6),
                    (12.9, 1.9),
                    (14.0, 1.6),
                    (14.2, 2.8),
                    (8.8, 9.2),
                ],
            );
            g.line(p, &[(6.8, 7.8), (8.4, 9.4)]);
            g.fill(
                p,
                &[
                    (6.0, 8.7),
                    (7.7, 10.2),
                    (7.2, 12.1),
                    (5.8, 13.4),
                    (3.8, 14.0),
                    (1.4, 13.5),
                    (3.4, 12.0),
                    (3.9, 10.2),
                    (4.7, 9.1),
                ],
            );
        }
        Icon::Pencil => {
            g.closed(
                p,
                &[
                    (2.2, 13.8),
                    (3.0, 10.1),
                    (10.8, 2.3),
                    (12.0, 2.0),
                    (14.0, 4.0),
                    (13.7, 5.2),
                    (5.9, 13.0),
                ],
            );
            g.line(p, &[(9.4, 3.7), (12.3, 6.6)]);
            g.line(p, &[(3.2, 10.2), (5.8, 12.8)]);
        }
        Icon::Eraser => {
            g.closed(
                p,
                &[
                    (2.0, 9.4),
                    (9.3, 2.1),
                    (14.0, 6.8),
                    (7.4, 13.4),
                    (5.9, 13.4),
                ],
            );
            g.fill(p, &[(6.3, 5.1), (9.3, 2.1), (14.0, 6.8), (11.0, 9.8)]);
            g.line(p, &[(7.2, 13.5), (14.0, 13.5)]);
        }
        Icon::Paint => {
            g.closed(p, &[(1.6, 8.3), (7.7, 2.2), (13.0, 7.5), (6.7, 13.6)]);
            g.fill(p, &[(2.1, 8.3), (12.0, 8.3), (6.7, 13.6)]);
            g.line(p, &[(4.5, 1.6), (7.4, 5.9)]);
            g.fill(
                p,
                &[
                    (13.4, 9.4),
                    (14.9, 12.1),
                    (14.8, 13.2),
                    (13.7, 13.9),
                    (12.5, 13.6),
                    (12.1, 12.4),
                ],
            );
        }
        Icon::Dropper => {
            g.closed(
                p,
                &[
                    (2.0, 14.0),
                    (2.8, 10.5),
                    (8.8, 4.5),
                    (11.5, 7.2),
                    (5.5, 13.2),
                ],
            );
            g.line(p, &[(7.5, 3.8), (12.1, 8.4)]);
            g.line(p, &[(10.0, 5.8), (12.6, 3.2)]);
            g.dot(p, 12.2, 3.8, 2.3);
        }
        Icon::Hand => {
            g.fill(
                p,
                &[
                    (4.6, 7.7),
                    (13.7, 7.5),
                    (13.5, 10.7),
                    (12.0, 13.7),
                    (7.2, 14.0),
                    (4.6, 11.4),
                ],
            );
            // Rounded fingertips leave open channels above the knuckles.
            for (base, tip) in [
                ((5.7, 8.6), (5.0, 4.2)),
                ((8.0, 8.4), (7.7, 2.2)),
                ((10.4, 8.4), (10.5, 3.0)),
                ((12.7, 8.8), (13.1, 5.1)),
                ((5.8, 10.9), (2.5, 7.7)),
            ] {
                let stroke = Stroke::new(1.8 * g.unit, colour);
                p.line_segment([g.p(base.0, base.1), g.p(tip.0, tip.1)], stroke);
                g.dot(p, tip.0, tip.1, 0.9);
            }
        }
        Icon::Zoom => {
            g.circle(p, 7.0, 7.0, 4.2);
            g.line(p, &[(10.2, 10.2), (13.6, 13.6)]);
            g.line(p, &[(5.0, 7.0), (9.0, 7.0)]);
            g.line(p, &[(7.0, 5.0), (7.0, 9.0)]);
        }
        Icon::ZoomOut => {
            g.circle(p, 7.0, 7.0, 4.2);
            g.line(p, &[(10.2, 10.2), (13.6, 13.6)]);
            g.line(p, &[(5.0, 7.0), (9.0, 7.0)]);
        }
        Icon::Forbidden => {
            g.circle(p, 8.0, 8.0, 5.5);
            g.line(p, &[(4.1, 11.9), (11.9, 4.1)]);
        }
        Icon::FirstFrame => {
            g.line(p, &[(3.5, 3.5), (3.5, 12.5)]);
            g.fill(p, &[(12.5, 3.5), (12.5, 12.5), (5.5, 8.0)]);
        }
        Icon::PrevFrame => g.line(p, &[(10.5, 3.2), (5.6, 8.0), (10.5, 12.8)]),
        Icon::Play => g.fill(p, &[(4.5, 2.8), (4.5, 13.2), (13.0, 8.0)]),
        Icon::Pause => {
            g.fill(p, &[(4.0, 3.0), (6.8, 3.0), (6.8, 13.0), (4.0, 13.0)]);
            g.fill(p, &[(9.2, 3.0), (12.0, 3.0), (12.0, 13.0), (9.2, 13.0)]);
        }
        Icon::NextFrame => g.line(p, &[(5.5, 3.2), (10.4, 8.0), (5.5, 12.8)]),
        Icon::LastFrame => {
            g.fill(p, &[(3.5, 3.5), (3.5, 12.5), (10.5, 8.0)]);
            g.line(p, &[(12.5, 3.5), (12.5, 12.5)]);
        }
        Icon::Eye => eye_outline(&g, p),
        Icon::EyeOff => {
            eye_outline(&g, p);
            g.line(p, &[(3.0, 13.0), (13.0, 3.0)]);
        }
        // Three overlapping sheets: onion skin.
        Icon::Onion => {
            g.closed(p, &[(2.2, 2.2), (10.4, 2.2), (10.4, 10.4), (2.2, 10.4)]);
            g.line(p, &[(12.6, 4.8), (12.6, 12.6), (4.8, 12.6)]);
            g.line(p, &[(14.8, 7.2), (14.8, 14.8), (7.2, 14.8)]);
        }
        Icon::Fit => {
            for (x, y, dx, dy) in [
                (2.5, 2.5, 1.0, 1.0),
                (13.5, 2.5, -1.0, 1.0),
                (2.5, 13.5, 1.0, -1.0),
                (13.5, 13.5, -1.0, -1.0),
            ] {
                g.line(p, &[(x, y + 3.5 * dy), (x, y), (x + 3.5 * dx, y)]);
            }
            g.closed(p, &[(6.0, 6.0), (10.0, 6.0), (10.0, 10.0), (6.0, 10.0)]);
        }
        Icon::RotateLeft | Icon::RotateRight => {
            let flip = if icon == Icon::RotateLeft { -1.0 } else { 1.0 };
            let arc: Vec<(f32, f32)> = (0..=10)
                .map(|i| {
                    let a = (-150.0 + 240.0 * i as f32 / 10.0_f32).to_radians();
                    (8.0 + flip * 5.0 * a.cos(), 8.5 + 5.0 * a.sin())
                })
                .collect();
            g.line(p, &arc);
            let tip = arc[0];
            g.line(
                p,
                &[
                    (tip.0 + flip * 2.8, tip.1 - 0.2),
                    tip,
                    (tip.0 + flip * 0.6, tip.1 + 2.8),
                ],
            );
        }
        Icon::Minus => g.line(p, &[(3.0, 8.0), (13.0, 8.0)]),
        // Two line ends with a dotted bridge between them.
        Icon::CloseGap => {
            g.line(p, &[(1.5, 12.0), (3.0, 9.5), (5.0, 7.0)]);
            g.line(p, &[(11.0, 7.0), (13.2, 5.0), (14.5, 2.8)]);
            g.dot(p, 5.0, 7.0, 1.35);
            g.dot(p, 11.0, 7.0, 1.35);
            g.dot(p, 8.0, 7.0, 0.65);
        }
        Icon::Plus => {
            g.line(p, &[(8.0, 3.0), (8.0, 13.0)]);
            g.line(p, &[(3.0, 8.0), (13.0, 8.0)]);
        }
        Icon::Trash => {
            g.line(p, &[(2.5, 4.5), (13.5, 4.5)]);
            g.line(p, &[(6.0, 4.5), (6.5, 2.5), (9.5, 2.5), (10.0, 4.5)]);
            g.closed(p, &[(4.0, 4.5), (12.0, 4.5), (11.2, 13.5), (4.8, 13.5)]);
            g.line(p, &[(6.8, 7.0), (6.8, 11.0)]);
            g.line(p, &[(9.2, 7.0), (9.2, 11.0)]);
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
    let transition = |key, on| {
        if ui.style().animation_time > 0.0 {
            ui.ctx().animate_bool_responsive(response.id.with(key), on)
        } else {
            f32::from(on)
        }
    };
    let hover = transition("hover", enabled && response.hovered());
    let selected = transition("selected", enabled && active);
    let bg = egui::Color32::TRANSPARENT
        .lerp_to_gamma(theme::BG_HOVER, hover)
        .lerp_to_gamma(theme::ACCENT_SELECT_FILL, selected);
    painter.rect_filled(rect, theme::RADIUS_CONTROL as f32, bg);
    let fg = if enabled {
        theme::TEXT_PRIMARY.lerp_to_gamma(theme::ON_ACCENT, selected)
    } else {
        theme::TEXT_DISABLED
    };
    paint(
        painter,
        Rect::from_center_size(rect.center(), Vec2::splat(icon_size)),
        icon,
        fg,
    );
    response.on_hover_text(tooltip)
}
