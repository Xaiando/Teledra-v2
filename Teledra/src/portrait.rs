//! The court portrait, drawn with Unicode half blocks.
//!
//! A terminal cell is roughly twice as tall as it is wide, so one cell can
//! carry two vertical pixels: `▄` painted with the lower pixel as foreground
//! and the upper pixel as background. That doubles the effective resolution of
//! the panel at no cost.
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

/// The half block: the lower half of the cell is filled.
const LOWER_HALF_BLOCK: &str = "\u{2584}";

/// One terminal cell: the two vertical pixels it stands in for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Cell {
    upper: (u8, u8, u8),
    lower: (u8, u8, u8),
}

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
    scaled: RefCell<Option<Scaled>>,
}

/// The court's likeness, compiled into the binary.
///
/// V1 resolved this from disk and paid for it repeatedly: the asset directory
/// was never committed, a canonicalised `\\?\` root silently broke every
/// lookup, and startup checks reported art as present that no child process
/// could open. A portrait is fixed at build time and weighs under a megabyte,
/// so shipping it inside the executable removes that entire class of failure.
const EMBEDDED_PORTRAIT: &[u8] = include_bytes!("../../assets/teledra.png");

impl Portrait {
    /// The portrait the court will actually wear.
    ///
    /// An operator may override the likeness by placing `teledra.png` in an
    /// assets directory; anything unreadable falls back to the embedded art, so
    /// this cannot fail and the court is never without a face.
    pub fn load() -> Self {
        locate_asset("teledra.png")
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
        Self {
            source,
            scaled: RefCell::new(None),
        }
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
                                LOWER_HALF_BLOCK,
                                Style::default()
                                    .fg(Color::Rgb(cell.lower.0, cell.lower.1, cell.lower.2))
                                    .bg(Color::Rgb(cell.upper.0, cell.upper.1, cell.upper.2)),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect()
    }

    fn rescale(&self, width: u16, height: u16) -> Scaled {
        // Two image rows per terminal row, hence the doubled height.
        let resized = self.source.resize_exact(
            u32::from(width),
            u32::from(height) * 2,
            FilterType::Lanczos3,
        );
        let rows = (0..u32::from(height))
            .map(|row| {
                (0..u32::from(width))
                    .map(|column| {
                        let upper = resized.get_pixel(column, row * 2).0;
                        let lower = resized.get_pixel(column, row * 2 + 1).0;
                        Cell {
                            upper: (upper[0], upper[1], upper[2]),
                            lower: (lower[0], lower[1], lower[2]),
                        }
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

    #[test]
    fn one_cell_carries_two_vertical_pixels() {
        let portrait = Portrait::from_image(banded());
        let lines = portrait.lines(4, 2);

        assert_eq!(lines.len(), 2, "two terminal rows for four image rows");
        assert_eq!(lines[0].spans.len(), 4);
        assert_eq!(lines[0].spans[0].content, LOWER_HALF_BLOCK);

        // The top band is red and the bottom blue, so the split has to land
        // between the two rendered rows rather than smearing across them.
        assert_eq!(lines[0].spans[0].style.bg, Some(Color::Rgb(255, 0, 0)));
        assert_eq!(lines[1].spans[0].style.fg, Some(Color::Rgb(0, 0, 255)));
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
        let portrait = Portrait::from_image(banded());
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
        let portrait = Portrait::from_image(banded());
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
}
