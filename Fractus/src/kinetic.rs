//! Continuous, seed-directed geometry. Animation changes the construction itself;
//! every time-dependent parameter is periodic, including the last-to-first seam.

use std::f32::consts::{PI, TAU};

use image::{Rgb, RgbImage};

use super::{Scene, SceneKind};

type Color = [f32; 3];

fn hash(seed: u64, salt: u64) -> f32 {
    let mut value = seed.wrapping_add(salt.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((value ^ (value >> 31)) >> 40) as f32 / 16_777_216.0
}

fn loop_phase(phase: f32) -> f32 {
    // f32::TAU is the public animation boundary; reduce before evaluating trig
    // so an explicitly rendered closing frame is byte-for-byte the first frame.
    phase.rem_euclid(TAU)
}

fn colors(name: &str) -> [Color; 6] {
    match name {
        "violet" | "royal" | "purple_haze" | "amethyst" => [
            [10., 7., 30.],
            [55., 30., 122.],
            [167., 77., 238.],
            [253., 172., 233.],
            [255., 219., 150.],
            [39., 151., 159.],
        ],
        "ember" | "gold" | "solar_gold" => [
            [20., 9., 17.],
            [118., 32., 39.],
            [231., 86., 37.],
            [255., 175., 76.],
            [255., 237., 184.],
            [45., 120., 133.],
        ],
        "ice" | "cyan" | "electric_cyan" | "blue_ocean" => [
            [5., 12., 32.],
            [24., 59., 127.],
            [35., 147., 199.],
            [103., 228., 230.],
            [221., 250., 236.],
            [163., 99., 206.],
        ],
        "neon_sunset" | "sunset" => [
            [18., 6., 34.],
            [77., 29., 117.],
            [197., 51., 131.],
            [255., 133., 92.],
            [255., 223., 140.],
            [69., 142., 164.],
        ],
        "twilight" | "pastel" => [
            [13., 14., 36.],
            [64., 61., 118.],
            [146., 112., 180.],
            [227., 171., 204.],
            [252., 224., 183.],
            [112., 193., 190.],
        ],
        "rainbow" | "ice_fire" => [
            [17., 12., 39.],
            [67., 51., 148.],
            [38., 174., 194.],
            [236., 230., 174.],
            [242., 109., 74.],
            [146., 48., 132.],
        ],
        "monochrome" | "chrome" => [
            [7., 9., 14.],
            [40., 49., 63.],
            [100., 113., 130.],
            [183., 198., 211.],
            [246., 247., 242.],
            [114., 128., 145.],
        ],
        _ => [
            [5., 17., 23.],
            [12., 70., 78.],
            [30., 156., 135.],
            [142., 220., 159.],
            [248., 228., 164.],
            [121., 89., 167.],
        ],
    }
}

fn gradient(palette: &[Color; 6], amount: f32) -> Color {
    let value = amount.rem_euclid(1.0) * 6.0;
    let index = value as usize;
    let blend = value.fract();
    let blend = blend * blend * (3.0 - 2.0 * blend);
    std::array::from_fn(|channel| {
        palette[index % 6][channel] * (1.0 - blend) + palette[(index + 1) % 6][channel] * blend
    })
}

fn rgb(color: Color, intensity: f32) -> Rgb<u8> {
    Rgb(std::array::from_fn(|i| {
        (color[i] * intensity).clamp(0.0, 255.0) as u8
    }))
}

fn complex_power(x: f64, y: f64, degree: u32) -> (f64, f64) {
    let (mut real, mut imaginary) = (x, y);
    for _ in 1..degree {
        (real, imaginary) = (real * x - imaginary * y, real * y + imaginary * x);
    }
    (real, imaginary)
}

pub(super) fn render_escape(scene: &Scene, image: &mut RgbImage, phase: f32) {
    let phase = loop_phase(phase) as f64;
    let motion = scene.motion as f64;
    let variant = (hash(scene.seed, 4) * 5.0) as usize;
    let julia = matches!(scene.kind, SceneKind::Julia);
    let degree = if matches!(scene.kind, SceneKind::Multibrot) {
        3 + (hash(scene.seed, 7) * 3.0) as u32
    } else {
        2
    };
    let (center_x, center_y, half_span) = match scene.kind {
        SceneKind::Julia => (0.0, 0.0, 1.58),
        SceneKind::Multibrot => (0.0, 0.0, 1.43),
        SceneKind::Tricorn => (-0.2, 0.0, 1.62),
        SceneKind::BurningShip => [
            (-1.76, -0.035, 0.10),
            (-1.70, -0.042, 0.15),
            (-1.84, -0.024, 0.07),
            (-1.77, -0.045, 0.13),
            (-1.66, -0.055, 0.19),
        ][variant],
        _ => [
            (-0.745, 0.186, 0.24),
            (-0.7435, 0.1314, 0.012),
            (-0.7453, 0.1127, 0.028),
            (-0.16, 0.89, 0.23),
            (-1.255, 0.065, 0.19),
        ][variant],
    };
    let julia_centers = [
        (-0.8, 0.156),
        (-0.745, 0.186),
        (-0.4, 0.6),
        (-0.7269, 0.1889),
        (-0.70176, -0.3842),
    ];
    let (julia_x, julia_y) = julia_centers[variant];
    let offset = hash(scene.seed, 11) as f64 * std::f64::consts::TAU;
    let orbit_size = (0.006 + hash(scene.seed, 12) as f64 * 0.012) * motion;
    let constant = (
        julia_x + orbit_size * (phase + offset).cos(),
        julia_y + orbit_size * (phase + offset).sin(),
    );
    // Scale by the shorter image dimension. Portrait and landscape images keep
    // circular features circular, exposing more of the plane on the long axis.
    let pixel_scale = 2.0 * half_span / scene.zoom
        * (0.86 + hash(scene.seed, 14) as f64 * 0.29)
        * (1.0 + 0.13 * motion * phase.sin())
        / image.width().min(image.height()) as f64;
    let palette = colors(&scene.palette);
    let hue = hash(scene.seed, 13) * 0.16;
    let sample = |x: f64, y: f64| {
        let px = (x - scene.width as f64 * 0.5) * pixel_scale + center_x;
        let py = (y - scene.height as f64 * 0.5) * pixel_scale + center_y;
        let (mut zx, mut zy, cx, cy) = if julia {
            (px, py, constant.0, constant.1)
        } else {
            (0.0, 0.0, px, py)
        };
        let mut count = 0;
        let mut trap = 10.0_f64;
        let mut magnitude = 0.0;
        while magnitude < 256.0 && count < scene.iterations {
            (zx, zy) = match scene.kind {
                SceneKind::BurningShip => (zx * zx - zy * zy + cx, 2.0 * (zx * zy).abs() + cy),
                SceneKind::Tricorn => (zx * zx - zy * zy + cx, -2.0 * zx * zy + cy),
                SceneKind::Multibrot => {
                    let powered = complex_power(zx, zy, degree);
                    (powered.0 + cx, powered.1 + cy)
                }
                _ => (zx * zx - zy * zy + cx, 2.0 * zx * zy + cy),
            };
            magnitude = zx * zx + zy * zy;
            trap = trap.min((magnitude - 0.55).abs());
            count += 1;
        }
        if count == scene.iterations {
            // Deep interiors stay quiet; a little orbit-trap tint reveals shape
            // without turning the negative space into a solid saturated blob.
            rgb(palette[0], 0.34 + 0.16 / (1.0 + trap as f32 * 8.0))
        } else {
            let smooth = (count as f64 + 1.0 - (magnitude.ln() * 0.5).ln() / (degree as f64).ln())
                .max(0.0) as f32;
            let amount = smooth * 0.026 + hue + 0.06 / (1.0 + trap as f32 * 5.0);
            let light = 0.42 + 0.58 * (1.0 - (-smooth * 0.075).exp());
            rgb(gradient(&palette, amount), light)
        }
    };
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = sample(x as f64 + 0.5, y as f64 + 0.5);
    }
    // Only sharp boundaries need extra orbit calculations. Supersampling these
    // pixels keeps recursive filaments from turning into flickering single-pixel
    // sparks while broad, smoothly shaded regions remain cheap to render.
    let first_pass = image.clone();
    let width = image.width();
    let height = image.height();
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let edge = [
            (x.saturating_sub(1), y),
            ((x + 1).min(width - 1), y),
            (x, y.saturating_sub(1)),
            (x, (y + 1).min(height - 1)),
        ]
        .iter()
        .any(|&(nx, ny)| {
            first_pass
                .get_pixel(nx, ny)
                .0
                .iter()
                .zip(pixel.0)
                .map(|(a, b)| (*a as i16 - b as i16).unsigned_abs() as u32)
                .sum::<u32>()
                > 95
        });
        if edge {
            let samples = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]
                .map(|(dx, dy)| sample(x as f64 + dx, y as f64 + dy));
            *pixel = Rgb(std::array::from_fn(|channel| {
                (samples
                    .iter()
                    .map(|sample| sample[channel] as u16)
                    .sum::<u16>()
                    / 4) as u8
            }));
        }
    }
}

