//! Cables bend while moving, then settle into a route aligned to the ports.

use eframe::egui::{self, Pos2, Stroke, Vec2};

const MAX_TRAIL: f32 = 21.0;

#[derive(Clone, Copy)]
pub struct Cable {
    from: Pos2,
    controls: [Pos2; 2],
    pub end: Pos2,
}

impl Cable {
    pub fn new(from: Pos2, end: Pos2) -> Self {
        let sag = ((end.x - from.x).abs() * 0.19).min(31.0);
        let bend = from.lerp(end, 0.5) + Vec2::Y * sag;
        Self {
            from,
            end,
            controls: [from.lerp(bend, 2.0 / 3.0), end.lerp(bend, 2.0 / 3.0)],
        }
    }

    /// Ease the loose curve into the port directions when connected.
    pub fn shape_for_ports(mut self, amount: f32) -> Self {
        let reach = (self.from.distance(self.end) * 0.34).min(86.0);
        self.controls[0] = self.controls[0].lerp(self.from + Vec2::Y * reach, amount);
        self.controls[1] = self.controls[1].lerp(self.end - Vec2::Y * reach, amount);
        self
    }

    pub fn offset(self) -> Vec2 {
        let [a, b] = Self::new(self.from, self.end).controls;
        (self.controls[0] - a) + (self.controls[1] - b)
    }

    pub fn with_offset(mut self, offset: Vec2) -> Self {
        self.controls[0] -= offset * 0.35;
        self.controls[1] += offset * 1.35;
        self
    }

    pub fn follow(self, from: Pos2, end: Pos2, dt: f32) -> Self {
        let movement = (end - from) - (self.end - self.from);
        let dt = dt.clamp(1.0 / 240.0, 0.1);
        let weight = 1.0 - (-16.0 * dt).exp();
        let limit = MAX_TRAIL.min(from.distance(end) * 0.2);
        let bounded = |v: Vec2| v * (limit / v.length().max(limit).max(0.001));
        // Ease the same two control points toward the direction of travel,
        // so reversing the pointer changes the bend smoothly.
        let target = bounded(movement * (-0.07 / dt));
        let offset = self.offset() * (1.0 - weight) + target * weight;
        Self::new(from, end).with_offset(bounded(offset))
    }

    /// The part before t, with exactly the same curve as the full cable.
    pub fn prefix(self, t: f32) -> Self {
        let [a, b] = self.controls;
        let ab = self.from.lerp(a, t);
        let bc = a.lerp(b, t);
        let cd = b.lerp(self.end, t);
        let bend = ab.lerp(bc, t);
        Self {
            from: self.from,
            controls: [ab, bend],
            end: bend.lerp(bc.lerp(cd, t), t),
        }
    }

    pub fn retract(self, from: Pos2, progress: f32) -> Self {
        let remaining = 1.0 - progress * progress * (3.0 - 2.0 * progress);
        let span = self.end - self.from;
        let sideways = Vec2::new(-span.y, span.x).normalized();
        let wiggle = (progress * std::f32::consts::TAU * 1.4).sin()
            * (1.0 - progress).powi(2)
            * (span.length() * 0.18).min(15.0);
        Self {
            from,
            controls: self.controls.map(|p| from + (p - self.from) * remaining),
            end: from + span * remaining + sideways * (wiggle * 0.5),
        }
        .with_offset(sideways * wiggle)
    }

    /// Add two small alternating bends to a moving cable, with fixed ends.
    pub fn paint_moving(self, painter: &egui::Painter, stroke: Stroke, flex: f32) {
        if flex.abs() < 0.1 {
            self.paint(painter, stroke);
            return;
        }
        let span = self.end - self.from;
        let normal = Vec2::new(-span.y, span.x).normalized();
        let flex = flex.clamp(-span.length() * 0.1, span.length() * 0.1);
        let points = (0..=24)
            .map(|step| {
                let t = step as f32 / 24.0;
                let envelope = (std::f32::consts::PI * t).sin().powi(2);
                self.prefix(t).end
                    + normal * (flex * envelope * (std::f32::consts::TAU * 2.0 * t).sin())
            })
            .collect();
        painter.add(egui::Shape::line(points, stroke));
    }

    pub fn paint(self, painter: &egui::Painter, stroke: Stroke) {
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            [self.from, self.controls[0], self.controls[1], self.end],
            false,
            egui::Color32::TRANSPARENT,
            stroke,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_can_add_a_shallow_second_bend() {
        let cable = Cable::new(Pos2::ZERO, Pos2::new(0.0, 100.0)).with_offset(Vec2::X * 10.0);
        assert!(cable.prefix(0.1).end.x < 0.0);
        assert!(cable.prefix(0.7).end.x > 0.0);
    }

    #[test]
    fn cable_follows_the_pointer_and_settles_without_overshoot() {
        let from = Pos2::ZERO;
        let end = Pos2::new(100.0, 70.0);
        let mut cable = Cable::new(from, from).follow(from, end, 1.0 / 60.0);
        assert_eq!(cable.end, end);
        assert!(cable.offset().length() <= MAX_TRAIL + 0.001);
        let initial = cable.offset();
        for _ in 0..60 {
            let next = cable.follow(from, end, 1.0 / 60.0);
            assert!(next.offset().length() <= cable.offset().length() + 0.001);
            assert!(next.offset().dot(initial) >= -0.001);
            cable = next;
        }
        assert!(cable.offset().length() < 0.01);
        assert_eq!(cable.retract(from, 1.0).end, from);
        assert_eq!(cable.prefix(1.0).end, end);
    }
}
