//! Bounded Fractus scene DSL, raster renderer, and render verification.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, DynamicImage, Frame, Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod kinetic;
mod orbits;
mod richness;
pub use richness::VisualRichness;

const MAX_DIMENSION: u32 = 2_048;
const MAX_ITERATIONS: u32 = 2_000;
pub const MAX_ANIMATION_FRAMES: u16 = 240;
pub const MAX_ANIMATION_PIXELS: u64 = 96_000_000;
const MAX_ANIMATION_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneKind {
    Mandelbrot,
    Julia,
    Multibrot,
    BurningShip,
    Tricorn,
    Newton,
    Mandala,
    LotusMandala,
    StarMandala,
    FlowerOfLife,
    RadialWeave,
    Kaleidoscope,
    Phyllotaxis,
    WovenWeb,
    Guilloche,
    Lissajous,
    Moire,
    OrbitalLace,
    Spirograph,
    Harmonograph,
    RoseCurve,
    StringArt,
    Sierpinski,
    KochSnowflake,
    FractalTree,
    DragonCurve,
    BarnsleyFern,
    LSystem,
    Truchet,
    HexWeave,
    OpArt,
    CellularAutomata,
    ReactionDiffusion,
    Particles,
    FlowField,
    StrangeAttractor,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    Normal,
    Add,
    Screen,
    Multiply,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SceneLayer {
    pub kind: SceneKind,
    pub alpha: f32,
    pub blend: BlendMode,
    pub palette: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Scene {
    pub version: u8,
    pub kind: SceneKind,
    pub width: u32,
    pub height: u32,
    pub iterations: u32,
    pub seed: u64,
    pub palette: String,
    #[serde(default)]
    pub layers: Vec<SceneLayer>,
    #[serde(default = "default_symmetry")]
    pub symmetry: u32,
    #[serde(default = "default_zoom")]
    pub zoom: f64,
    #[serde(default = "default_motion")]
    pub motion: f32,
    /// Layer names the Artist asked for that this renderer does not know.
    ///
    /// Recorded on the scene so the manifest shows what was requested and not
    /// drawn, instead of the difference disappearing between the request and
    /// the picture.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknown_layers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RenderManifest {
    pub scene: Scene,
    pub image: PathBuf,
    pub non_black_pixels: u64,
    pub verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<AnimationMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_richness: Option<VisualRichness>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnimationMetadata {
    pub frame_count: u16,
    pub fps: u16,
    pub loop_seconds: f64,
    pub distinct_frames: usize,
    pub mean_frame_delta: f64,
    pub seam_delta: f64,
}

fn default_symmetry() -> u32 {
    12
}
fn default_zoom() -> f64 {
    1.0
}
fn default_motion() -> f32 {
    0.65
}

#[derive(Debug, Error)]
pub enum FractusError {
    #[error("invalid Fractus scene: {0}")]
    Invalid(String),
    #[error("Fractus render failed: {0}")]
    Image(#[from] image::ImageError),
    #[error("Fractus manifest failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Fractus manifest JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Fractus animation encoding failed: {0}")]
    Png(#[from] png::EncodingError),
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            version: 2,
            kind: SceneKind::LotusMandala,
            width: 900,
            height: 1_200,
            iterations: 420,
            seed: 0x07e1_ed2a,
            palette: "emerald".to_owned(),
            layers: Vec::new(),
            symmetry: default_symmetry(),
            zoom: default_zoom(),
            motion: default_motion(),
            unknown_layers: Vec::new(),
        }
    }
}

impl Scene {
    pub fn parse(dsl: &str) -> Result<Self, FractusError> {
        let mut fields = HashMap::new();
        for line in dsl_fields(dsl) {
            let line = line.as_str();
            if let Some(version) = line.strip_prefix("version ") {
                fields.insert("version".to_owned(), version.trim().to_owned());
                continue;
            }
            let (key, value) = line
                .split_once(['=', ':'])
                .ok_or_else(|| FractusError::Invalid(format!("unknown DSL line '{line}'")))?;
            fields.insert(key.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
        let mut scene = Self::default();
        if let Some(value) = fields.get("version") {
            scene.version = parse(value, "version")?;
        }
        if let Some(value) = fields.get("type").or_else(|| fields.get("kind")) {
            scene.kind = match value.to_ascii_lowercase().replace('-', "_").as_str() {
                "mandelbrot" | "fractal" => SceneKind::Mandelbrot,
                "julia" => SceneKind::Julia,
                "multibrot" => SceneKind::Multibrot,
                "burning_ship" => SceneKind::BurningShip,
                "tricorn" => SceneKind::Tricorn,
                "newton" => SceneKind::Newton,
                "mandala" => SceneKind::Mandala,
                "lotus_mandala" | "fractal_flower" => SceneKind::LotusMandala,
                "star_mandala" => SceneKind::StarMandala,
                "flower_of_life" => SceneKind::FlowerOfLife,
                "radial_weave" => SceneKind::RadialWeave,
                "kaleidoscope" => SceneKind::Kaleidoscope,
                "phyllotaxis" => SceneKind::Phyllotaxis,
                "woven_web" => SceneKind::WovenWeb,
                "guilloche" => SceneKind::Guilloche,
                "lissajous" => SceneKind::Lissajous,
                "moire" => SceneKind::Moire,
                "orbital_lace" => SceneKind::OrbitalLace,
                // The spirograph renderer uses the rolling-inside-circle
                // hypotrochoid equations, including the minus sign in y.
                "spirograph" | "hypotrochoid" => SceneKind::Spirograph,
                "harmonograph" => SceneKind::Harmonograph,
                "rose_curve" => SceneKind::RoseCurve,
                "string_art" => SceneKind::StringArt,
                "sierpinski" | "sierpinski_triangle" => SceneKind::Sierpinski,
                "koch" | "koch_snowflake" => SceneKind::KochSnowflake,
                "fractal_tree" => SceneKind::FractalTree,
                "dragon_curve" => SceneKind::DragonCurve,
                "barnsley_fern" => SceneKind::BarnsleyFern,
                "l_system" | "lsystem" | "lindenmayer" => SceneKind::LSystem,
                "truchet" => SceneKind::Truchet,
                "hex_weave" => SceneKind::HexWeave,
                "op_art" => SceneKind::OpArt,
                "cellular" | "cellular_automaton" | "cellular_automata" => {
                    SceneKind::CellularAutomata
                }
                "reaction_diffusion" => SceneKind::ReactionDiffusion,
                "particles" | "particle" => SceneKind::Particles,
                "flow" | "flow_field" | "flowfield" => SceneKind::FlowField,
                "strange_attractor" | "lorenz" => SceneKind::StrangeAttractor,
                _ => {
                    return Err(FractusError::Invalid(format!(
                        "unknown scene type '{value}'"
                    )));
                }
            };
        }
        if let Some(value) = fields.get("width") {
            scene.width = parse(value, "width")?;
        }
        if let Some(value) = fields.get("height") {
            scene.height = parse(value, "height")?;
        }
        if let Some(value) = fields.get("iterations") {
            scene.iterations = parse(value, "iterations")?;
        }
        if let Some(value) = fields.get("seed") {
            scene.seed = parse(value, "seed")?;
        }
        // Out-of-range style numbers are clamped rather than refused: a
        // symmetry of 4 or a zoom of 12 is a clear intent, and refusing it
        // cost the court another model call to ask for the same scene again.
        // Non-finite values still fail validation.
        if let Some(value) = fields.get("symmetry") {
            scene.symmetry = parse::<u32>(value, "symmetry")?.clamp(6, 24);
        }
        if let Some(value) = fields.get("zoom") {
            let zoom: f64 = parse(value, "zoom")?;
            scene.zoom = if zoom.is_finite() {
                zoom.clamp(0.5, 8.0)
            } else {
                zoom
            };
        }
        if let Some(value) = fields.get("motion") {
            let motion: f32 = parse(value, "motion")?;
            scene.motion = if motion.is_finite() {
                motion.clamp(0.0, 1.0)
            } else {
                motion
            };
        }
        if let Some(value) = fields.get("palette") {
            if !is_known_palette(value) {
                return Err(FractusError::Invalid(format!(
                    "unknown palette '{}'; choose one of {}",
                    value.chars().take(60).collect::<String>(),
                    PALETTE_NAMES.join(", ")
                )));
            }
            scene.palette = value.trim().to_ascii_lowercase();
        }
        if let Some(value) = fields.get("layers") {
            let (layers, skipped) = parse_layers(value, &scene.palette)?;
            // Without an explicit type the base took the default, so a request
            // for a kaleidoscope over a spirograph still rendered particles
            // underneath everything. The first layer is the subject; it should
            // be what the scene is.
            if !fields.contains_key("type")
                && !fields.contains_key("kind")
                && let Some(first) = layers.first()
            {
                scene.kind = first.kind;
            }
            scene.layers = layers;
            scene.unknown_layers = skipped;
        }
        scene.validate()?;
        Ok(scene)
    }

    pub fn validate(&self) -> Result<(), FractusError> {
        if self.version != 2 {
            return Err(FractusError::Invalid(
                "only DSL version 2 is supported".to_owned(),
            ));
        }
        if self.width < 64
            || self.height < 64
            || self.width > MAX_DIMENSION
            || self.height > MAX_DIMENSION
        {
            return Err(FractusError::Invalid(format!(
                "dimensions must be 64..={MAX_DIMENSION}"
            )));
        }
        if self.iterations == 0 || self.iterations > MAX_ITERATIONS {
            return Err(FractusError::Invalid(format!(
                "iterations must be 1..={MAX_ITERATIONS}"
            )));
        }
        if self.layers.len() > 8 {
            return Err(FractusError::Invalid(
                "a scene supports at most eight compositing layers".to_owned(),
            ));
        }
        if !(6..=24).contains(&self.symmetry)
            || !self.zoom.is_finite()
            || !(0.5..=8.0).contains(&self.zoom)
            || !self.motion.is_finite()
            || !(0.0..=1.0).contains(&self.motion)
        {
            return Err(FractusError::Invalid(
                "symmetry must be 6..24, zoom 0.5..8, and motion 0..1; all values must be finite"
                    .to_owned(),
            ));
        }
        if !is_known_palette(&self.palette)
            || self
                .layers
                .iter()
                .any(|layer| !is_known_palette(&layer.palette))
        {
            return Err(FractusError::Invalid(
                "scene and layer palettes must use a supported name".to_owned(),
            ));
        }
        for layer in &self.layers {
            if !layer.alpha.is_finite() || !(0.0..=1.0).contains(&layer.alpha) {
                return Err(FractusError::Invalid(
                    "layer alpha must be finite and between 0 and 1".to_owned(),
                ));
            }
        }
        Ok(())
    }

    pub fn render(&self, output: &Path) -> Result<RenderManifest, FractusError> {
        self.validate()?;
        let image = self.render_frame(0.0);
        image.save(output)?;
        verify(self.clone(), output.to_path_buf(), &image)
    }

    /// Preview the actual subject before a costly animation render. The court
    /// uses this explicit operator preference; primitive renders remain usable
    /// through the general renderer for callers that deliberately request them.
    pub fn require_visual_richness(&self) -> Result<VisualRichness, FractusError> {
        self.validate()?;
        let facts = richness::measure(&self.render_frame(0.0));
        let concerns = facts.concerns();
        if concerns.is_empty() {
            Ok(facts)
        } else {
            Err(FractusError::Invalid(format!(
                "visual richness check: {}. Develop visible internal structure, tonal depth and framing within this one family; a larger image, motion-only change or unrelated overlays cannot satisfy this check",
                concerns.join("; ")
            )))
        }
    }

    /// Lossless true-colour animation: no per-frame GIF palette quantization.
    /// Frames are encoded one at a time and every frame is verified before the
    /// artifact can be published. The Studio's decode budget uses the same cap.
    pub fn render_animation(
        &self,
        output: &Path,
        frame_count: u16,
        fps: u16,
    ) -> Result<RenderManifest, FractusError> {
        self.validate()?;
        validate_animation_budget(self, frame_count)?;
        if !(12..=30).contains(&fps) {
            return Err(FractusError::Invalid(
                "animation frame rate must be 12..30".to_owned(),
            ));
        }
        let first = self.render_frame(0.0);
        let mut manifest = verify(self.clone(), output.to_path_buf(), &first)?;
        let result = (|| -> Result<AnimationMetadata, FractusError> {
            let file = BufWriter::new(File::create(output)?);
            let mut encoder = png::Encoder::new(file, self.width, self.height);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Fast);
            encoder.set_animated(u32::from(frame_count), 0)?;
            encoder.set_frame_delay(1, fps)?;
            encoder.set_blend_op(png::BlendOp::Source)?;
            encoder.set_dispose_op(png::DisposeOp::None)?;
            let mut writer = encoder.write_header()?;
            writer.write_image_data(first.as_raw())?;
            let mut previous = first.clone();
            let mut distinct = std::collections::HashSet::from([frame_digest(&first)]);
            let mut delta = 0.0;
            for index in 1..frame_count {
                let phase = index as f32 / frame_count as f32 * std::f32::consts::TAU;
                let frame = self.render_frame(phase);
                verify(self.clone(), output.to_path_buf(), &frame)?;
                delta += frame_delta(&previous, &frame);
                distinct.insert(frame_digest(&frame));
                writer.write_image_data(frame.as_raw())?;
                previous = frame;
            }
            writer.finish()?;
            if std::fs::metadata(output)?.len() > MAX_ANIMATION_BYTES {
                return Err(FractusError::Invalid(
                    "encoded animation exceeds the Studio's 128 MiB file limit".to_owned(),
                ));
            }
            let seam_delta = frame_delta(&previous, &first);
            if self.motion > 0.0 && distinct.len() < 2 {
                return Err(FractusError::Invalid(
                    "animated scene produced identical frames".to_owned(),
                ));
            }
            Ok(AnimationMetadata {
                frame_count,
                fps,
                loop_seconds: f64::from(frame_count) / f64::from(fps),
                distinct_frames: distinct.len(),
                mean_frame_delta: (delta + seam_delta) / f64::from(frame_count),
                seam_delta,
            })
        })();
        match result {
            Ok(animation) => {
                manifest.animation = Some(animation);
                Ok(manifest)
            }
            Err(error) => {
                let _ = std::fs::remove_file(output);
                Err(error)
            }
        }
    }

    /// Renders a bounded, looping live preview suitable for projection or OBS.
    pub fn render_gif(
        &self,
        output: &Path,
        frame_count: u16,
    ) -> Result<RenderManifest, FractusError> {
        self.validate()?;
        validate_animation_budget(self, frame_count)?;
        if !(2..=120).contains(&frame_count) {
            return Err(FractusError::Invalid(
                "live frame count must be 2..=120".to_owned(),
            ));
        }
        let first = self.render_frame(0.0);
        let manifest = verify(self.clone(), output.to_path_buf(), &first)?;
        let file = BufWriter::new(File::create(output)?);
        let mut encoder = GifEncoder::new(file);
        encoder.set_repeat(Repeat::Infinite)?;
        for index in 0..frame_count {
            let phase = index as f32 / frame_count as f32 * std::f32::consts::TAU;
            let rgb = if index == 0 {
                first.clone()
            } else {
                self.render_frame(phase)
            };
            let rgba = DynamicImage::ImageRgb8(rgb).into_rgba8();
            encoder.encode_frame(Frame::from_parts(
                rgba,
                0,
                0,
                Delay::from_numer_denom_ms(80, 1),
            ))?;
        }
        Ok(manifest)
    }

    fn render_frame(&self, phase: f32) -> RgbImage {
        let mut image = RgbImage::new(self.width, self.height);
        self.render_single(&mut image, phase);
        for layer in &self.layers {
            let mut overlay = RgbImage::new(self.width, self.height);
            let mut scene = self.clone();
            scene.kind = layer.kind;
            scene.palette = layer.palette.clone();
            scene.layers.clear();
            scene.render_single(&mut overlay, phase);
            composite(&mut image, &overlay, layer.alpha, layer.blend);
        }
        image
    }

    fn render_single(&self, image: &mut RgbImage, phase: f32) {
        match self.kind {
            SceneKind::Mandelbrot
            | SceneKind::Julia
            | SceneKind::Multibrot
            | SceneKind::BurningShip
            | SceneKind::Tricorn => kinetic::render_escape(self, image, phase),
            SceneKind::Newton => kinetic::render_newton(self, image, phase),
            SceneKind::Mandala
            | SceneKind::LotusMandala
            | SceneKind::StarMandala
            | SceneKind::FlowerOfLife
            | SceneKind::RadialWeave
            | SceneKind::Kaleidoscope
            | SceneKind::Phyllotaxis => kinetic::render_radial(self, image, phase),
            SceneKind::Guilloche
            | SceneKind::Spirograph
            | SceneKind::OrbitalLace
            | SceneKind::RoseCurve => orbits::render(self, image, phase),
            SceneKind::WovenWeb
            | SceneKind::Lissajous
            | SceneKind::Moire
            | SceneKind::Harmonograph
            | SceneKind::StringArt => render_curves(self, image, phase.sin() * self.motion),
            SceneKind::Sierpinski
            | SceneKind::KochSnowflake
            | SceneKind::FractalTree
            | SceneKind::DragonCurve
            | SceneKind::BarnsleyFern
            | SceneKind::LSystem => render_recursive(self, image, phase.sin() * self.motion),
            SceneKind::Truchet | SceneKind::HexWeave | SceneKind::OpArt => {
                render_tiles(self, image, phase.sin() * self.motion)
            }
            SceneKind::CellularAutomata | SceneKind::ReactionDiffusion => {
                render_cells(self, image, phase.sin() * self.motion)
            }
            SceneKind::Particles => render_particles(self, image, phase.sin() * self.motion),
            SceneKind::FlowField => render_flow(self, image, phase.sin() * self.motion),
            SceneKind::StrangeAttractor => render_attractor(self, image, phase.sin() * self.motion),
        }
    }
}

/// The DSL with one field per line, as every line-based consumer expects.
pub fn canonical_dsl(dsl: &str) -> String {
    dsl_fields(dsl).join("\n")
}

/// Keep the selected subject and its parameters, without compositing other
/// families. The gallery uses this policy; `Scene::parse` and the renderer
/// still support deliberately authored composite scenes for other callers.
pub fn single_family_dsl(dsl: &str) -> Result<String, FractusError> {
    let scene = Scene::parse(dsl)?;
    let fields = dsl_fields(dsl);
    let has_subject = fields.iter().any(|field| {
        field.split_once(['=', ':']).is_some_and(|(key, _)| {
            matches!(key.trim().to_ascii_lowercase().as_str(), "type" | "kind")
        })
    });
    let mut fields = fields
        .into_iter()
        .filter(|field| {
            !field
                .split_once(['=', ':'])
                .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("layers"))
        })
        .collect::<Vec<_>>();
    if !has_subject && let Some(subject) = scene.layers.first() {
        // A layers-only request names its subject first. Promote that exact
        // family and palette instead of replacing it with the scene default.
        let kind = serde_json::to_value(subject.kind)?;
        fields.push(format!("type={}", kind.as_str().unwrap_or_default()));
        fields.push(format!("palette={}", subject.palette));
    }
    Ok(fields.join("\n"))
}

pub fn write_manifest(path: &Path, manifest: &RenderManifest) -> Result<(), FractusError> {
    std::fs::write(path, serde_json::to_vec_pretty(manifest)?)?;
    Ok(())
}

fn validate_animation_budget(scene: &Scene, frame_count: u16) -> Result<(), FractusError> {
    if !(2..=MAX_ANIMATION_FRAMES).contains(&frame_count)
        || u64::from(scene.width) * u64::from(scene.height) * u64::from(frame_count)
            > MAX_ANIMATION_PIXELS
    {
        return Err(FractusError::Invalid(format!(
            "animation must have 2..{MAX_ANIMATION_FRAMES} frames and at most {MAX_ANIMATION_PIXELS} decoded pixels"
        )));
    }
    Ok(())
}

fn frame_digest(frame: &RgbImage) -> u64 {
    frame
        .as_raw()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
        })
}

fn frame_delta(first: &RgbImage, second: &RgbImage) -> f64 {
    first
        .as_raw()
        .iter()
        .zip(second.as_raw())
        .map(|(a, b)| f64::from(a.abs_diff(*b)))
        .sum::<f64>()
        / (first.as_raw().len() as f64 * 255.0)
}

/// One DSL field per entry, whether fields arrive on lines or after `;`.
///
/// The Artist's profile teaches the one-line form, `version 2; type=julia;
/// ...`, and the model writes exactly that. Read line by line, the whole line
/// was the version field, so every tagged composition failed with "version is
/// not a valid number" and the prose fallback chose the art instead: a
/// Lissajous knot the Artist had only mentioned in passing was drawn over the
/// mandala it asked for. `;` also separates entries inside `layers=`, so a
/// piece without a key of its own continues the layer stack before it.
fn dsl_fields(dsl: &str) -> Vec<String> {
    let mut fields: Vec<String> = Vec::new();
    for raw in dsl.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut in_layers = false;
        for piece in line.split(';').map(str::trim) {
            if piece.is_empty() {
                continue;
            }
            let keyed = piece.starts_with("version") || piece.contains(['=', ':']);
            if in_layers
                && !keyed
                && let Some(layers) = fields.last_mut()
            {
                layers.push(';');
                layers.push_str(piece);
                continue;
            }
            in_layers = piece
                .split_once(['=', ':'])
                .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("layers"));
            fields.push(piece.to_owned());
        }
    }
    fields
}