pub(super) fn render_newton(scene: &Scene, image: &mut RgbImage, phase: f32) {
    let phase = loop_phase(phase) as f64;
    let motion = scene.motion as f64;
    let degree = 3 + (hash(scene.seed, 29) * 4.0) as u32;
    let seed_phase = hash(scene.seed, 31) as f64 * std::f64::consts::TAU;
    let angle = 0.13 * motion * (phase + seed_phase).sin();
    let amplitude = 1.0 + 0.09 * motion * phase.cos();
    let target = (amplitude * angle.cos(), amplitude * angle.sin());
    let relaxation = (
        0.96 + 0.06 * motion * phase.sin(),
        0.09 * motion * phase.cos(),
    );
    let scale = 3.4 / image.width().min(image.height()) as f64 / scene.zoom;
    let palette = colors(&scene.palette);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let mut zx = (x as f64 + 0.5 - scene.width as f64 * 0.5) * scale;
        let mut zy = (y as f64 + 0.5 - scene.height as f64 * 0.5) * scale;
        let mut count = 0.0;
        let mut converged = false;
        let mut previous_error = 1.0_f64;
        for iteration in 0..scene.iterations.min(100) {
            let (dx, dy) = complex_power(zx, zy, degree - 1);
            let (fx, fy) = (dx * zx - dy * zy - target.0, dx * zy + dy * zx - target.1);
            let error = fx * fx + fy * fy;
            if error < 1e-10 {
                let fraction = if previous_error > 1e-10 && error > 0.0 {
                    ((1e-10_f64.ln() - previous_error.ln()) / (error.ln() - previous_error.ln()))
                        .clamp(0.0, 1.0)
                } else {
                    1.0
                };
                count = iteration as f64 - 1.0 + fraction;
                converged = true;
                break;
            }
            let denominator = degree as f64 * (dx * dx + dy * dy);
            if denominator < 1e-24 || !denominator.is_finite() {
                break;
            }
            let step = (
                (fx * dx + fy * dy) / denominator,
                (fy * dx - fx * dy) / denominator,
            );
            zx -= step.0 * relaxation.0 - step.1 * relaxation.1;
            zy -= step.0 * relaxation.1 + step.1 * relaxation.0;
            previous_error = error;
        }
        if converged {
            let root = ((zy.atan2(zx) - angle / degree as f64) / std::f64::consts::TAU
                * degree as f64)
                .round();
            let hue = root as f32 / degree as f32 + 0.30 + hash(scene.seed, 32) * 0.2;
            let contour = 0.5 + 0.5 * (count as f32 * 0.75).cos();
            let light = (0.28 + 0.55 * contour + (count as f32 * 0.035).min(0.17)).min(1.0);
            *pixel = rgb(gradient(&palette, hue + count as f32 * 0.009), light);
        } else {
            *pixel = rgb(palette[0], 0.4);
        }
    }
}

