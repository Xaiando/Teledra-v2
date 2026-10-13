//! A measured floor for the operator's request to avoid primitive-looking art.
//! These are preview structure checks, not beauty scores or user approval.

use image::RgbImage;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VisualRichness {
    /// Measurements use an area-averaged 128 by 128 preview, independent of
    /// output resolution. This prevents more pixels from inflating detail.
    pub bright_fraction: f64,
    pub tonal_entropy_bits: f64,
    pub edge_fraction: f64,
}

impl VisualRichness {
    pub fn from_image(image: &RgbImage) -> Self {
        measure(image)
    }

    pub fn concerns(&self) -> Vec<String> {
        let mut concerns = Vec::new();
        if self.bright_fraction < 0.08 {
            concerns.push(format!(
                "only {:.1}% of the preview is visibly lit (minimum 8%)",
                self.bright_fraction * 100.0
            ));
        }
        if self.tonal_entropy_bits < 1.2 {
            concerns.push(format!(
                "tonal variation is {:.2} bits (minimum 1.20)",
                self.tonal_entropy_bits
            ));
        }
        if self.edge_fraction < 0.06 {
            concerns.push(format!(
                "spatial detail is {:.1}% (minimum 6%)",
                self.edge_fraction * 100.0
            ));
        }
        concerns
    }
}