fn parse<T>(value: &str, name: &str) -> Result<T, FractusError>
where
    T: std::str::FromStr,
{
    value
        .parse()
        .map_err(|_| FractusError::Invalid(format!("{name} is not a valid number")))
}

/// Parses a layer stack, keeping the layers that are valid.
///
/// The Artist invents family names. Rejecting the whole stack because one of
/// them is unknown discards the layers that were correct, and the renderer then
/// falls back to its default scene — so a request for a kaleidoscope over a
/// spirograph produced the same emerald particles as every other request. The
/// names that do not resolve are reported instead, and the rest still render.
fn parse_layers(
    value: &str,
    default_palette: &str,
) -> Result<(Vec<SceneLayer>, Vec<String>), FractusError> {
    let mut layers = Vec::new();
    let mut skipped = Vec::new();
    for layer in value.split(';').filter(|layer| !layer.trim().is_empty()) {
        match parse_layer(layer, default_palette) {
            Ok(parsed) => layers.push(parsed),
            Err(_) => {
                let name = layer.split('@').next().unwrap_or(layer).trim();
                if !name.is_empty() {
                    skipped.push(name.to_owned());
                }
            }
        }
    }
    if layers.is_empty() && !skipped.is_empty() {
        return Err(FractusError::Invalid(format!(
            "no usable layers in '{}'",
            skipped.join(", ")
        )));
    }
    Ok((layers, skipped))
}