struct Ink<'a> {
    image: &'a mut RgbImage,
    scale: f32,
    center: (f32, f32),
}

impl Ink<'_> {
    fn point(&self, p: (f32, f32)) -> (f32, f32) {
        (
            self.center.0 + p.0 * self.scale,
            self.center.1 + p.1 * self.scale,
        )
    }

    /// Analytic segment coverage gives subpixel antialiasing and a restrained
    /// halo. Max compositing keeps tessellation joints from accumulating white.
    fn line(&mut self, a: (f32, f32), b: (f32, f32), color: Color, width: f32) {
        let (a, b) = (self.point(a), self.point(b));
        let core = (width * self.scale).max(0.38);
        let halo = core * 2.3 + 1.25;
        let left = (a.0.min(b.0) - halo).floor().max(0.0) as u32;
        let right = ((a.0.max(b.0) + halo).ceil() as u32).min(self.image.width().saturating_sub(1));
        let top = (a.1.min(b.1) - halo).floor().max(0.0) as u32;
        let bottom =
            ((a.1.max(b.1) + halo).ceil() as u32).min(self.image.height().saturating_sub(1));
        let vx = b.0 - a.0;
        let vy = b.1 - a.1;
        let length2 = (vx * vx + vy * vy).max(1e-10);
        for y in top..=bottom {
            for x in left..=right {
                let dx = x as f32 + 0.5 - a.0;
                let dy = y as f32 + 0.5 - a.1;
                let t = ((dx * vx + dy * vy) / length2).clamp(0.0, 1.0);
                let distance = ((dx - vx * t).powi(2) + (dy - vy * t).powi(2)).sqrt();
                let coverage = (core + 0.70 - distance).clamp(0.0, 1.0);
                let glow = (1.0 - distance / halo).max(0.0).powi(2) * 0.23;
                let intensity = (coverage * 0.88 + glow).min(1.0);
                if intensity > 0.005 {
                    let pixel = self.image.get_pixel_mut(x, y);
                    for channel in 0..3 {
                        pixel[channel] = pixel[channel].max((color[channel] * intensity) as u8);
                    }
                }
            }
        }
    }

    fn curve(&mut self, steps: usize, color: Color, width: f32, f: impl Fn(f32) -> (f32, f32)) {
        let mut previous = f(0.0);
        for i in 1..=steps {
            let point = f(i as f32 / steps as f32);
            self.line(previous, point, color, width);
            previous = point;
        }
    }

    fn circle(&mut self, center: (f32, f32), radius: f32, color: Color, width: f32) {
        let steps = (radius * self.scale * 2.8).clamp(24.0, 560.0) as usize;
        self.curve(steps, color, width, |u| {
            let angle = u * TAU;
            (
                center.0 + radius * angle.cos(),
                center.1 + radius * angle.sin(),
            )
        });
    }

    fn petal(
        &mut self,
        angle: f32,
        span: (f32, f32),
        breadth: f32,
        bend: f32,
        color: Color,
        width: f32,
    ) {
        let (start, end) = span;
        let (sin, cos) = angle.sin_cos();
        if width >= 0.0019 {
            // Shade the actual leaf surface before outlining it. A directional
            // ridge and a soft interior falloff give each petal volume while
            // preserving the dark gaps between the overlapping constructions.
            let extent = breadth + bend.abs();
            let corners = [
                (start * cos - extent * sin, start * sin + extent * cos),
                (start * cos + extent * sin, start * sin - extent * cos),
                (end * cos - extent * sin, end * sin + extent * cos),
                (end * cos + extent * sin, end * sin - extent * cos),
            ]
            .map(|p| self.point(p));
            let left = corners
                .iter()
                .map(|p| p.0)
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.0) as u32;
            let right = (corners
                .iter()
                .map(|p| p.0)
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil() as u32)
                .min(self.image.width().saturating_sub(1));
            let top = corners
                .iter()
                .map(|p| p.1)
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.0) as u32;
            let bottom = (corners
                .iter()
                .map(|p| p.1)
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil() as u32)
                .min(self.image.height().saturating_sub(1));
            for y in top..=bottom {
                for x in left..=right {
                    let dx = (x as f32 + 0.5 - self.center.0) / self.scale;
                    let dy = (y as f32 + 0.5 - self.center.1) / self.scale;
                    let u = (dx * cos + dy * sin - start) / (end - start);
                    if !(0.0..=1.0).contains(&u) {
                        continue;
                    }
                    let envelope = (u * PI).sin().max(0.0);
                    let half_width = breadth * envelope.powf(1.2);
                    let lateral = -dx * sin + dy * cos - bend * envelope;
                    let coverage = ((half_width - lateral.abs()) * self.scale).clamp(0.0, 1.0);
                    if coverage == 0.0 {
                        continue;
                    }
                    let across = (lateral / half_width.max(0.00001)).clamp(-1.0, 1.0);
                    let ridge = (1.0 - across * across).sqrt();
                    let directional = (1.0 - (across + 0.38).abs()).max(0.0);
                    let light = (0.055 + ridge * 0.135 + directional * 0.105)
                        * (0.55 + 0.45 * envelope)
                        * coverage;
                    let pixel = self.image.get_pixel_mut(x, y);
                    for channel in 0..3 {
                        pixel[channel] = pixel[channel].max((color[channel] * light) as u8);
                    }
                }
            }
        }
        self.curve(76, color, width, |t| {
            let u = if t <= 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
            let side = if t <= 0.5 { 1.0 } else { -1.0 };
            let envelope = (PI * u).sin().max(0.0);
            let radial = start + (end - start) * u;
            let tangent = side * breadth * envelope.powf(1.2) + bend * envelope;
            (radial * cos - tangent * sin, radial * sin + tangent * cos)
        });
    }
}

