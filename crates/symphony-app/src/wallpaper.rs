//! Wallpaper and glass masks share a small ladder of prefiltered textures.
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use image::{imageops, ImageReader, RgbImage};
use serde::{Deserialize, Serialize};
use symphony_ui::theme;

const BLUR_LEVELS: [f32; 5] = [0.0, 3.0, 8.0, 15.0, 26.0];
const GLASS_BLUR: u8 = 32;

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub enum Choice {
    #[default]
    Black,
    Plain,
    Studio,
    Dusk,
    Custom(PathBuf),
}

impl Choice {
    fn label(&self) -> &'static str {
        match self {
            Self::Black => "Black",
            Self::Plain => "Plain",
            Self::Studio => "Studio",
            Self::Dusk => "Dusk",
            Self::Custom(_) => "Custom picture…",
        }
    }
}

pub struct Wallpaper {
    pub choice: Choice,
    pub blur: u8,
    pending: Option<Receiver<Result<Vec<RgbImage>, String>>>,
    textures: Vec<egui::TextureHandle>,
    backdrop: Rect,
    error: Option<String>,
}

impl Wallpaper {
    pub fn new(ctx: &egui::Context, choice: Choice, blur: u8) -> Self {
        let mut wallpaper = Self {
            choice: Choice::Black,
            pending: None,
            blur: blur.min(100),
            textures: Vec::new(),
            backdrop: Rect::NOTHING,
            error: None,
        };
        wallpaper.select(ctx, choice);
        wallpaper
    }

    fn select(&mut self, ctx: &egui::Context, choice: Choice) {
        self.textures.clear();
        self.pending = None;
        self.error = None;
        if let Choice::Custom(path) = &choice {
            let path = path.clone();
            let ctx = ctx.clone();
            let (tx, rx) = mpsc::channel();
            self.pending = Some(rx);
            std::thread::spawn(move || {
                let result = load(&path);
                let _ = tx.send(result);
                ctx.request_repaint();
            });
        }
        self.choice = choice;
    }

    pub fn paint(&mut self, ui: &egui::Ui) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(Ok(images)) => {
                    self.textures = images
                        .into_iter()
                        .enumerate()
                        .map(|(level, image)| {
                            let size = [image.width() as usize, image.height() as usize];
                            ui.ctx().load_texture(
                                format!("workspace-wallpaper-{level}"),
                                egui::ColorImage::from_rgb(size, image.as_raw()),
                                egui::TextureOptions::LINEAR,
                            )
                        })
                        .collect();
                    self.pending = None;
                }
                Ok(Err(error)) => {
                    self.error = Some(error);
                    self.pending = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.error = Some("The wallpaper couldn’t be loaded.".into());
                    self.pending = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let rect = ui.max_rect();
        self.backdrop = rect;
        let painter = ui.painter();
        let palette = match self.choice {
            Choice::Studio => Some([[46, 56, 59], [28, 36, 43], [21, 25, 31], [35, 40, 47]]),
            Choice::Dusk => Some([[60, 43, 64], [35, 48, 69], [28, 29, 44], [49, 37, 51]]),
            _ => None,
        };
        if let Some(palette) = palette {
            let mut mesh = egui::Mesh::default();
            for (pos, [r, g, b]) in [
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                rect.left_bottom(),
            ]
            .into_iter()
            .zip(palette)
            {
                mesh.colored_vertex(pos, Color32::from_rgb(r, g, b));
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(egui::Shape::mesh(mesh));
        } else {
            painter.rect_filled(
                rect,
                0.0,
                if self.choice == Choice::Black {
                    Color32::BLACK
                } else {
                    theme::BG_BASE
                },
            );
            if let Some(image) = self.sample(rect, 0.0, self.blur, Color32::WHITE) {
                painter.add(image);
                painter.rect_filled(rect, 0.0, Color32::from_black_alpha(65));
            }
        }
    }

    /// Sample the same wallpaper in window coordinates, clipped to rounded glass.
    pub fn glass(&self, rect: Rect) -> Option<egui::Shape> {
        self.sample(
            rect,
            f32::from(theme::GLASS_RADIUS),
            self.blur.max(GLASS_BLUR),
            Color32::from_gray(204),
        )
    }

    fn sample(&self, rect: Rect, radius: f32, amount: u8, tint: Color32) -> Option<egui::Shape> {
        if !rect.is_positive() || !self.backdrop.is_positive() || self.textures.is_empty() {
            return None;
        }
        let (lower, upper, blend) = blur_mix(amount);
        let shape = |level: usize, tint| {
            egui::Shape::mesh(mask_mesh(
                self.textures[level].id(),
                rect,
                self.backdrop,
                radius,
                tint,
            ))
        };
        let lower = shape(lower, tint);
        if blend == 0.0 {
            Some(lower)
        } else {
            Some(egui::Shape::Vec(vec![
                lower,
                shape(upper, tint.gamma_multiply(blend)),
            ]))
        }
    }

    pub fn preferences(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Wallpaper");
            egui::ComboBox::from_id_salt("wallpaper")
                .selected_text(self.choice.label())
                .show_ui(ui, |ui| {
                    for choice in [Choice::Black, Choice::Plain, Choice::Studio, Choice::Dusk] {
                        if ui
                            .selectable_label(self.choice == choice, choice.label())
                            .clicked()
                        {
                            self.select(ui.ctx(), choice);
                        }
                    }
                    if ui.button("Choose picture…").clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Wallpaper image", &["png", "jpg", "jpeg"])
                            .pick_file()
                        {
                            self.select(ui.ctx(), Choice::Custom(path));
                        }
                    }
                });
        });
        if matches!(self.choice, Choice::Custom(_)) {
            symphony_ui::widgets::precise_slider(
                ui,
                egui::Slider::new(&mut self.blur, 0..=100)
                    .text("Wallpaper blur")
                    .suffix("%"),
            );
        }
        if self.pending.is_some() {
            ui.horizontal(|ui| {
                symphony_ui::feedback::spinner(ui, 14.0);
                ui.label("Preparing wallpaper");
                if ui.button("Cancel").clicked() {
                    self.select(ui.ctx(), Choice::Plain);
                }
            });
        }
        if let Some(error) = &self.error {
            ui.label(error);
        }
    }
}