fn parse_layer(layer: &str, default_palette: &str) -> Result<SceneLayer, FractusError> {
    {
        let parts = layer.split('@').map(str::trim).collect::<Vec<_>>();
        let kind = Scene::parse(&format!(
            "version 2\ntype={}",
            parts.first().copied().unwrap_or_default()
        ))?
        .kind;
        // The Artist writes alphas like "3" and "4". Clamping keeps the layer
        // it asked for; refusing the number discarded the whole stack and left
        // the default scene in its place.
        let alpha = parts
            .get(1)
            .copied()
            .unwrap_or("0.5")
            .parse::<f32>()
            .map_err(|_| FractusError::Invalid("layer alpha is not a number".to_owned()))?
            .clamp(0.05, 1.0);
        let blend = match parts
            .get(2)
            .copied()
            .unwrap_or("screen")
            .to_ascii_lowercase()
            .as_str()
        {
            "normal" => BlendMode::Normal,
            "add" | "plus" => BlendMode::Add,
            "screen" => BlendMode::Screen,
            "multiply" => BlendMode::Multiply,
            value => {
                return Err(FractusError::Invalid(format!(
                    "unknown layer blend '{value}'"
                )));
            }
        };
        Ok(SceneLayer {
            kind,
            alpha,
            blend,
            palette: parts
                .get(3)
                .copied()
                .filter(|name| is_known_palette(name))
                .unwrap_or(default_palette)
                .trim()
                .to_ascii_lowercase(),
        })
    }
}

