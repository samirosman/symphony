//! One quadratic bend per cable. Pointer movement can push the bend a
//! little off its rest position, then it settles without oscillating.

use eframe::egui::{self, Pos2, Stroke, Vec2};

#[derive(Clone, Copy)]
pub struct Cable {
    from: Pos2,
    bend: Pos2,
    pub end: Pos2,
}

impl Cable {
    pub fn new(from: Pos2, end: Pos2) -> Self {
        let sag = ((end.x - from.x).abs() * 0.18).min(30.0);
        Self {
            from,
            end,
            bend: from.lerp(end, 0.5) + Vec2::Y * sag,
        }
    }

    pub fn offset(self) -> Vec2 {
        self.bend - Self::new(self.from, self.end).bend
    }

    pub fn with_offset(mut self, offset: Vec2) -> Self {
        self.bend += offset;
        self
    }

    pub fn follow(self, from: Pos2, end: Pos2, dt: f32) -> Self {
        let movement = (end - from) - (self.end - self.from);
        let decay = (-14.0 * dt.clamp(0.0, 0.1)).exp();
        let offset = (self.offset() - movement * 0.18) * decay;
        let limit = 14.0_f32.min(from.distance(end) * 0.2);
        Self::new(from, end).with_offset(offset * (limit / offset.length().max(limit).max(0.001)))
    }

    /// The part before t, with exactly the same curve as the full cable.
    pub fn prefix(self, t: f32) -> Self {
        let bend = self.from.lerp(self.bend, t);
        let end = bend.lerp(self.bend.lerp(self.end, t), t);
        Self {
            from: self.from,
            bend,
            end,
        }
    }

    pub fn retract(self, from: Pos2, remaining: f32) -> Self {
        Self {
            from,
            bend: from + (self.bend - self.from) * remaining,
            end: from + (self.end - self.from) * remaining,
        }
    }

    pub fn paint(self, painter: &egui::Painter, stroke: Stroke) {
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            [
                self.from,
                self.from.lerp(self.bend, 2.0 / 3.0),
                self.end.lerp(self.bend, 2.0 / 3.0),
                self.end,
            ],
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
    fn cable_follows_the_pointer_and_settles_without_overshoot() {
        let from = Pos2::ZERO;
        let end = Pos2::new(100.0, 70.0);
        let mut cable = Cable::new(from, from).follow(from, end, 1.0 / 60.0);
        assert_eq!(cable.end, end);
        assert!(cable.offset().length() <= 14.001);
        let initial = cable.offset();
        for _ in 0..60 {
            let next = cable.follow(from, end, 1.0 / 60.0);
            assert!(next.offset().length() <= cable.offset().length());
            assert!(next.offset().dot(initial) >= 0.0);
            cable = next;
        }
        assert!(cable.offset().length() < 0.01);
        assert_eq!(cable.retract(from, 0.0).end, from);
        assert_eq!(cable.prefix(1.0).end, end);
    }
}
