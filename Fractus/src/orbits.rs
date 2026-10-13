//! Closed roulette curves with moving pen offsets and luminous filaments.
//! The rolling-circle construction follows the parametric hypotrochoid:
//! https://mathworld.wolfram.com/Hypotrochoid.html
use super::{Scene, SceneKind};
use image::{Rgb, RgbImage};
use std::f32::consts::TAU;

pub(super) fn render(scene: &Scene, image: &mut RgbImage, phase: f32) {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let mut light = vec![[0.0_f32; 3]; width * height];
    let seed =
        ((scene.seed ^ (scene.seed >> 23)).wrapping_mul(0x9e37_79b9) % 10007) as f32 / 10007.0;
    let rose = matches!(scene.kind, SceneKind::RoseCurve);
    // A rose closes in one turn. Retracing it repeatedly accumulates white
    // exposure and hides its palette instead of adding geometric detail.
    let turns = if rose {
        1
    } else {
        2 + (scene.seed % 3) as usize
    };
    let teeth = scene.symmetry as f32 + 1.0;
    let inner = turns as f32 / (teeth + turns as f32);
    let ratio = (1.0 - inner) / inner;
    let scale = scene.width.min(scene.height) as f32 * 0.43 * (scene.zoom as f32).sqrt().min(1.4);
    let center = (scene.width as f32 * 0.5, scene.height as f32 * 0.5);
    let layers = if rose {
        (scene.iterations / 15).clamp(12, 28)
    } else {
        (scene.iterations / 30).clamp(6, 14)
    } as usize;
    let samples = (scene.width.max(scene.height) as usize * turns * 4).clamp(1000, 16000);
    let (low, high) = super::palette(&scene.palette);
    for layer in 0..layers {
        let depth = layer as f32 / (layers - 1) as f32;
        let pen = 0.24 + depth * 0.46 + phase.sin() * scene.motion * 0.035;
        let rotate = seed * TAU + (phase.cos() - 1.0) * scene.motion * (depth - 0.5) * 0.23;
        let (sr, cr) = rotate.sin_cos();
        let accent = (depth * std::f32::consts::PI).sin().powi(2);
        let colour: [f32; 3] = std::array::from_fn(|channel| {
            if rose {
                let tone = 0.15 + depth * 0.7;
                return (f32::from(low[channel]) * (1.0 - tone) + f32::from(high[channel]) * tone)
                    / 255.0;
            }
            let base = f32::from(high[channel]) / 255.0;
            let complementary = f32::from(high[(channel + 1) % 3]) / 255.0;
            0.25 + 0.75 * (base * (1.0 - accent * 0.6) + complementary * accent * 0.6)
        });
        let mut previous: Option<(f32, f32)> = None;
        for index in 0..=samples {
            let t = index as f32 / samples as f32 * TAU * turns as f32;
            let orbit = ratio * t;
            let (x, y) = match scene.kind {
                SceneKind::RoseCurve => {
                    // Petal bands belong to the same rose family. A seeded
                    // harmonic rib and depth-dependent turn reveal its inner
                    // structure without compositing another decorative subject.
                    let petals = (scene.symmetry / 2).max(3) as f32;
                    let angle = t
                        + rotate
                        + depth * (0.035 + seed * 0.075)
                        + phase.sin() * scene.motion * (depth - 0.5) * 0.035;
                    let lobe = (petals * angle).cos().abs();
                    let rib = (petals * 3.0 * angle + seed * TAU).sin();
                    let radius =
                        (0.22 + depth * 0.76) * lobe.powf(0.75 + seed * 0.25) * (0.96 + rib * 0.04);
                    (radius * angle.cos(), radius * angle.sin())
                }
                SceneKind::OrbitalLace => {
                    let radius =
                        0.57 + depth * 0.2 + 0.12 * (teeth * t + phase.sin() * scene.motion).cos();
                    (radius * t.cos(), radius * t.sin())
                }
                _ => (
                    ((1.0 - inner) * t.cos() + pen * orbit.cos()) / 1.5,
                    ((1.0 - inner) * t.sin() - pen * orbit.sin()) / 1.5,
                ),
            };
            let point = (
                center.0 + scale * (x * cr - y * sr),
                center.1 + scale * (x * sr + y * cr),
            );
            if let Some(from) = previous {
                let steps = ((point.0 - from.0)
                    .abs()
                    .max((point.1 - from.1).abs())
                    .ceil() as usize)
                    .clamp(1, 256);
                for step in 0..steps {
                    let mix = step as f32 / steps as f32;
                    deposit(
                        &mut light,
                        width,
                        height,
                        from.0 + (point.0 - from.0) * mix,
                        from.1 + (point.1 - from.1) * mix,
                        colour,
                    );
                }
            }
            previous = Some(point);
        }
    }
    let halo = blur(&blur(&light, width, height), width, height);
    for (index, pixel) in image.pixels_mut().enumerate() {
        let x = (index % width) as f32 / width as f32 - 0.5;
        let y = (index / width) as f32 / height as f32 - 0.5;
        let vignette = (1.0 - (x * x + y * y)).max(0.0);
        *pixel = Rgb(std::array::from_fn(|channel| {
            let exposure = light[index][channel] * 0.68 + halo[index][channel] * 1.8;
            ((1.0 - (-exposure).exp()) * 243.0 + [3.0, 5.0, 10.0][channel] * vignette) as u8
        }));
    }
}

fn deposit(light: &mut [[f32; 3]], width: usize, height: usize, x: f32, y: f32, colour: [f32; 3]) {
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let fx = x - x.floor();
    let fy = y - y.floor();
    for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
            let (px, py) = (ix + dx, iy + dy);
            if px >= 0 && py >= 0 && px < width as i32 && py < height as i32 {
                let pixel = &mut light[py as usize * width + px as usize];
                for channel in 0..3 {
                    pixel[channel] += colour[channel] * wx * wy * 0.48;
                }
            }
        }
    }
}

fn blur(source: &[[f32; 3]], width: usize, height: usize) -> Vec<[f32; 3]> {
    let mut output = vec![[0.0; 3]; source.len()];
    for y in 0..height {
        for x in 0..width {
            for (dx, dy, weight) in [
                (0, 0, 0.4),
                (-1, 0, 0.15),
                (1, 0, 0.15),
                (0, -1, 0.15),
                (0, 1, 0.15),
            ] {
                let px = (x as i32 + dx).clamp(0, width as i32 - 1) as usize;
                let py = (y as i32 + dy).clamp(0, height as i32 - 1) as usize;
                for channel in 0..3 {
                    output[y * width + x][channel] += source[py * width + px][channel] * weight;
                }
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roulette_geometry_closes_its_motion_and_responds_to_the_seed() {
        let mut scene = Scene::parse(
            "version 2\ntype=spirograph\nwidth=96\nheight=128\niterations=120\nseed=463",
        )
        .unwrap();
        let render_at = |phase| {
            let mut image = RgbImage::new(96, 128);
            render(&scene, &mut image, phase);
            image
        };
        let first = render_at(0.0);
        assert!(super::super::frame_delta(&first, &render_at(TAU)) < 0.0001);
        assert!(super::super::frame_delta(&first, &render_at(1.2)) > 0.002);
        scene.seed += 7;
        let mut changed = RgbImage::new(96, 128);
        render(&scene, &mut changed, 0.0);
        assert!(super::super::frame_delta(&first, &changed) > 0.01);
    }
}
