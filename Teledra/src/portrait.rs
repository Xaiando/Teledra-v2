//! The court portrait, drawn with Unicode block glyphs.
//!
//! A terminal cell can stand in for more than one pixel. The cheapest trick is
//! the half block `▄`: foreground paints the lower pixel, background the upper,
//! so one cell carries two vertical pixels. Quadrants (`▘▖▗▝…`) carry four in a
//! 2x2 grid and sextants (`🬀…`) carry six in a 2x3 grid, at the cost of only
//! two distinct colours per cell.
//!
//! Sextants are the default: three times the pixels of a half block, and their
//! 2x3 grid is the closest match to the roughly 1:2 shape of a terminal cell,
//! so the sub-pixels stay near square instead of stretching.
//!
//! Rescaling is the expensive step, so the rendered rows are memoised and
//! recomputed only when the panel changes size.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use image::{DynamicImage, GenericImageView, imageops::FilterType};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

/// How many pixels one terminal cell is asked to carry.
///
/// Every mode paints at most two colours per cell, so denser modes trade colour
/// fidelity inside a cell for spatial detail. On a photographic portrait the
/// detail wins: neighbouring sub-pixels are nearly the same colour anyway.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Density {
    /// 1x2 pixels per cell. Universally supported, including bare consoles.
    #[default]
    Half,
    /// 2x2 pixels per cell. Supported wherever the Block Elements are.
    Quadrant,
    /// 2x3 pixels per cell. Needs Symbols for Legacy Computing (Unicode 13).
    Sextant,
}

impl Density {
    /// Sub-pixel columns in one cell.
    const fn columns(self) -> u32 {
        match self {
            Self::Half => 1,
            Self::Quadrant | Self::Sextant => 2,
        }
    }

    /// Sub-pixel rows in one cell.
    const fn rows(self) -> u32 {
        match self {
            Self::Half | Self::Quadrant => 2,
            Self::Sextant => 3,
        }
    }

    /// The glyph whose filled sub-pixels are exactly the set bits of `mask`.
    ///
    /// Bits run left to right, then top to bottom, so bit 0 is the top-left
    /// sub-pixel and the last bit is the bottom-right one.
    fn glyph(self, mask: u8) -> &'static str {
        match self {
            Self::Half => HALVES[usize::from(mask) & 0b11],
            Self::Quadrant => QUADRANTS[usize::from(mask) & 0b1111],
            Self::Sextant => SEXTANTS[usize::from(mask) & 0b11_1111],
        }
    }

    /// The glyph with every sub-pixel filled.
    const fn solid(self) -> &'static str {
        "\u{2588}"
    }

    /// The density an operator asked for, falling back to the default.
    ///
    /// Fonts that predate Unicode 13 render sextants as tofu, so an escape
    /// hatch has to exist that does not require rebuilding the court.
    pub fn from_environment() -> Self {
        match std::env::var("TELEDRA_PORTRAIT_DENSITY")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "half" | "1" | "2580" => Self::Half,
            "quadrant" | "quad" | "4" => Self::Quadrant,
            // V1 used the lower-half block renderer. Keep that as the shipped
            // look because it works in every Windows terminal/font; denser
            // modes remain explicit opt-ins for operators who want them.
            "sextant" | "six" | "6" => Self::Sextant,
            _ => Self::Half,
        }
    }
}

/// Half blocks indexed by `top | bottom << 1`.
const HALVES: [&str; 4] = [" ", "\u{2580}", "\u{2584}", "\u{2588}"];

/// Quadrants indexed by `tl | tr << 1 | bl << 2 | br << 3`.
const QUADRANTS: [&str; 16] = [
    " ", "\u{2598}", "\u{259d}", "\u{2580}", "\u{2596}", "\u{258c}", "\u{259e}", "\u{259b}",
    "\u{2597}", "\u{259a}", "\u{2590}", "\u{259c}", "\u{2584}", "\u{2599}", "\u{259f}", "\u{2588}",
];

/// Sextants indexed by the six sub-pixels in reading order.
///
/// Symbols for Legacy Computing omits the four patterns that already exist as
/// Block Elements — empty, left half, right half, and full — so those four
/// entries reach back into the older block.
const SEXTANTS: [&str; 64] = [
    " ", "🬀", "🬁", "🬂", "🬃", "🬄", "🬅", "🬆", "🬇", "🬈", "🬉", "🬊", "🬋", "🬌", "🬍", "🬎", "🬏", "🬐", "🬑",
    "🬒", "🬓", "▌", "🬔", "🬕", "🬖", "🬗", "🬘", "🬙", "🬚", "🬛", "🬜", "🬝", "🬞", "🬟", "🬠", "🬡", "🬢", "🬣",
    "🬤", "🬥", "🬦", "🬧", "▐", "🬨", "🬩", "🬪", "🬫", "🬬", "🬭", "🬮", "🬯", "🬰", "🬱", "🬲", "🬳", "🬴", "🬵",
    "🬶", "🬷", "🬸", "🬹", "🬺", "🬻", "█",
];

