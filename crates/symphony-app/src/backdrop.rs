//! Glass with a live blur of whatever is behind (Preferences, floating
//! views and windows, search). The blur itself is drawn by the vendored
//! renderer (vendor/egui-wgpu, `BackdropBlur`); this builds the shapes:
//! the blur marker, the blurred backdrop in the window's rounded shape, and
//! a dark grey tint over it. With interface effects off, or before the
//! first blur exists, it's the plain solid glass.

use eframe::egui::{self, Color32, Pos2, Rect, Shape};
use eframe::egui_wgpu;
use symphony_ui::theme;

/// Tint over the blurred backdrop: a very dark grey, translucent enough to
/// show the blur.
const OVER_BLUR: Color32 = Color32::from_rgba_premultiplied(24, 25, 28, 184);

/// The shapes to put under a floating surface's contents.
pub fn glass(ctx: &egui::Context, rect: Rect, radius: f32) -> Vec<Shape> {
    if !theme::effects() || !rect.is_positive() {
        return vec![Shape::rect_filled(rect, radius, theme::GLASS_FLOATING)];
    }
    let mut shapes = vec![Shape::Callback(egui::PaintCallback {
        rect,
        callback: std::sync::Arc::new(egui_wgpu::BackdropBlur),
    })];
    match egui_wgpu::backdrop_texture_id() {
        Some(id) => {
            shapes.push(Shape::mesh(rounded_mesh(ctx, rect, radius, id)));
            shapes.push(Shape::rect_filled(rect, radius, OVER_BLUR));
        }
        // The first frame asks for the blur; until then, solid glass.
        None => shapes.push(Shape::rect_filled(rect, radius, theme::GLASS_FLOATING)),
    }
    shapes
}

/// A rounded rectangle textured with the full-screen backdrop, each point
/// sampling what's behind it.
fn rounded_mesh(ctx: &egui::Context, rect: Rect, radius: f32, id: egui::TextureId) -> egui::Mesh {
    let screen = ctx.input(|i| i.viewport_rect());
    let r = radius
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0)
        .max(0.0);
    let uv = |p: Pos2| {
        Pos2::new(
            (p.x - screen.left()) / screen.width().max(1.0),
            (p.y - screen.top()) / screen.height().max(1.0),
        )
    };
    let mut outline: Vec<Pos2> = Vec::new();
    let corners = [
        (Pos2::new(rect.right() - r, rect.top() + r), -90.0f32),
        (Pos2::new(rect.right() - r, rect.bottom() - r), 0.0),
        (Pos2::new(rect.left() + r, rect.bottom() - r), 90.0),
        (Pos2::new(rect.left() + r, rect.top() + r), 180.0),
    ];
    for (c, start) in corners {
        for k in 0..=6 {
            let a = (start + 90.0 * k as f32 / 6.0).to_radians();
            outline.push(Pos2::new(c.x + a.cos() * r, c.y + a.sin() * r));
        }
    }
    let mut mesh = egui::Mesh::with_texture(id);
    let centre = rect.center();
    mesh.vertices.push(egui::epaint::Vertex {
        pos: centre,
        uv: uv(centre),
        color: Color32::WHITE,
    });
    for p in &outline {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: *p,
            uv: uv(*p),
            color: Color32::WHITE,
        });
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.indices.extend([0, 1 + i, 1 + (i + 1) % n]);
    }
    mesh
}