fn polar(radius: f32, angle: f32) -> (f32, f32) {
    (radius * angle.cos(), radius * angle.sin())
}

fn ornament(
    ink: &mut Ink<'_>,
    palette: &[Color; 6],
    symmetry: u32,
    seed: u64,
    phase: f32,
    motion: f32,
) {
    let rotation = hash(seed, 51) * TAU;
    let color = gradient(palette, 0.68);
    for ring in [0.945, 0.962] {
        ink.circle((0.0, 0.0), ring, rgb_color(color, 0.5), 0.0014);
    }
    for index in 0..symmetry * 4 {
        let angle = rotation + index as f32 * TAU / (symmetry * 4) as f32;
        let radius = 0.909 + 0.006 * motion * (phase + angle * symmetry as f32).sin();
        ink.circle(
            polar(radius, angle),
            if index % 4 == 0 { 0.010 } else { 0.0035 },
            rgb_color(color, if index % 4 == 0 { 0.9 } else { 0.6 }),
            0.0017,
        );
    }
}

fn rgb_color(color: Color, light: f32) -> Color {
    color.map(|channel| channel * light)
}

pub(super) fn render_radial(scene: &Scene, image: &mut RgbImage, phase: f32) {
    let phase = loop_phase(phase);
    let palette = colors(&scene.palette);
    let min_dimension = image.width().min(image.height()) as f32;
    let scale = min_dimension * 0.48 * (scene.zoom as f32).sqrt();
    let center = (image.width() as f32 * 0.5, image.height() as f32 * 0.5);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let radius = ((x as f32 + 0.5 - center.0).powi(2) + (y as f32 + 0.5 - center.1).powi(2))
            .sqrt()
            / scale;
        let atmosphere = 0.22 + 0.24 * (-radius * radius * 2.8).exp();
        *pixel = rgb(palette[0], atmosphere);
    }
    let mut ink = Ink {
        image,
        scale,
        center,
    };
    match scene.kind {
        SceneKind::FlowerOfLife => flower_of_life(scene, &mut ink, &palette, phase),
        SceneKind::Phyllotaxis => phyllotaxis(scene, &mut ink, &palette, phase),
        SceneKind::RadialWeave => radial_weave(scene, &mut ink, &palette, phase),
        SceneKind::StarMandala => star_mandala(scene, &mut ink, &palette, phase),
        SceneKind::Kaleidoscope => kaleidoscope(scene, &mut ink, &palette, phase),
        _ => lotus(scene, &mut ink, &palette, phase),
    }
}