fn composite(base: &mut RgbImage, overlay: &RgbImage, alpha: f32, blend: BlendMode) {
    for (base_pixel, overlay_pixel) in base.pixels_mut().zip(overlay.pixels()) {
        for channel in 0..3 {
            let bottom = base_pixel[channel] as f32 / 255.0;
            let top = overlay_pixel[channel] as f32 / 255.0;
            let blended = match blend {
                BlendMode::Normal => top,
                BlendMode::Add => (bottom + top).min(1.0),
                BlendMode::Screen => 1.0 - (1.0 - bottom) * (1.0 - top),
                BlendMode::Multiply => bottom * top,
            };
            base_pixel[channel] = ((bottom * (1.0 - alpha) + blended * alpha) * 255.0) as u8;
        }
    }
}

fn verify(scene: Scene, path: PathBuf, image: &RgbImage) -> Result<RenderManifest, FractusError> {
    let non_black_pixels = image
        .pixels()
        .filter(|pixel| pixel.0.iter().any(|channel| *channel > 4))
        .count() as u64;
    let minimum = u64::from(image.width()) * u64::from(image.height()) / 200;
    let bright_pixels = image
        .pixels()
        .filter(|pixel| pixel.0.iter().any(|channel| *channel > 24))
        .count() as u64;
    if non_black_pixels < minimum || bright_pixels < minimum {
        return Err(FractusError::Invalid(
            "render verification found an effectively blank or unlit image".to_owned(),
        ));
    }
    Ok(RenderManifest {
        scene,
        image: path,
        non_black_pixels,
        verified: true,
        animation: None,
        visual_richness: Some(richness::measure(image)),
    })
}

fn render_curves(scene: &Scene, image: &mut RgbImage, phase: f32) {
    let palette = palette(&scene.palette);
    if matches!(scene.kind, SceneKind::Moire) {
        for y in 0..scene.height {
            for x in 0..scene.width {
                let nx = x as f32 / scene.width as f32 * 2.0 - 1.0;
                let ny = y as f32 / scene.height as f32 * 2.0 - 1.0;
                let first = ((nx + ny * 0.16 + phase * 0.05) * 45.0).sin();
                let second = ((nx * phase.cos() - ny * phase.sin()) * 46.5).sin();
                let value = (first + second).abs() * 0.5;
                if value > 0.72 {
                    image.put_pixel(x, y, shade(palette, value));
                }
            }
        }
        return;
    }
    let samples = scene.iterations.saturating_mul(12).clamp(1_200, 24_000);
    let scale = scene.width.min(scene.height) as f32 * 0.43;
    let center_x = scene.width as f32 * 0.5;
    let center_y = scene.height as f32 * 0.5;
    let mut previous = None;
    for sample in 0..samples {
        let t = sample as f32 / samples as f32 * std::f32::consts::TAU * 8.0 + phase;
        let (mut x, mut y) = match scene.kind {
            SceneKind::Guilloche | SceneKind::Spirograph => {
                let ratio = if matches!(scene.kind, SceneKind::Guilloche) {
                    6.2
                } else {
                    5.4
                };
                let offset = if matches!(scene.kind, SceneKind::Guilloche) {
                    0.42
                } else {
                    0.62
                };
                let radius = 1.0 - offset;
                (
                    radius * t.cos() + offset * (t * ratio).cos(),
                    radius * t.sin() - offset * (t * ratio).sin(),
                )
            }
            SceneKind::Harmonograph => {
                let decay = (-0.018 * t).exp();
                ((3.01 * t).sin() * decay, (2.0 * t + 1.2).sin() * decay)
            }
            SceneKind::StringArt => {
                let alternate = (sample * 37 % samples) as f32 / samples as f32;
                ((t).cos(), (alternate * std::f32::consts::TAU).sin())
            }
            SceneKind::OrbitalLace => {
                let radius = 0.62 + (7.0 * t).sin() * 0.28;
                (
                    radius * (t + (3.0 * t).sin() * 0.12).cos(),
                    radius * (t + (5.0 * t).cos() * 0.12).sin(),
                )
            }
            SceneKind::WovenWeb => (
                (5.0 * t).sin() * 0.82 + (12.0 * t).sin() * 0.16,
                (7.0 * t).sin() * 0.82 + (9.0 * t).cos() * 0.16,
            ),
            _ => ((5.0 * t).sin(), (8.0 * t + 0.7).sin()),
        };
        let maximum = x.abs().max(y.abs()).max(1.0);
        x /= maximum;
        y /= maximum;
        let point = (center_x + x * scale, center_y + y * scale);
        if let Some((old_x, old_y)) = previous {
            draw_line(
                image,
                old_x,
                old_y,
                point.0,
                point.1,
                shade(palette, sample as f32 / samples as f32),
            );
        }
        previous = Some(point);
    }
}

