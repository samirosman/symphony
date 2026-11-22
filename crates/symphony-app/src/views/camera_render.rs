//! Stage texture for Camera. Pan and rotation only move the existing texture.

use eframe::egui::{self, Color32, Pos2};
use symphony_core::id::Id;
use symphony_core::math::Affine;
use symphony_render::{render_camera, StrokePreview};

use crate::session::{Session, Tool};

#[derive(PartialEq)]
struct RenderKey {
    session: String,
    version: u64,
    frame: u32,
    stage: [u32; 2],
    selection: Option<(Id, Vec<Id>, Affine)>,
}

#[derive(Default)]
pub(super) struct Cache {
    key: Option<RenderKey>,
    texture: Option<egui::TextureHandle>,
}

impl Cache {
    pub(super) fn paint(
        &mut self,
        painter: &egui::Painter,
        s: &Session,
        corners: [Pos2; 4],
        transform: Option<Affine>,
    ) -> bool {
        let selection = if s.tool == Tool::Select {
            s.selection
                .drawing
                .zip(transform)
                .map(|(drawing, transform)| (drawing, s.selection.strokes.clone(), transform))
        } else {
            None
        };
        let stage = [s.doc.settings.width.max(1), s.doc.settings.height.max(1)];
        let key = RenderKey {
            session: s.key.clone(),
            version: s.doc.version,
            frame: s.frame,
            stage,
            selection,
        };
        if self.key.as_ref() != Some(&key) {
            let scale = (1920.0 / stage[0].max(stage[1]) as f32).min(1.0);
            let [width, height] = stage.map(|size| (size as f32 * scale).round().max(1.0) as u32);
            let preview =
                key.selection
                    .as_ref()
                    .map(|(drawing, strokes, transform)| StrokePreview {
                        drawing: *drawing,
                        strokes,
                        transform: *transform,
                    });
            if let Some(image) = render_camera(&s.doc, s.frame, width, height, preview) {
                let image = egui::ColorImage::from_rgba_premultiplied(
                    [width as usize, height as usize],
                    image.data(),
                );
                if let Some(texture) = &mut self.texture {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.texture = Some(painter.ctx().load_texture(
                        "camera-display",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            } else {
                self.texture = None;
            }
            self.key = Some(key);
        }

        let Some(texture) = &self.texture else {
            return false;
        };
        let mut mesh = egui::Mesh::with_texture(texture.id());
        let uv = [
            Pos2::ZERO,
            Pos2::new(1.0, 0.0),
            Pos2::new(1.0, 1.0),
            Pos2::new(0.0, 1.0),
        ];
        mesh.vertices.extend(
            corners
                .into_iter()
                .zip(uv)
                .map(|(pos, uv)| egui::epaint::Vertex {
                    pos,
                    uv,
                    color: Color32::WHITE,
                }),
        );
        mesh.indices.extend([0, 1, 2, 0, 2, 3]);
        painter.add(egui::Shape::mesh(mesh));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Profile;
    use crate::views::camera::{self, CanvasState};
    use egui_kittest::Harness;
    use symphony_core::graph::{Gradient, NodeKind};

    #[test]
    fn camera_draws_the_display_texture_and_drawing_does_not() {
        let mut session = Session::new_scene();
        session.doc.settings.width = 100;
        session.doc.settings.height = 64;
        let gradient = session.doc.graph.add(
            "Gradient",
            NodeKind::Gradient(Gradient {
                start: [30, 50, 80, 255],
                end: [30, 50, 80, 255],
                angle: 0.0,
            }),
        );
        session.doc.graph.connect(gradient, session.doc.display, 0);

        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 480.0))
            .build_ui_state(
                |ui, (session, camera_state, drawing_state, drawing_only)| {
                    let state = if *drawing_only {
                        drawing_state
                    } else {
                        camera_state
                    };
                    camera::ui(ui, session, state, &[], *drawing_only, Profile::Harmony);
                },
                (
                    session,
                    CanvasState::default(),
                    CanvasState::default(),
                    false,
                ),
            );
        harness.state_mut().0.frame = 2;
        harness.run_steps(1);
        let camera_state = &harness.state().1;
        let texture = camera_state.display.texture.as_ref().unwrap().id();
        assert_eq!(camera_state.display.key.as_ref().unwrap().frame, 2);
        let image = render_camera(&harness.state().0.doc, 2, 100, 64, None).unwrap();
        assert_eq!(&image.data()[..4], &[30, 50, 80, 255]);

        let draws_camera_texture = |output: &egui::FullOutput| {
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Mesh(mesh) if mesh.texture_id == texture)
            })
        };
        assert!(draws_camera_texture(harness.output()));
        harness.state_mut().3 = true;
        harness.run_steps(1);
        assert!(!draws_camera_texture(harness.output()));
        assert!(harness.state().2.display.texture.is_none());
    }
}