fn lotus(scene: &Scene, ink: &mut Ink<'_>, palette: &[Color; 6], phase: f32) {
    let symmetry = scene.symmetry;
    let motion = scene.motion;
    let seed_angle = hash(scene.seed, 71) * TAU;
    let lotus = matches!(scene.kind, SceneKind::LotusMandala);
    let layers = if lotus { 4 } else { 5 } + (hash(scene.seed, 74) * 3.0) as i32;
    let spread = 0.78 + hash(scene.seed, 72) * 0.4;
    ornament(ink, palette, symmetry, scene.seed, phase, motion);
    for layer in (0..layers).rev() {
        let l = layer as f32;
        let ratio = l / (layers - 1) as f32;
        let breathe = 1.0 + motion * 0.035 * (phase + l * 0.9).sin();
        let end = (0.23 + ratio * 0.65) * breathe;
        let start = if lotus { end * 0.26 } else { end * 0.48 };
        let spin = if layer % 2 == 0 { 1.0 } else { -1.0 };
        let turn =
            seed_angle + l * PI / symmetry as f32 + spin * motion * 0.12 * (phase + l * 0.55).sin();
        let breadth = end * PI / symmetry as f32 * spread;
        let bend = motion * breadth * 0.30 * (phase + l).sin();
        let color = gradient(palette, 0.36 + ratio * 0.39 + hash(scene.seed, 73) * 0.1);
        for petal in 0..symmetry {
            let angle = turn + petal as f32 * TAU / symmetry as f32;
            ink.petal(angle, (start, end), breadth, bend, color, 0.0021);
            ink.petal(
                angle,
                (start + (end - start) * 0.10, end * 0.965),
                breadth * 0.68,
                bend * 0.85,
                rgb_color(color, 0.56),
                0.0011,
            );
            if layer > 1 {
                ink.petal(
                    angle,
                    (end * 0.69, end * 0.91),
                    breadth * 0.24,
                    bend * 0.15,
                    gradient(palette, 0.67),
                    0.0016,
                );
                ink.circle(
                    polar(end * 0.77, angle),
                    0.005 + ratio * 0.002,
                    gradient(palette, 0.67),
                    0.0013,
                );
            }
        }
        if !lotus {
            ink.circle((0.0, 0.0), end * 0.76, rgb_color(color, 0.44), 0.0010);
        }
    }
    for (radius, light) in [(0.077, 0.8), (0.065, 0.52), (0.021, 1.0)] {
        ink.circle(
            (0.0, 0.0),
            radius,
            rgb_color(gradient(palette, 0.67), light),
            0.0018,
        );
    }
}