fn render_recursive(scene: &Scene, image: &mut RgbImage, phase: f32) {
    let palette = palette(&scene.palette);
    match scene.kind {
        SceneKind::FractalTree | SceneKind::LSystem => {
            let depth = 9;
            draw_branch(
                image,
                scene.width as f32 * 0.5,
                scene.height as f32 * 0.94,
                scene.height as f32 * 0.22,
                -std::f32::consts::FRAC_PI_2 + phase * 0.03,
                depth,
                palette,
            );
        }
        SceneKind::KochSnowflake => {
            let margin = scene.width.min(scene.height) as f32 * 0.12;
            let radius = scene.width.min(scene.height) as f32 * 0.4;
            let center = (scene.width as f32 * 0.5, scene.height as f32 * 0.52);
            let points = (0..3)
                .map(|index| {
                    let angle = phase * 0.03 - std::f32::consts::FRAC_PI_2
                        + index as f32 * std::f32::consts::TAU / 3.0;
                    (
                        center.0 + angle.cos() * radius,
                        center.1 + angle.sin() * radius,
                    )
                })
                .collect::<Vec<_>>();
            let depth = if margin > 0.0 { 5 } else { 4 };
            for index in 0..3 {
                draw_koch(
                    image,
                    points[index],
                    points[(index + 1) % 3],
                    depth,
                    shade(palette, index as f32 / 3.0),
                );
            }
        }
        _ => {
            let mut state = scene.seed.max(1);
            let points = scene.iterations.saturating_mul(120).clamp(20_000, 240_000);
            let (mut x, mut y) = (0.0_f32, 0.0_f32);
            for index in 0..points {
                let random_value = random(&mut state);
                (x, y) = match scene.kind {
                    SceneKind::BarnsleyFern => {
                        if random_value < 0.01 {
                            (0.0, 0.16 * y)
                        } else if random_value < 0.86 {
                            (0.85 * x + 0.04 * y, -0.04 * x + 0.85 * y + 1.6)
                        } else if random_value < 0.93 {
                            (0.2 * x - 0.26 * y, 0.23 * x + 0.22 * y + 1.6)
                        } else {
                            (-0.15 * x + 0.28 * y, 0.26 * x + 0.24 * y + 0.44)
                        }
                    }
                    SceneKind::DragonCurve => {
                        if random_value < 0.5 {
                            (0.5 * (x - y), 0.5 * (x + y))
                        } else {
                            (0.5 * (x + y) + 1.0, 0.5 * (-x + y))
                        }
                    }
                    _ => {
                        let vertex = (random_value * 3.0).floor() as usize;
                        let vertices = [(-1.0, 0.9), (1.0, 0.9), (0.0, -0.9)];
                        (
                            (x + vertices[vertex].0) * 0.5,
                            (y + vertices[vertex].1) * 0.5,
                        )
                    }
                };
                let (px, py) = if matches!(scene.kind, SceneKind::BarnsleyFern) {
                    (
                        scene.width as f32 * (0.5 + x / 5.5),
                        scene.height as f32 * (0.96 - y / 11.0),
                    )
                } else if matches!(scene.kind, SceneKind::DragonCurve) {
                    (
                        scene.width as f32 * (0.18 + x / 3.0),
                        scene.height as f32 * (0.18 + y / 2.5),
                    )
                } else {
                    (
                        scene.width as f32 * (0.5 + x * 0.44),
                        scene.height as f32 * (0.52 + y * 0.44),
                    )
                };
                if index > 20 {
                    draw_glow(
                        image,
                        px as i32,
                        py as i32,
                        1,
                        shade(palette, index as f32 / points as f32),
                    );
                }
            }
        }
    }
}

fn draw_branch(
    image: &mut RgbImage,
    x: f32,
    y: f32,
    length: f32,
    angle: f32,
    depth: u8,
    colours: ([u8; 3], [u8; 3]),
) {
    if depth == 0 || length < 2.0 {
        return;
    }
    let end_x = x + angle.cos() * length;
    let end_y = y + angle.sin() * length;
    draw_line(
        image,
        x,
        y,
        end_x,
        end_y,
        shade(colours, 1.0 - depth as f32 / 10.0),
    );
    draw_branch(
        image,
        end_x,
        end_y,
        length * 0.72,
        angle - 0.43,
        depth - 1,
        colours,
    );
    draw_branch(
        image,
        end_x,
        end_y,
        length * 0.72,
        angle + 0.49,
        depth - 1,
        colours,
    );
}

fn draw_koch(image: &mut RgbImage, start: (f32, f32), end: (f32, f32), depth: u8, colour: Rgb<u8>) {
    if depth == 0 {
        draw_line(image, start.0, start.1, end.0, end.1, colour);
        return;
    }
    let one = ((start.0 * 2.0 + end.0) / 3.0, (start.1 * 2.0 + end.1) / 3.0);
    let two = ((start.0 + end.0 * 2.0) / 3.0, (start.1 + end.1 * 2.0) / 3.0);
    let dx = two.0 - one.0;
    let dy = two.1 - one.1;
    let peak = (
        one.0 + dx * 0.5 - dy * 0.866_025_4,
        one.1 + dy * 0.5 + dx * 0.866_025_4,
    );
    draw_koch(image, start, one, depth - 1, colour);
    draw_koch(image, one, peak, depth - 1, colour);
    draw_koch(image, peak, two, depth - 1, colour);
    draw_koch(image, two, end, depth - 1, colour);
}

fn render_tiles(scene: &Scene, image: &mut RgbImage, phase: f32) {
    let palette = palette(&scene.palette);
    match scene.kind {
        SceneKind::OpArt => {
            for y in 0..scene.height {
                for x in 0..scene.width {
                    let nx = x as f32 / scene.width as f32 * 2.0 - 1.0;
                    let ny = y as f32 / scene.height as f32 * 2.0 - 1.0;
                    let wave = ((nx * 24.0 + (ny * 7.0 + phase).sin() * 5.0).sin()
                        + (ny * 20.0).cos())
                        * 0.5;
                    if wave.abs() > 0.65 {
                        image.put_pixel(x, y, shade(palette, (wave + 1.0) * 0.5));
                    }
                }
            }
        }
        SceneKind::HexWeave => {
            let radius = (scene.width.min(scene.height) / 28).max(8) as f32;
            let vertical = radius * 1.5;
            let horizontal = radius * 1.732;
            let rows = (scene.height as f32 / vertical) as usize + 2;
            let columns = (scene.width as f32 / horizontal) as usize + 2;
            for row in 0..rows {
                for column in 0..columns {
                    let cx = column as f32 * horizontal
                        + if row % 2 == 0 { 0.0 } else { horizontal * 0.5 };
                    let cy = row as f32 * vertical;
                    let mut prior: Option<(f32, f32)> = None;
                    for edge in 0..=6 {
                        let angle = edge as f32 * std::f32::consts::TAU / 6.0 + phase * 0.02;
                        let point = (cx + angle.cos() * radius, cy + angle.sin() * radius);
                        if let Some(previous) = prior {
                            draw_line(
                                image,
                                previous.0,
                                previous.1,
                                point.0,
                                point.1,
                                shade(palette, row as f32 / rows as f32),
                            );
                        }
                        prior = Some(point);
                    }
                }
            }
        }
        _ => {
            let tiles = 18_u32;
            let width = scene.width as f32 / tiles as f32;
            let height = scene.height as f32 / tiles as f32;
            let mut state = scene.seed.max(1);
            for row in 0..tiles {
                for column in 0..tiles {
                    let left = column as f32 * width;
                    let top = row as f32 * height;
                    let colour = shade(palette, (row + column) as f32 / (tiles * 2) as f32);
                    if random(&mut state) > 0.5 {
                        draw_line(image, left, top, left + width, top + height, colour);
                    } else {
                        draw_line(image, left + width, top, left, top + height, colour);
                    }
                }
            }
        }
    }
}

