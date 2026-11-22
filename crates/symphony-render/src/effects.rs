//! Raster effects operate on premultiplied RGBA so transparent edges stay clean.

use symphony_core::graph::{Effect, Gradient};
use tiny_skia::Pixmap;

pub fn apply(image: &mut Pixmap, effect: Effect, scale: f32) {
    match effect {
        Effect::Blur { radius } => {
            let radius = (radius * scale).round().clamp(0.0, 256.0) as usize;
            if radius > 0 {
                let width = image.width() as usize;
                let height = image.height() as usize;
                let mut scratch = vec![0; image.data().len()];
                box_pass(image.data(), &mut scratch, width, height, 1, width, radius);
                box_pass(&scratch, image.data_mut(), height, width, width, 1, radius);
            }
        }
        Effect::Grain { amount, seed } => {
            let amount = amount.clamp(0.0, 1.0);
            for (index, pixel) in image
                .data_mut()
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .enumerate()
            {
                // Stable pixel-space noise. Modulate colour without changing coverage.
                let mut hash = (index as u32 ^ seed).wrapping_add(0x9e3779b9);
                hash = (hash ^ (hash >> 16)).wrapping_mul(0x85ebca6b);
                hash = (hash ^ (hash >> 13)).wrapping_mul(0xc2b2ae35);
                let noise = ((hash ^ (hash >> 16)) >> 24) as f32 / 127.5 - 1.0;
                let alpha = pixel[3] as f32;
                for channel in &mut pixel[..3] {
                    *channel = (*channel as f32 + noise * amount * alpha)
                        .round()
                        .clamp(0.0, alpha) as u8;
                }
            }
        }
        Effect::Opacity { amount } => {
            let amount = amount.clamp(0.0, 1.0);
            for value in image.data_mut() {
                *value = (*value as f32 * amount).round() as u8;
            }
        }
    }
}

fn box_pass(
    src: &[u8],
    dst: &mut [u8],
    length: usize,
    lines: usize,
    stride: usize,
    line_stride: usize,
    radius: usize,
) {
    let samples = (radius * 2 + 1) as u32;
    for line in 0..lines {
        let at = |offset| (line * line_stride + offset * stride) * 4;
        let mut sum = [0u32; 4];
        for i in 0..=radius.min(length - 1) {
            for (total, &value) in sum.iter_mut().zip(&src[at(i)..at(i) + 4]) {
                *total += value as u32;
            }
        }
        for i in 0..length {
            for (channel, &total) in sum.iter().enumerate() {
                dst[at(i) + channel] = ((total + samples / 2) / samples) as u8;
            }
            if let Some(left) = i.checked_sub(radius) {
                for (total, &value) in sum.iter_mut().zip(&src[at(left)..at(left) + 4]) {
                    *total -= value as u32;
                }
            }
            let right = i + radius + 1;
            if right < length {
                for (total, &value) in sum.iter_mut().zip(&src[at(right)..at(right) + 4]) {
                    *total += value as u32;
                }
            }
        }
    }
}

pub fn gradient(image: &mut Pixmap, gradient: Gradient) {
    let (sin, cos) = gradient.angle.to_radians().sin_cos();
    let width = image.width() as usize;
    let height = image.height() as usize;
    let span = cos.abs() * width as f32 + sin.abs() * height as f32;
    for (index, pixel) in image
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .enumerate()
    {
        let x = (index % width) as f32 + 0.5 - width as f32 / 2.0;
        let y = (index / width) as f32 + 0.5 - height as f32 / 2.0;
        let t = (0.5 + (x * cos + y * sin) / span).clamp(0.0, 1.0);
        let mix = |c: usize| gradient.start[c] as f32 * (1.0 - t) + gradient.end[c] as f32 * t;
        let alpha = mix(3) / 255.0;
        for (c, value) in pixel[..3].iter_mut().enumerate() {
            *value = (mix(c) * alpha + *value as f32 * (1.0 - alpha)).round() as u8;
        }
        pixel[3] = (mix(3) + pixel[3] as f32 * (1.0 - alpha)).round() as u8;
    }
}