fn star_mandala(scene: &Scene, ink: &mut Ink<'_>, palette: &[Color; 6], phase: f32) {
    let n = scene.symmetry as f32;
    let seed = hash(scene.seed, 81) * TAU;
    let depth = 0.25 + hash(scene.seed, 82) * 0.15;
    ornament(
        ink,
        palette,
        scene.symmetry,
        scene.seed,
        phase,
        scene.motion,
    );
    let layers = 6 + (hash(scene.seed, 83) * 3.0) as i32;
    for layer in 0..layers {
        let l = layer as f32;
        let outer = 0.87 * 0.79_f32.powi(layer);
        let turn = seed + l * PI / n + scene.motion * 0.105 * (phase + l * 0.83).sin();
        let inner = outer * (1.0 - depth - 0.035 * scene.motion * (phase - l).cos());
        let color = gradient(palette, 0.32 + l * 0.065);
        for inset in [1.0, 0.962] {
            ink.curve(
                (scene.symmetry * 2) as usize,
                rgb_color(color, if inset == 1.0 { 1.0 } else { 0.46 }),
                if inset == 1.0 { 0.0021 } else { 0.0011 },
                |u| {
                    let index = (u * n * 2.0).round() as u32;
                    polar(
                        if index.is_multiple_of(2) {
                            outer * inset
                        } else {
                            inner * inset
                        },
                        turn + u * TAU,
                    )
                },
            );
        }
        for point in 0..scene.symmetry {
            let angle = turn + point as f32 * TAU / n;
            let a = polar(outer * 0.79, angle);
            let b = polar(outer * 0.92, angle);
            let c = polar(outer * 0.845, angle + 0.037);
            let d = polar(outer * 0.845, angle - 0.037);
            for (from, to) in [(a, c), (c, b), (b, d), (d, a)] {
                ink.line(from, to, rgb_color(gradient(palette, 0.66), 0.83), 0.0013);
            }
        }
    }
    ink.circle((0.0, 0.0), 0.05, gradient(palette, 0.68), 0.0018);
}

fn flower_of_life(scene: &Scene, ink: &mut Ink<'_>, palette: &[Color; 6], phase: f32) {
    let rings = 2 + (hash(scene.seed, 91) * 2.0) as i32;
    let cell = 0.82 / (rings as f32 + 1.0);
    let rotation = hash(scene.seed, 92) * TAU;
    let breathing = 1.0 + 0.035 * scene.motion * phase.sin();
    let (sin, cos) = rotation.sin_cos();
    ornament(ink, palette, 6, scene.seed, phase, scene.motion);
    for q in -rings..=rings {
        for r in -rings..=rings {
            let distance = q.abs().max(r.abs()).max((q + r).abs());
            if distance > rings {
                continue;
            }
            let x = cell * (q as f32 + r as f32 * 0.5) * breathing;
            let y = cell * r as f32 * 3.0_f32.sqrt() * 0.5 * breathing;
            let center = (x * cos - y * sin, x * sin + y * cos);
            let radius =
                cell * (1.0 + scene.motion * 0.027 * (phase + distance as f32 * 0.7).cos());
            let color = gradient(palette, 0.37 + distance as f32 * 0.085);
            ink.circle(center, radius, color, 0.0020);
            ink.circle(center, radius * 0.963, rgb_color(color, 0.38), 0.0009);
            // Develop the circle lattice itself: recessed rings expose its
            // interlocking cells at different depths. The detail control now
            // affects this family rather than requesting unused iterations.
            for rib in 1..=(scene.iterations / 90).clamp(1, 3) {
                let depth = rib as f32;
                ink.circle(
                    center,
                    radius * (0.94 - depth * 0.08),
                    rgb_color(
                        gradient(palette, 0.40 + distance as f32 * 0.07 + depth * 0.025),
                        0.55 + depth * 0.08,
                    ),
                    0.0010,
                );
            }
            ink.circle(center, 0.006, gradient(palette, 0.68), 0.0014);
        }
    }
    // A smaller seed-of-life rosette anchors the otherwise repeated lattice.
    for i in 0..6 {
        let angle = rotation + i as f32 * TAU / 6.0;
        ink.circle(
            polar(cell * 0.48, angle),
            cell * 0.48,
            rgb_color(gradient(palette, 0.68), 0.75),
            0.0011,
        );
    }
}