fn render_cells(scene: &Scene, image: &mut RgbImage, phase: f32) {
    let palette = palette(&scene.palette);
    if matches!(scene.kind, SceneKind::ReactionDiffusion) {
        for y in 0..scene.height {
            for x in 0..scene.width {
                let nx = x as f32 / scene.width as f32 * 10.0 - 5.0;
                let ny = y as f32 / scene.height as f32 * 10.0 - 5.0;
                let field = ((nx * nx + ny * ny).sqrt() * 5.5
                    + (nx * 2.1 + phase).sin()
                    + (ny * 1.7 - phase).cos())
                .sin();
                if field > 0.15 {
                    image.put_pixel(x, y, shade(palette, (field + 1.0) * 0.5));
                }
            }
        }
        return;
    }
    let cells = scene.width.min(1_024) as usize;
    let mut row = vec![false; cells];
    row[cells / 2] = true;
    let cell_width = scene.width as f32 / cells as f32;
    for y in 0..scene.height {
        let source = row.clone();
        for (x, active) in source.iter().copied().enumerate() {
            if active {
                let start = (x as f32 * cell_width) as u32;
                let end = (((x + 1) as f32 * cell_width) as u32).min(scene.width);
                for pixel_x in start..end {
                    image.put_pixel(pixel_x, y, shade(palette, y as f32 / scene.height as f32));
                }
            }
            let left = source[(x + cells - 1) % cells] as u8;
            let center = source[x] as u8;
            let right = source[(x + 1) % cells] as u8;
            let pattern = (left << 2) | (center << 1) | right;
            row[x] = ((90_u8 >> pattern) & 1) == 1;
        }
    }
}

fn render_attractor(scene: &Scene, image: &mut RgbImage, phase: f32) {
    let palette = palette(&scene.palette);
    let (a, b, c, d) = (-1.4 + phase.sin() * 0.04, 1.6, 1.0, 0.7);
    let (mut x, mut y) = (0.1_f32, 0.1_f32);
    let points = scene.iterations.saturating_mul(160).clamp(40_000, 320_000);
    for index in 0..points {
        let next_x = (a * y).sin() + c * (a * x).cos();
        let next_y = (b * x).sin() + d * (b * y).cos();
        x = next_x;
        y = next_y;
        if index > 100 {
            let px = scene.width as f32 * (0.5 + x * 0.22);
            let py = scene.height as f32 * (0.5 + y * 0.22);
            draw_glow(
                image,
                px as i32,
                py as i32,
                1,
                shade(palette, index as f32 / points as f32),
            );
        }
    }
}

fn draw_line(
    image: &mut RgbImage,
    start_x: f32,
    start_y: f32,
    end_x: f32,
    end_y: f32,
    colour: Rgb<u8>,
) {
    let dx = end_x - start_x;
    let dy = end_y - start_y;
    let steps = dx.abs().max(dy.abs()).ceil().clamp(1.0, 4_096.0) as usize;
    for step in 0..=steps {
        let amount = step as f32 / steps as f32;
        draw_glow(
            image,
            (start_x + dx * amount) as i32,
            (start_y + dy * amount) as i32,
            1,
            colour,
        );
    }
}

fn render_particles(scene: &Scene, image: &mut RgbImage, phase: f32) {
    let palette = palette(&scene.palette);
    let mut state = scene.seed.max(1);
    for index in 0..scene.iterations * 10 {
        let angle = random(&mut state) * std::f32::consts::TAU;
        let radius = random(&mut state).sqrt() * scene.width.min(scene.height) as f32 * 0.46;
        let spiral = angle + radius * 0.018 + phase;
        let x = scene.width as f32 * 0.5 + spiral.cos() * radius;
        let y = scene.height as f32 * 0.5 + spiral.sin() * radius * 1.25;
        draw_glow(
            image,
            x as i32,
            y as i32,
            2 + (random(&mut state) * 5.0) as i32,
            shade(palette, index as f32 / (scene.iterations * 10) as f32),
        );
    }
}

fn render_flow(scene: &Scene, image: &mut RgbImage, phase: f32) {
    let palette = palette(&scene.palette);
    let mut state = scene.seed.max(1);
    for line in 0..scene.iterations {
        let mut x = random(&mut state) * scene.width as f32;
        let mut y = random(&mut state) * scene.height as f32;
        let colour = shade(palette, line as f32 / scene.iterations as f32);
        for _ in 0..90 {
            draw_glow(image, x as i32, y as i32, 1, colour);
            let angle = (x * 0.012 + phase).sin() * 2.1 + (y * 0.009 - phase * 0.7).cos() * 1.7;
            x += angle.cos() * 2.2;
            y += angle.sin() * 2.2;
        }
    }
}

fn draw_glow(image: &mut RgbImage, cx: i32, cy: i32, radius: i32, colour: Rgb<u8>) {
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let x = cx + dx;
            let y = cy + dy;
            if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
                continue;
            }
            let distance = ((dx * dx + dy * dy) as f32).sqrt();
            let strength = (1.0 - distance / (radius as f32 + 0.5)).max(0.0);
            let pixel = image.get_pixel_mut(x as u32, y as u32);
            for channel in 0..3 {
                pixel[channel] =
                    pixel[channel].saturating_add((colour[channel] as f32 * strength) as u8);
            }
        }
    }
}

/// Every palette this renderer knows, by every name it answers to.
pub const PALETTE_NAMES: [&str; 20] = [
    "violet",
    "royal",
    "purple_haze",
    "amethyst",
    "ember",
    "gold",
    "solar_gold",
    "ice",
    "cyan",
    "electric_cyan",
    "blue_ocean",
    "neon_sunset",
    "sunset",
    "twilight",
    "pastel",
    "rainbow",
    "ice_fire",
    "monochrome",
    "chrome",
    "emerald",
];

/// Whether a name resolves to a real palette.
///
/// The field used to accept any string and fall through to the default, so a
/// whole paragraph of the Artist's prose could be stored as the palette and the
/// render would report success. Every artwork came out the same because the
/// request had quietly become the default scene.
pub fn is_known_palette(name: &str) -> bool {
    let wanted = name.trim().to_ascii_lowercase();
    PALETTE_NAMES.iter().any(|known| *known == wanted)
}