fn blur_mix(amount: u8) -> (usize, usize, f32) {
    let max = BLUR_LEVELS.len() - 1;
    let level = f32::from(amount.min(100)) * max as f32 / 100.0;
    let lower = level.floor() as usize;
    (lower, (lower + 1).min(max), level.fract())
}

fn mask_mesh(
    id: egui::TextureId,
    rect: Rect,
    backdrop: Rect,
    radius: f32,
    tint: Color32,
) -> egui::Mesh {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let uv = |point: Pos2| {
        let local = point - backdrop.min;
        Pos2::new(local.x / backdrop.width(), local.y / backdrop.height())
    };
    let mut mesh = egui::Mesh::with_texture(id);
    let mut vertex = |pos| {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv: uv(pos),
            color: tint,
        })
    };
    vertex(rect.center());
    for (index, corner) in [
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ]
    .into_iter()
    .enumerate()
    {
        if radius == 0.0 {
            vertex(corner);
            continue;
        }
        let offset = corner - rect.center();
        let arc = corner - Vec2::new(offset.x.signum() * radius, offset.y.signum() * radius);
        for segment in 0..=6 {
            let angle = (index as f32 - 1.0 + segment as f32 / 6.0) * std::f32::consts::FRAC_PI_2;
            vertex(arc + Vec2::angled(angle) * radius);
        }
    }
    let last = mesh.vertices.len() as u32 - 1;
    for index in 1..=last {
        mesh.add_triangle(0, index, if index == last { 1 } else { index + 1 });
    }
    mesh
}

fn load(path: &Path) -> Result<Vec<RgbImage>, String> {
    let result = (|| -> image::ImageResult<_> {
        let mut reader = ImageReader::open(path)?.with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        reader.decode()
    })();
    let image = result.map_err(|e| format!("Couldn’t open the wallpaper ({e})."))?;
    let sharp = image.thumbnail(1600, 1600).into_rgb8();
    let filtered = image
        .resize(768, 768, imageops::FilterType::Triangle)
        .into_rgb8();
    let levels = std::iter::once(sharp)
        .chain(
            BLUR_LEVELS[1..]
                .iter()
                .map(|&sigma| imageops::blur(&filtered, sigma)),
        )
        .collect();
    Ok(levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_slider_blends_cached_levels_and_preserves_zero() {
        assert_eq!(blur_mix(0), (0, 1, 0.0));
        assert_eq!(blur_mix(25), (1, 2, 0.0));
        assert_eq!(blur_mix(100), (4, 4, 0.0));
        let (lower, upper, blend) = blur_mix(62);
        assert_eq!((lower, upper), (2, 3));
        assert!((blend - 0.48).abs() < 0.0001);
    }

    #[test]
    fn moving_glass_resamples_wallpaper_in_screen_coordinates() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 400.0));
        let pane = Rect::from_min_size(Pos2::new(100.0, 120.0), Vec2::new(200.0, 120.0));
        let id = egui::TextureId::Managed(1);
        let mask = mask_mesh(id, pane, screen, 12.0, Color32::WHITE);
        let moved = mask_mesh(
            id,
            pane.translate(Vec2::X * 80.0),
            screen,
            12.0,
            Color32::WHITE,
        );
        assert_eq!(mask.vertices[0].pos, pane.center());
        assert_eq!(moved.texture_id, mask.texture_id);
        assert!((moved.vertices[0].uv.x - mask.vertices[0].uv.x - 0.1).abs() < 0.0001);
        assert!(mask
            .vertices
            .iter()
            .all(|v| v.pos.distance(pane.right_top()) > 0.0));
    }
}