fn radial_weave(scene: &Scene, ink: &mut Ink<'_>, palette: &[Color; 6], phase: f32) {
    let n = scene.symmetry as f32;
    let seed = hash(scene.seed, 101) * TAU;
    let twist = 0.28 + hash(scene.seed, 102) * 0.35;
    let count = 16 + (hash(scene.seed, 103) * 9.0) as usize;
    ornament(
        ink,
        palette,
        scene.symmetry,
        scene.seed,
        phase,
        scene.motion,
    );
    for layer in 0..count {
        let l = layer as f32 / count as f32;
        let radius = 0.13 + l * 0.73;
        let turn = seed + twist * l + scene.motion * 0.12 * (phase + l * TAU).sin();
        let amplitude = (0.021 + l * 0.045) * (1.0 + scene.motion * 0.18 * (phase - l * PI).cos());
        let color = gradient(palette, 0.30 + l * 0.39);
        ink.curve(480, rgb_color(color, 0.9), 0.0016, |u| {
            let a = u * TAU;
            polar(radius + amplitude * (a * n + turn * n).cos(), a)
        });
    }
    for spoke in 0..scene.symmetry * 2 {
        let a = seed + spoke as f32 * TAU / (scene.symmetry * 2) as f32;
        let direction = if spoke.is_multiple_of(2) { 1.0 } else { -1.0 };
        ink.curve(120, rgb_color(gradient(palette, 0.67), 0.68), 0.0011, |u| {
            let radius = 0.13 + u * 0.73;
            let angle = a + direction * (twist * u + scene.motion * 0.09 * (phase + u * TAU).sin());
            polar(radius, angle)
        });
    }
}

fn kaleidoscope(scene: &Scene, ink: &mut Ink<'_>, palette: &[Color; 6], phase: f32) {
    let n = scene.symmetry as f32;
    let seed_angle = hash(scene.seed, 111) * TAU;
    let branches = 3 + (hash(scene.seed, 112) * 3.0) as usize;
    ornament(
        ink,
        palette,
        scene.symmetry,
        scene.seed,
        phase,
        scene.motion,
    );
    // Mirrored florets inside mirrored wedges: the smaller construction repeats
    // at each parent tip, producing actual recursive visual detail.
    for level in 0..3 {
        let scale = 0.76 * 0.53_f32.powi(level);
        let turn = seed_angle
            + level as f32 * PI / n
            + scene.motion * 0.10 * (phase + level as f32 * 1.7).sin();
        for wedge in 0..scene.symmetry {
            let angle = turn + wedge as f32 * TAU / n;
            let color = gradient(palette, 0.36 + level as f32 * 0.115);
            ink.petal(
                angle,
                (scale * 0.26, scale * 1.13),
                scale * PI / n * 0.66,
                0.0,
                color,
                0.002,
            );
            for mirror in [-1.0_f32, 1.0] {
                for child in 0..branches {
                    let u = child as f32 / branches as f32;
                    let child_angle = angle + mirror * (0.15 + u * 0.7) * PI / n;
                    ink.petal(
                        child_angle,
                        (scale * (0.38 + 0.15 * u), scale * (0.96 - u * 0.18)),
                        scale * 0.045,
                        mirror * scale * 0.015 * scene.motion * phase.sin(),
                        rgb_color(gradient(palette, 0.51 + u * 0.18), 0.82),
                        0.0011,
                    );
                    ink.circle(
                        polar(scale * (0.77 - u * 0.14), child_angle),
                        scale * 0.009,
                        gradient(palette, 0.68),
                        0.0012,
                    );
                }
            }
        }
    }
    ink.circle((0.0, 0.0), 0.04, gradient(palette, 0.68), 0.0017);
}