fn palette(name: &str) -> ([u8; 3], [u8; 3]) {
    match name.trim().to_ascii_lowercase().as_str() {
        "violet" | "royal" | "purple_haze" | "amethyst" => ([8, 2, 14], [255, 120, 245]),
        "ember" | "gold" | "solar_gold" => ([100, 24, 0], [255, 240, 168]),
        "ice" | "cyan" | "electric_cyan" | "blue_ocean" => ([2, 7, 18], [180, 255, 255]),
        "neon_sunset" | "sunset" => ([16, 3, 11], [255, 230, 80]),
        "twilight" | "pastel" => ([9, 7, 25], [255, 217, 160]),
        "rainbow" | "ice_fire" => ([18, 2, 29], [255, 99, 63]),
        "monochrome" | "chrome" => ([3, 3, 3], [255, 255, 255]),
        _ => ([3, 16, 7], [220, 255, 220]),
    }
}

fn shade((low, high): ([u8; 3], [u8; 3]), amount: f32) -> Rgb<u8> {
    let amount = amount.clamp(0.0, 1.0);
    Rgb(std::array::from_fn(|index| {
        (low[index] as f32 * (1.0 - amount) + high[index] as f32 * amount) as u8
    }))
}

fn random(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state as u32) as f32 / u32::MAX as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gallery_removes_the_reported_mandala_and_tree_without_changing_the_subject() {
        let original = "version 2; type=lissajous; width=96; height=128; iterations=180; seed=18365444092935997690; palette=amethyst; symmetry=12; zoom=1.025; motion=0.675; layers=mandala@0.22@screen@electric_cyan;fractal_tree@0.15@screen@amethyst";
        let clean = single_family_dsl(original).unwrap();
        let composite = Scene::parse(original).unwrap();
        let cleaned = Scene::parse(&clean).unwrap();
        let mut subject_only = composite.clone();
        subject_only.layers.clear();
        assert_eq!(
            serde_json::to_value(&cleaned).unwrap(),
            serde_json::to_value(&subject_only).unwrap()
        );
        assert_eq!(single_family_dsl(&clean).unwrap(), clean);
        for phase in [0.0, 0.9, 2.1] {
            let actual = cleaned.render_frame(phase);
            assert_eq!(actual, subject_only.render_frame(phase));
            assert_ne!(actual, composite.render_frame(phase));
        }
        // Explicit library composites remain available outside the gallery.
        assert_eq!(composite.layers.len(), 2);
    }

    #[test]
    fn a_layers_only_gallery_request_preserves_its_first_family_and_palette() {
        let clean = single_family_dsl(
            "version 2; width=96; height=128; seed=9; layers=spirograph@0.3@screen@ice_fire;fractal_tree@0.1@add@gold",
        )
        .unwrap();
        let actual = Scene::parse(&clean).unwrap();
        let expected = Scene::parse(
            "version 2; width=96; height=128; seed=9; type=spirograph; palette=ice_fire",
        )
        .unwrap();
        assert!(actual.layers.is_empty());
        assert_eq!(actual.render_frame(0.7), expected.render_frame(0.7));
    }

    #[test]
    fn a_single_family_request_is_preserved_and_invalid_subjects_still_fail() {
        let original = "version 2; kind=julia; width=96; height=128; iterations=180; seed=9; palette=solar_gold; symmetry=12; zoom=2; motion=0; title=Quiet Current";
        assert_eq!(
            single_family_dsl(original).unwrap(),
            canonical_dsl(original)
        );
        assert!(single_family_dsl("type=made_up; layers=mandala@0.2").is_err());
        assert!(Scene::default().layers.is_empty());
    }

    #[test]
    fn the_researched_hypotrochoid_name_selects_its_existing_spirograph_geometry() {
        let payload = "version 2; type=hypotrochoid; width=96; height=128; seed=463; palette=solar_gold; symmetry=12; zoom=1.2; motion=0.65";
        let actual = Scene::parse(payload).unwrap();
        let named = Scene::parse(&payload.replace("hypotrochoid", "spirograph")).unwrap();
        assert!(matches!(actual.kind, SceneKind::Spirograph));
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(&named).unwrap()
        );
        assert_eq!(actual.render_frame(0.7), named.render_frame(0.7));
        assert!(Scene::parse("type=epitrochoid").is_err());
    }

    /// The payload shapes actually seen on stream, and what each must produce.
    #[test]
    fn a_layer_stack_sets_the_subject_rather_than_defaulting_to_particles() {
        let scene = Scene::parse(concat!(
            "version 2
",
            "layers=kaleidoscope@0.45@screen@violet;lissajous@0.35@multiply@electric_cyan"
        ))
        .expect("a usable stack");
        assert!(
            matches!(scene.kind, SceneKind::Kaleidoscope),
            "the first layer is the subject, got {:?}",
            scene.kind
        );
        assert_eq!(scene.layers.len(), 2);
    }

    /// Verbatim from the stream: the Artist's tagged payload, rejected with
    /// "version is not a valid number" while prose salvage rendered a scene
    /// it had not asked for.
    #[test]
    fn the_one_line_form_the_artist_is_taught_parses_with_its_layer_stack() {
        let scene = Scene::parse(
            "version 2; type=julia; width=600; height=800; iterations=180; seed=3947825609874563210; palette=solar_gold; symmetry=12; zoom=2; motion=0.7; layers=mandala@0.18@screen@electric_cyan;fractal_tree@0.1@screen@solar_gold",
        )
        .expect("the documented one-line DSL must parse");
        assert!(matches!(scene.kind, SceneKind::Julia), "{:?}", scene.kind);
        assert_eq!(scene.seed, 3_947_825_609_874_563_210);
        assert_eq!(scene.palette, "solar_gold");
        assert_eq!((scene.width, scene.height, scene.symmetry), (600, 800, 12));
        assert_eq!(scene.layers.len(), 2, "{:?}", scene.layers);
        assert!(matches!(scene.layers[1].kind, SceneKind::FractalTree));
        assert!(scene.unknown_layers.is_empty());
    }

    #[test]
    fn one_line_and_multi_line_forms_describe_the_same_scene() {
        let one_line = Scene::parse(
            "version 2; type=mandala; seed=9; palette=violet; layers=moire@0.3@screen@ice;woven_web@0.2@add",
        )
        .unwrap();
        let lines = Scene::parse(
            "version 2\ntype=mandala\nseed=9\npalette=violet\nlayers=moire@0.3@screen@ice;woven_web@0.2@add",
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(one_line).unwrap(),
            serde_json::to_value(lines).unwrap()
        );
    }

    #[test]
    fn a_one_line_payload_still_reports_its_real_error() {
        // The correction loop can only help when it names the actual problem.
        let error = Scene::parse("version 2; type=hyptotrochoid; palette=violet")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("unknown scene type 'hyptotrochoid'"),
            "{error}"
        );
        assert!(Scene::parse("version 2; type=mandala; stray words").is_err());
    }

    #[test]
    fn prose_is_refused_rather_than_stored_as_a_palette() {
        // Verbatim: a whole paragraph arrived as the palette and rendered the
        // default scene while reporting success.
        let prose = "To explore subtle shadow manipulation in generative art,                      let's consider a Fractal Tree structure";
        assert!(
            Scene::parse(&format!(
                "version 2
palette={prose}"
            ))
            .is_err()
        );
    }

    #[test]
    fn an_unknown_palette_is_refused() {
        assert!(
            Scene::parse(
                "version 2
palette=rose_gold"
            )
            .is_err()
        );
        assert!(
            Scene::parse(
                "version 2
palette=electric_cyan"
            )
            .is_ok()
        );
    }

    #[test]
    fn an_out_of_range_layer_alpha_is_clamped_not_rejected() {
        // The Artist writes alphas of 3 and 4; the layer is still what it asked
        // for, so the number is corrected rather than the request discarded.
        let scene = Scene::parse(
            "version 2
layers=moire@3@screen@electric_cyan",
        )
        .expect("the layer survives a bad alpha");
        assert_eq!(scene.layers.len(), 1);
        assert!((scene.layers[0].alpha - 1.0).abs() < 1e-6);
    }

    /// Verbatim from the stream: two real families and one the Artist invented.
    #[test]
    fn an_invented_layer_does_not_discard_the_real_ones() {
        let scene = Scene::parse(concat!(
            "version 2
",
            "layers=kaleidoscope@0.45@screen@violet;",
            "spirograph@0.35@add@solar_gold;",
            "reeds_and_inkwells@0.2@screen@violet"
        ))
        .expect("the usable layers must survive");
        assert_eq!(scene.layers.len(), 2, "{:?}", scene.layers);
        assert_eq!(scene.unknown_layers, vec!["reeds_and_inkwells".to_owned()]);
    }

    #[test]
    fn a_stack_of_only_invented_layers_is_refused() {
        // Rendering the default scene under an unrelated request is worse than
        // saying the request could not be met.
        assert!(
            Scene::parse(
                "version 2
layers=reeds_and_inkwells@0.2"
            )
            .is_err()
        );
    }
    use image::AnimationDecoder;

    #[test]
    fn version_two_dsl_parses_and_is_bounded() {
        let scene = Scene::parse(
            "version 2\ntype=flow_field\nwidth=320\nheight=480\niterations=80\npalette=violet",
        )
        .unwrap();
        assert_eq!(scene.width, 320);
        assert!(matches!(scene.kind, SceneKind::FlowField));
    }

    #[test]
    fn oversized_render_is_refused() {
        let scene = Scene {
            width: MAX_DIMENSION + 1,
            ..Scene::default()
        };
        assert!(scene.validate().is_err());
    }

    #[test]
    fn live_preview_frame_count_is_bounded() {
        assert!(
            Scene::default()
                .render_gif(Path::new("unused.gif"), 1)
                .is_err()
        );
    }

    #[test]
    fn new_geometry_parameters_are_bounded_and_old_scenes_still_load() {
        // Non-finite values are refused; finite out-of-range values are the
        // model's clear intent and are clamped into range.
        for field in ["zoom=NaN", "motion=inf"] {
            assert!(
                Scene::parse(&format!("version 2\n{field}")).is_err(),
                "{field}"
            );
        }
        let clamped = Scene::parse("version 2\nsymmetry=2\nzoom=100\nmotion=-0.1").unwrap();
        assert_eq!(
            (clamped.symmetry, clamped.zoom, clamped.motion),
            (6, 8.0, 0.0)
        );
        assert_eq!(Scene::parse("version 2\nsymmetry=25").unwrap().symmetry, 24);
        let scene =
            Scene::parse("version 2\ntype=mandala\nsymmetry=18\nzoom=1.2\nmotion=0.4").unwrap();
        assert_eq!(scene.symmetry, 18);
        let mut old = serde_json::to_value(scene).unwrap();
        for field in ["symmetry", "zoom", "motion"] {
            old.as_object_mut().unwrap().remove(field);
        }
        let restored: Scene = serde_json::from_value(old).unwrap();
        assert_eq!(restored.symmetry, 12);
        assert_eq!(restored.zoom, 1.0);
        assert_eq!(restored.motion, 0.65);
    }

    #[test]
    fn animation_preserves_full_colour_frame_timing_and_motion() {
        let path =
            std::env::temp_dir().join(format!("teledra-fractus-apng-{}.png", std::process::id()));
        let scene = Scene::parse("version 2\ntype=lotus_mandala\nwidth=96\nheight=128\niterations=80\npalette=neon_sunset").unwrap();
        let manifest = scene.render_animation(&path, 8, 24).unwrap();
        let metadata = manifest.animation.unwrap();
        assert_eq!(metadata.frame_count, 8);
        assert_eq!(metadata.fps, 24);
        assert!(metadata.distinct_frames > 1);
        let decoder = image::codecs::png::PngDecoder::new(std::io::BufReader::new(
            File::open(&path).unwrap(),
        ))
        .unwrap();
        assert!(decoder.is_apng().unwrap());
        let frames = decoder
            .apng()
            .unwrap()
            .into_frames()
            .collect_frames()
            .unwrap();
        assert_eq!(frames.len(), 8);
        for (index, frame) in frames.iter().enumerate() {
            let expected = scene.render_frame(index as f32 / 8.0 * std::f32::consts::TAU);
            let decoded = DynamicImage::ImageRgba8(frame.buffer().clone()).to_rgb8();
            assert_eq!(
                decoded, expected,
                "full-colour frame {index} must roundtrip losslessly"
            );
            let (num, den) = frame.delay().numer_denom_ms();
            assert!((f64::from(num) / f64::from(den) - 1000.0 / 24.0).abs() < 0.01);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn excessive_animation_memory_and_invalid_fps_are_rejected_before_writing() {
        let scene = Scene {
            width: 2048,
            height: 2048,
            ..Scene::default()
        };
        assert!(
            scene
                .render_animation(Path::new("must-not-exist.png"), 120, 24)
                .is_err()
        );
        assert!(
            Scene::default()
                .render_animation(Path::new("must-not-exist.png"), 8, 0)
                .is_err()
        );
    }

    #[test]
    fn live_preview_contains_distinct_animation_frames() {
        let path = std::env::temp_dir().join(format!(
            "teledra-fractus-animation-{}.gif",
            std::process::id()
        ));
        let scene = Scene::parse(
            "version 2\ntype=particles\nwidth=96\nheight=128\niterations=48\npalette=emerald",
        )
        .unwrap();
        scene.render_gif(&path, 4).unwrap();
        let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
            std::fs::File::open(&path).unwrap(),
        ))
        .unwrap();
        let frames = decoder.into_frames().collect_frames().unwrap();
        assert_eq!(frames.len(), 4);
        assert_ne!(frames[0].buffer().as_raw(), frames[1].buffer().as_raw());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn original_artist_families_render_non_blank_frames() {
        for family in [
            "julia",
            "burning_ship",
            "tricorn",
            "newton",
            "mandala",
            "woven_web",
            "guilloche",
            "lissajous",
            "moire",
            "orbital_lace",
        ] {
            let scene = Scene::parse(&format!(
                "version 2\ntype={family}\nwidth=96\nheight=96\niterations=40\npalette=purple_haze"
            ))
            .unwrap();
            let image = scene.render_frame(0.0);
            let lit = image
                .pixels()
                .filter(|pixel| pixel.0.iter().any(|channel| *channel > 4))
                .count();
            assert!(lit > 40, "{family} was blank");
        }
    }

    #[test]
    fn layered_scene_supports_alpha_and_blend_modes() {
        let scene = Scene::parse(
            "version 2\ntype=mandala\nwidth=96\nheight=96\niterations=40\npalette=purple_haze\nlayers=moire@0.35@screen@electric_cyan;woven_web@0.2@add@solar_gold",
        )
        .unwrap();
        assert_eq!(scene.layers.len(), 2);
        let image = scene.render_frame(0.0);
        assert!(
            image
                .pixels()
                .any(|pixel| pixel.0.iter().any(|value| *value > 10))
        );
    }
}