pub(super) fn measure(image: &RgbImage) -> VisualRichness {
    const SIDE: usize = 128;
    let enlarged;
    let image = if image.width() < SIDE as u32 || image.height() < SIDE as u32 {
        enlarged = image::imageops::resize(
            image,
            image.width().max(SIDE as u32),
            image.height().max(SIDE as u32),
            image::imageops::FilterType::Nearest,
        );
        &enlarged
    } else {
        image
    };
    let mut cells = vec![[0.0_f64; 3]; SIDE * SIDE];
    let mut counts = vec![0_u32; SIDE * SIDE];
    // Area averaging rather than point sampling keeps thin strokes measurable.
    for (x, y, pixel) in image.enumerate_pixels() {
        let cell = (y as usize * SIDE / image.height() as usize) * SIDE
            + x as usize * SIDE / image.width() as usize;
        counts[cell] += 1;
        for channel in 0..3 {
            cells[cell][channel] += f64::from(pixel[channel]);
        }
    }
    let mut histogram = [0_u32; 16];
    let mut bright = 0;
    let mut sampled = 0;
    for (index, pixel) in cells.iter_mut().enumerate() {
        if counts[index] == 0 {
            continue;
        }
        for channel in pixel.iter_mut() {
            *channel /= f64::from(counts[index]);
        }
        let intensity = pixel.iter().copied().fold(0.0, f64::max);
        histogram[(intensity as usize / 16).min(15)] += 1;
        bright += usize::from(intensity >= 32.0);
        sampled += 1;
    }
    let mut edges = 0;
    let mut pairs = 0;
    for y in 0..SIDE {
        for x in 0..SIDE {
            let here = y * SIDE + x;
            if counts[here] == 0 {
                continue;
            }
            for next in [
                ((x + 1) < SIDE).then_some(here + 1),
                ((y + 1) < SIDE).then_some(here + SIDE),
            ]
            .into_iter()
            .flatten()
            {
                if counts[next] == 0 {
                    continue;
                }
                let difference = (0..3)
                    .map(|channel| (cells[here][channel] - cells[next][channel]).abs())
                    .fold(0.0, f64::max);
                edges += usize::from(difference > 24.0);
                pairs += 1;
            }
        }
    }
    let entropy = histogram
        .into_iter()
        .filter(|count| *count > 0)
        .map(|count| {
            let p = f64::from(count) / sampled.max(1) as f64;
            -p * p.log2()
        })
        .sum();
    VisualRichness {
        bright_fraction: bright as f64 / sampled.max(1) as f64,
        tonal_entropy_bits: entropy,
        edge_fraction: edges as f64 / pairs.max(1) as f64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn original_single_rose_outline_fails_while_internal_rose_structure_passes() {
        let scene = crate::Scene::parse("type=rose_curve; width=600; height=800; iterations=180; seed=11036126381597215977; palette=solar_gold; symmetry=12; zoom=1; motion=0.65").unwrap();
        // Reproduce the reported legacy renderer's seven-lobe outline.
        let mut outline = RgbImage::new(scene.width, scene.height);
        let samples = scene.iterations * 12;
        let mut prior: Option<(f32, f32)> = None;
        for index in 0..samples {
            let t = index as f32 / samples as f32 * std::f32::consts::TAU * 8.0;
            let radius = (7.0 * t).cos() * scene.width as f32 * 0.43;
            let point = (
                scene.width as f32 * 0.5 + radius * t.cos(),
                scene.height as f32 * 0.5 + radius * t.sin(),
            );
            if let Some(previous) = prior {
                crate::draw_line(
                    &mut outline,
                    previous.0,
                    previous.1,
                    point.0,
                    point.1,
                    Rgb([255, 255, 255]),
                );
            }
            prior = Some(point);
        }
        let old = measure(&outline);
        assert!(old.bright_fraction < 0.08, "{old:?}");
        assert!(old.tonal_entropy_bits < 1.2, "{old:?}");
        assert!(scene.layers.is_empty());
        let current = scene.require_visual_richness().unwrap();
        assert!(current.bright_fraction > old.bright_fraction * 2.0);
    }

    #[test]
    fn rose_controls_change_geometry_and_animation_closes_without_new_subjects() {
        let scene = crate::Scene::parse("type=rose_curve; width=160; height=192; iterations=180; seed=731; symmetry=12; motion=0.65").unwrap();
        let original = scene.render_frame(0.0);
        for changed in [
            crate::Scene {
                seed: 739,
                ..scene.clone()
            },
            crate::Scene {
                symmetry: 18,
                ..scene.clone()
            },
            crate::Scene {
                zoom: 1.2,
                ..scene.clone()
            },
            crate::Scene {
                iterations: 300,
                ..scene.clone()
            },
        ] {
            assert!(crate::frame_delta(&original, &changed.render_frame(0.0)) > 0.002);
            assert!(changed.layers.is_empty());
        }
        assert!(crate::frame_delta(&original, &scene.render_frame(1.2)) > 0.002);
        assert!(crate::frame_delta(&original, &scene.render_frame(std::f32::consts::TAU)) < 0.0001);
        let stationary = crate::Scene {
            motion: 0.0,
            ..scene
        };
        assert_eq!(stationary.render_frame(0.0), stationary.render_frame(1.2));
    }

    #[test]
    fn flat_shapes_and_gradients_do_not_count_as_detailed_compositions() {
        let mut flat = RgbImage::from_pixel(256, 256, Rgb([0, 0, 0]));
        for y in 64..192 {
            for x in 64..192 {
                flat.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }
        assert!(!measure(&flat).concerns().is_empty());
        let gradient = RgbImage::from_fn(256, 256, |x, _| Rgb([x as u8, x as u8, x as u8]));
        let facts = measure(&gradient);
        assert!(facts.tonal_entropy_bits > 3.0);
        assert!(
            facts
                .concerns()
                .iter()
                .any(|concern| concern.contains("spatial detail"))
        );
    }

    #[test]
    fn resolution_alone_does_not_make_a_plain_shape_richer() {
        let small = RgbImage::from_fn(128, 128, |x, y| {
            if (x / 32 + y / 32) % 2 == 0 {
                Rgb([255, 255, 255])
            } else {
                Rgb([0, 0, 0])
            }
        });
        let large = image::imageops::resize(&small, 512, 512, image::imageops::FilterType::Nearest);
        let a = measure(&small);
        let b = measure(&large);
        assert_eq!(a.bright_fraction, b.bright_fraction);
        assert_eq!(a.tonal_entropy_bits, b.tonal_entropy_bits);
        assert_eq!(a.edge_fraction, b.edge_fraction);
        assert!(!a.concerns().is_empty());
    }
}