fn phyllotaxis(scene: &Scene, ink: &mut Ink<'_>, palette: &[Color; 6], phase: f32) {
    let count =
        ((scene.iterations as f32 * (1.3 + hash(scene.seed, 123) * 1.8)) as u32).clamp(180, 850);
    let golden = PI * (3.0 - 5.0_f32.sqrt());
    let offset = hash(scene.seed, 121) * TAU;
    let spread = 0.78 + hash(scene.seed, 122) * 0.28;
    for point in (0..count).rev() {
        let u = (point as f32 + 0.5) / count as f32;
        let radius = u.sqrt() * 0.89 * (1.0 + scene.motion * 0.018 * (phase + u * TAU).sin());
        let angle =
            point as f32 * golden + offset + scene.motion * 0.075 * (phase + radius * 5.0).sin();
        let center = polar(radius, angle);
        let length = (0.014 + u * 0.011) * (650.0 / count as f32).sqrt() * spread;
        let orientation = angle + 0.42 + scene.motion * 0.18 * (phase - u * 4.0).sin();
        let (sin, cos) = orientation.sin_cos();
        let color = gradient(palette, 0.33 + u * 0.4);
        ink.curve(20, color, 0.0015, |t| {
            let theta = t * TAU;
            let x = length * theta.cos();
            let y = length * 0.60 * theta.sin();
            (center.0 + x * cos - y * sin, center.1 + x * sin + y * cos)
        });
        if point % 3 == 0 {
            ink.line(
                (
                    center.0 - length * 0.45 * cos,
                    center.1 - length * 0.45 * sin,
                ),
                (
                    center.0 + length * 0.45 * cos,
                    center.1 + length * 0.45 * sin,
                ),
                rgb_color(gradient(palette, 0.68), 0.72),
                0.0010,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(kind: SceneKind, seed: u64, phase: f32, motion: f32) -> RgbImage {
        let scene = Scene {
            kind,
            seed,
            width: 96,
            height: 96,
            iterations: 90,
            motion,
            ..Scene::default()
        };
        let mut image = RgbImage::new(scene.width, scene.height);
        match kind {
            SceneKind::Mandelbrot | SceneKind::Julia => render_escape(&scene, &mut image, phase),
            SceneKind::Newton => render_newton(&scene, &mut image, phase),
            _ => render_radial(&scene, &mut image, phase),
        }
        image
    }

    #[test]
    fn animated_constructions_close_the_loop_and_change_in_between() {
        for kind in [
            SceneKind::Julia,
            SceneKind::Mandelbrot,
            SceneKind::Newton,
            SceneKind::Mandala,
            SceneKind::LotusMandala,
            SceneKind::StarMandala,
            SceneKind::FlowerOfLife,
            SceneKind::RadialWeave,
            SceneKind::Kaleidoscope,
            SceneKind::Phyllotaxis,
        ] {
            let first = frame(kind, 97, 0.0, 0.8);
            assert_eq!(first, frame(kind, 97, TAU, 0.8), "loop seam: {kind:?}");
            let middle = frame(kind, 97, 1.4, 0.8);
            let changed = first
                .pixels()
                .zip(middle.pixels())
                .filter(|(a, b)| a != b)
                .count();
            assert!(
                changed > 150,
                "no meaningful geometric movement: {kind:?} ({changed})"
            );
            assert_eq!(
                frame(kind, 97, 0.0, 0.0),
                frame(kind, 97, 1.4, 0.0),
                "motion=0 must remain still: {kind:?}"
            );
        }
    }

    #[test]
    fn seeds_invent_different_compositions() {
        for kind in [
            SceneKind::Julia,
            SceneKind::Mandelbrot,
            SceneKind::Mandala,
            SceneKind::FlowerOfLife,
            SceneKind::Kaleidoscope,
            SceneKind::Phyllotaxis,
        ] {
            let first = frame(kind, 97, 0.0, 0.7);
            let second = frame(kind, 221, 0.0, 0.7);
            let difference: usize = first
                .as_raw()
                .iter()
                .zip(second.as_raw())
                .map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs() as usize)
                .sum();
            assert!(difference > 10_000, "seed has little effect: {kind:?}");
        }
    }

    #[test]
    fn radial_geometry_keeps_its_aspect_ratio() {
        let square = Scene {
            width: 96,
            height: 96,
            kind: SceneKind::LotusMandala,
            ..Scene::default()
        };
        let wide = Scene {
            width: 144,
            ..square.clone()
        };
        let mut a = RgbImage::new(96, 96);
        let mut b = RgbImage::new(144, 96);
        render_radial(&square, &mut a, 0.0);
        render_radial(&wide, &mut b, 0.0);
        for y in 0..96 {
            for x in 0..96 {
                let first = a.get_pixel(x, y);
                let second = b.get_pixel(x + 24, y);
                assert!(
                    first
                        .0
                        .iter()
                        .zip(second.0)
                        .all(|(v, w)| (*v as i16 - w as i16).abs() <= 1)
                );
            }
        }
    }
}