/// One terminal cell: a glyph and the two colours that fill it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Cell {
    glyph: &'static str,
    /// Colour of the sub-pixels the glyph fills.
    ink: (u8, u8, u8),
    /// Colour of the sub-pixels it leaves bare.
    paper: (u8, u8, u8),
}

/// One rendered cell: the glyph, the colour it fills, the colour it leaves.
pub type RenderedCell = (&'static str, (u8, u8, u8), (u8, u8, u8));

/// Rows of cells rendered for one specific panel size.
#[derive(Clone, Debug)]
struct Scaled {
    width: u16,
    height: u16,
    rows: Vec<Vec<Cell>>,
}

#[derive(Debug)]
pub struct Portrait {
    source: DynamicImage,
    density: Density,
    scaled: RefCell<Option<Scaled>>,
}

/// The court's likeness, compiled into the binary.
///
/// V1 resolved this from disk and paid for it repeatedly: the asset directory
/// was never committed, a canonicalised `\\?\` root silently broke every
/// lookup, and startup checks reported art as present that no child process
/// could open. A portrait is fixed at build time and weighs under a megabyte,
/// so shipping it inside the executable removes that entire class of failure.
const EMBEDDED_PORTRAIT: &[u8] = include_bytes!("../../assets/teledra_bg (2).png");

impl Portrait {
    /// The portrait the court will actually wear.
    ///
    /// An operator may override the likeness by placing `teledra.png` in an
    /// assets directory; anything unreadable falls back to the embedded art, so
    /// this cannot fail and the court is never without a face.
    pub fn load() -> Self {
        locate_asset("teledra_bg (2).png")
            .and_then(|path| Self::from_path(&path))
            .unwrap_or_else(Self::embedded)
    }

    /// The likeness compiled into this binary.
    pub fn embedded() -> Self {
        let source = image::load_from_memory(EMBEDDED_PORTRAIT)
            .expect("the portrait compiled into this binary must decode");
        Self::from_image(source)
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        let source = image::open(path).ok()?;
        Some(Self::from_image(source))
    }

    pub fn from_image(source: DynamicImage) -> Self {
        Self::with_density(source, Density::from_environment())
    }

    pub fn with_density(source: DynamicImage, density: Density) -> Self {
        Self {
            source,
            density,
            scaled: RefCell::new(None),
        }
    }

    /// The rendered cells for a panel of this size, as (glyph, ink, paper).
    ///
    /// Screen capture is not available on the machine this runs on, so the only
    /// way to see what the portrait panel actually draws is to ask it. This is
    /// what `--portrait` dumps to an image.
    pub fn cells(&self, width: u16, height: u16) -> Vec<Vec<RenderedCell>> {
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let mut cache = self.scaled.borrow_mut();
        let fits = cache
            .as_ref()
            .is_some_and(|scaled| scaled.width == width && scaled.height == height);
        if !fits {
            *cache = Some(self.rescale(width, height));
        }
        cache
            .as_ref()
            .map(|scaled| {
                scaled
                    .rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|cell| (cell.glyph, cell.ink, cell.paper))
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Rows for a panel of exactly this size, rescaling only when it changed.
    pub fn lines(&self, width: u16, height: u16) -> Vec<Line<'static>> {
        if width == 0 || height == 0 {
            return Vec::new();
        }

        let mut cache = self.scaled.borrow_mut();
        let fits = cache
            .as_ref()
            .is_some_and(|scaled| scaled.width == width && scaled.height == height);
        if !fits {
            *cache = Some(self.rescale(width, height));
        }

        cache
            .as_ref()
            .expect("the cache was just populated")
            .rows
            .iter()
            .map(|row| {
                Line::from(
                    row.iter()
                        .map(|cell| {
                            Span::styled(
                                cell.glyph,
                                Style::default()
                                    .fg(Color::Rgb(cell.ink.0, cell.ink.1, cell.ink.2))
                                    .bg(Color::Rgb(cell.paper.0, cell.paper.1, cell.paper.2)),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect()
    }

    fn rescale(&self, width: u16, height: u16) -> Scaled {
        let columns = self.density.columns();
        let rows_per_cell = self.density.rows();
        let resized = self.source.resize_exact(
            u32::from(width) * columns,
            u32::from(height) * rows_per_cell,
            FilterType::Lanczos3,
        );

        let mut samples = Vec::with_capacity((columns * rows_per_cell) as usize);
        let rows = (0..u32::from(height))
            .map(|row| {
                (0..u32::from(width))
                    .map(|column| {
                        samples.clear();
                        for sub_row in 0..rows_per_cell {
                            for sub_column in 0..columns {
                                let pixel = resized
                                    .get_pixel(
                                        column * columns + sub_column,
                                        row * rows_per_cell + sub_row,
                                    )
                                    .0;
                                samples.push((pixel[0], pixel[1], pixel[2]));
                            }
                        }
                        quantise(self.density, &samples)
                    })
                    .collect()
            })
            .collect();

        Scaled {
            width,
            height,
            rows,
        }
    }
}

/// Reduces one cell's sub-pixels to a glyph and the two colours it can carry.
///
/// The split runs along whichever channel varies most across the cell, which is
/// the cheapest stand-in for the dominant axis of the colour cluster: an edge
/// between hair and armour separates on the channel that actually distinguishes
/// them rather than on luminance, which can be equal on both sides.
fn quantise(density: Density, samples: &[(u8, u8, u8)]) -> Cell {
    let channel = |sample: &(u8, u8, u8), index: usize| match index {
        0 => sample.0,
        1 => sample.1,
        _ => sample.2,
    };

    let mut widest = 0;
    let mut spread = 0;
    for index in 0..3 {
        let low = samples.iter().map(|s| channel(s, index)).min().unwrap_or(0);
        let high = samples.iter().map(|s| channel(s, index)).max().unwrap_or(0);
        if high - low >= spread {
            spread = high - low;
            widest = index;
        }
    }

    // A flat cell has no edge to preserve: paint it solid and skip the split,
    // which also keeps large smooth areas free of dithering artefacts.
    //
    // It is painted as a full block rather than a blank, even though both look
    // identical when the two colours match. A blank cell carries its colour
    // only in the background, so any terminal that drops background colour
    // turns every smooth region into a hole and leaves the portrait as a field
    // of disconnected edges. A full block keeps its colour in the foreground
    // and degrades to a silhouette instead.
    if spread == 0 {
        let solid = samples.first().copied().unwrap_or((0, 0, 0));
        return Cell {
            glyph: density.solid(),
            ink: solid,
            paper: solid,
        };
    }

    let low = samples
        .iter()
        .map(|s| channel(s, widest))
        .min()
        .unwrap_or(0);
    let threshold = u16::from(low) + u16::from(spread) / 2;

    let mut mask = 0u8;
    let mut ink = (0u32, 0u32, 0u32, 0u32);
    let mut paper = (0u32, 0u32, 0u32, 0u32);
    for (index, sample) in samples.iter().enumerate() {
        let bucket = if u16::from(channel(sample, widest)) > threshold {
            mask |= 1 << index;
            &mut ink
        } else {
            &mut paper
        };
        bucket.0 += u32::from(sample.0);
        bucket.1 += u32::from(sample.1);
        bucket.2 += u32::from(sample.2);
        bucket.3 += 1;
    }

    Cell {
        glyph: density.glyph(mask),
        ink: mean(ink),
        paper: mean(paper),
    }
}

/// The average of a bucket, or black when nothing landed in it.
fn mean((red, green, blue, count): (u32, u32, u32, u32)) -> (u8, u8, u8) {
    if count == 0 {
        return (0, 0, 0);
    }
    (
        (red / count) as u8,
        (green / count) as u8,
        (blue / count) as u8,
    )
}

/// Finds an asset without any absolute path baked into the binary.
///
/// `TELEDRA_ASSETS` wins when set; otherwise the working directory and the
/// directory holding the executable are searched, so the court runs both from
/// a development checkout and from an installed copy.
fn locate_asset(name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(configured) = std::env::var_os("TELEDRA_ASSETS") {
        roots.push(PathBuf::from(configured));
    }
    if let Ok(working) = std::env::current_dir() {
        roots.push(working.join("assets"));
    }
    if let Some(beside_executable) = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(Path::to_path_buf))
    {
        roots.push(beside_executable.join("assets"));
    }

    roots
        .into_iter()
        .map(|root| root.join(name))
        .find(|candidate| candidate.is_file())
}

/// Writes what the portrait panel draws into a PNG, one block per cell.
///
/// Each cell becomes a rectangle painted with its ink and paper colours in the
/// arrangement its glyph implies, so the output is a faithful picture of the
/// terminal rather than a rescale of the source art.
pub fn dump(portrait: &Portrait, path: &str, columns: u16, rows: u16) -> std::io::Result<()> {
    const CELL_W: u32 = 6;
    const CELL_H: u32 = 12;
    let cells = portrait.cells(columns, rows);
    let mut canvas = image::RgbImage::new(u32::from(columns) * CELL_W, u32::from(rows) * CELL_H);

    for (row_index, row) in cells.iter().enumerate() {
        for (column_index, (glyph, ink, paper)) in row.iter().enumerate() {
            let mask: Vec<bool> = glyph
                .chars()
                .next()
                .map_or_else(|| vec![false; 6], sub_pixels_of);
            for y in 0..CELL_H {
                for x in 0..CELL_W {
                    let sub_column = (x * 2 / CELL_W) as usize;
                    let sub_row = (y * 3 / CELL_H) as usize;
                    let filled = mask[sub_row * 2 + sub_column];
                    let (r, g, b) = if filled { *ink } else { *paper };
                    canvas.put_pixel(
                        u32::try_from(column_index).unwrap_or(0) * CELL_W + x,
                        u32::try_from(row_index).unwrap_or(0) * CELL_H + y,
                        image::Rgb([r, g, b]),
                    );
                }
            }
        }
    }
    canvas.save(path).map_err(std::io::Error::other)
}

/// Which of a cell's six sub-pixels a glyph fills, in reading order.
fn sub_pixels_of(character: char) -> Vec<bool> {
    match character {
        ' ' => vec![false; 6],
        '\u{2588}' => vec![true; 6],
        '\u{2580}' => vec![true, true, false, false, false, false],
        '\u{2584}' => vec![false, false, false, false, true, true],
        '\u{258c}' => vec![true, false, true, false, true, false],
        '\u{2590}' => vec![false, true, false, true, false, true],
        _ => {
            // Quadrants and sextants: look the glyph up in the tables the
            // renderer itself used, so the dump can never drift from it.
            if let Some(index) = QUADRANTS.iter().position(|g| g.starts_with(character)) {
                let bits = [index & 1, index & 2, index & 4, index & 8];
                return vec![
                    bits[0] != 0,
                    bits[1] != 0,
                    bits[2] != 0,
                    bits[3] != 0,
                    bits[2] != 0,
                    bits[3] != 0,
                ];
            }
            if let Some(index) = SEXTANTS.iter().position(|g| g.starts_with(character)) {
                return (0..6).map(|bit| index & (1 << bit) != 0).collect();
            }
            vec![false; 6]
        }
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgb, RgbImage};

    use super::*;

    /// Two stacked bands, so the upper and lower halves of a cell must differ.
    fn banded() -> DynamicImage {
        let mut image = RgbImage::new(4, 4);
        for (_, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = if y < 2 {
                Rgb([255, 0, 0])
            } else {
                Rgb([0, 0, 255])
            };
        }
        DynamicImage::ImageRgb8(image)
    }

    /// Vertical stripes, which only a mode with two sub-pixel columns can show.
    fn striped() -> DynamicImage {
        let mut image = RgbImage::new(4, 6);
        for (x, _, pixel) in image.enumerate_pixels_mut() {
            *pixel = if x < 2 {
                Rgb([255, 255, 255])
            } else {
                Rgb([0, 0, 0])
            };
        }
        DynamicImage::ImageRgb8(image)
    }

    #[test]
    fn one_cell_carries_two_vertical_pixels() {
        let portrait = Portrait::with_density(banded(), Density::Half);
        let lines = portrait.lines(4, 2);

        assert_eq!(lines.len(), 2, "two terminal rows for four image rows");
        assert_eq!(lines[0].spans.len(), 4);

        // The top band is red and the bottom blue, so the split has to land
        // between the two rendered rows rather than smearing across them.
        let top = &lines[0].spans[0];
        assert!(
            [top.style.fg, top.style.bg].contains(&Some(Color::Rgb(255, 0, 0))),
            "the first row is the red band"
        );
        let bottom = &lines[1].spans[0];
        assert!(
            [bottom.style.fg, bottom.style.bg].contains(&Some(Color::Rgb(0, 0, 255))),
            "the second row is the blue band"
        );
    }

    #[test]
    fn a_sextant_cell_carries_six_pixels() {
        // One cell, so any detail that survives came from sub-pixels rather
        // than from the panel being large enough to resolve it.
        let portrait = Portrait::with_density(striped(), Density::Sextant);
        let lines = portrait.lines(1, 1);

        assert_eq!(lines.len(), 1);
        let cell = &lines[0].spans[0];
        assert_eq!(
            cell.content, "\u{258c}",
            "a left-hand white stripe is the left half of the cell"
        );

        // Lanczos rings slightly across the stripe edge, so the colours land
        // near rather than exactly on the source values; what matters is that
        // the cell still holds a bright side and a dark one.
        let Some(Color::Rgb(ink, _, _)) = cell.style.fg else {
            panic!("the lit half must carry a colour");
        };
        let Some(Color::Rgb(paper, _, _)) = cell.style.bg else {
            panic!("the unlit half must carry a colour");
        };
        assert!(ink > 200, "the white stripe stays bright, got {ink}");
        assert!(paper < 55, "the black stripe stays dark, got {paper}");
    }

    #[test]
    fn a_half_block_cell_cannot_carry_a_vertical_stripe() {
        // The counterpart to the test above: the same image in the old mode
        // collapses to a single averaged column, which is the resolution loss
        // the denser modes exist to fix.
        let portrait = Portrait::with_density(striped(), Density::Half);
        let cell = portrait.lines(1, 1)[0].spans[0].clone();
        assert_eq!(cell.style.fg, cell.style.bg, "no edge survives one column");
    }

    #[test]
    fn every_density_maps_all_of_its_patterns_to_a_glyph() {
        for (density, patterns) in [
            (Density::Half, 4u16),
            (Density::Quadrant, 16),
            (Density::Sextant, 64),
        ] {
            let glyphs: Vec<&str> = (0..patterns)
                .map(|mask| density.glyph(mask as u8))
                .collect();
            let mut unique = glyphs.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(
                unique.len(),
                glyphs.len(),
                "{density:?} must map each sub-pixel pattern to a distinct glyph"
            );
        }
    }

    /// The cached size, with the borrow released before returning: holding one
    /// across a `lines()` call would panic, so no test may keep the guard.
    fn cached_size(portrait: &Portrait) -> Option<(u16, u16)> {
        portrait
            .scaled
            .borrow()
            .as_ref()
            .map(|scaled| (scaled.width, scaled.height))
    }

    #[test]
    fn rescaling_happens_once_per_size() {
        let portrait = Portrait::with_density(banded(), Density::Sextant);
        assert_eq!(cached_size(&portrait), None, "nothing scaled until asked");

        let first = portrait.lines(4, 2);
        let cached = portrait.lines(4, 2);
        assert_eq!(first.len(), cached.len());
        assert_eq!(cached_size(&portrait), Some((4, 2)));

        // A resize must replace the cache rather than reuse stale rows.
        let resized = portrait.lines(8, 3);
        assert_eq!(resized.len(), 3);
        assert_eq!(resized[0].spans.len(), 8);
        assert_eq!(cached_size(&portrait), Some((8, 3)));
    }

    #[test]
    fn a_collapsed_panel_renders_nothing() {
        let portrait = Portrait::with_density(banded(), Density::Sextant);
        assert!(portrait.lines(0, 10).is_empty());
        assert!(portrait.lines(10, 0).is_empty());
    }

    #[test]
    fn the_embedded_likeness_decodes_and_renders() {
        // If this fails the shipped asset is corrupt, and it fails here at
        // build time rather than as a blank panel in front of an audience.
        let portrait = Portrait::embedded();
        let lines = portrait.lines(24, 12);
        assert_eq!(lines.len(), 12);
        assert_eq!(lines[0].spans.len(), 24);
    }

    #[test]
    fn an_unreadable_override_falls_back_to_the_embedded_likeness() {
        assert!(
            Portrait::from_path(Path::new("no-such-portrait.png")).is_none(),
            "an absent override must not masquerade as art"
        );

        // Tests run with the crate directory as the working directory, so no
        // override is discoverable here: `load` is exercising the fallback.
        let portrait = Portrait::load();
        assert_eq!(
            portrait.lines(20, 10).len(),
            10,
            "the court still has a face"
        );
    }

    #[test]
    fn a_smooth_region_keeps_its_colour_in_the_foreground() {
        // A blank cell carries colour only in the background. If a terminal
        // ever drops background colour, a portrait made of blanks becomes a
        // field of holes; one made of full blocks stays a silhouette.
        let mut flat = RgbImage::new(8, 8);
        for pixel in flat.pixels_mut() {
            *pixel = Rgb([90, 40, 120]);
        }
        let portrait = Portrait::with_density(DynamicImage::ImageRgb8(flat), Density::Half);
        for line in portrait.lines(4, 2) {
            for span in line.spans {
                assert_eq!(span.content, "\u{2588}", "smooth cells must be solid");
                assert_eq!(span.style.fg, Some(Color::Rgb(90, 40, 120)));
            }
        }
    }
}
