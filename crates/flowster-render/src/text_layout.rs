//! Laid-out text (SPEC §9): positioned glyph runs for a display-list text block,
//! produced by `flowster-text` from real font data so line breaks and widths are
//! the same in every exporter. Coordinates are inches in the block's frame:
//! origin at the top-left of the text block, +Y down.

use std::sync::Arc;

use flowster_core::value::Rgba;

/// Font bytes shared between layouts (a face of a font file or collection).
#[derive(Clone, Debug)]
pub struct FontFace {
    pub data: Arc<Vec<u8>>,
    pub index: u32,
    /// Stable key for de-duplicating embedded fonts.
    pub key: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedGlyph {
    pub id: u32,
    /// Offset from the run origin (x along the baseline, y down), inches.
    pub x: f64,
    pub y: f64,
    pub advance: f64,
}

#[derive(Clone, Debug)]
pub struct GlyphRun {
    /// Index into [`TextLayout::fonts`].
    pub font: usize,
    /// Font size in inches.
    pub size: f64,
    pub color: Rgba,
    pub underline: bool,
    pub strike: bool,
    /// Run origin: x from the block's left edge, y = the line's baseline.
    pub x: f64,
    pub baseline: f64,
    pub width: f64,
    /// The text the glyphs came from (for searchable PDF text).
    pub text: String,
    pub glyphs: Vec<PlacedGlyph>,
}

#[derive(Clone, Debug, Default)]
pub struct TextLayout {
    pub runs: Vec<GlyphRun>,
    pub fonts: Vec<FontFace>,
    /// Extent of the laid-out text (for `TEXTWIDTH`/`TEXTHEIGHT` and backgrounds).
    pub width: f64,
    pub height: f64,
    /// Lines as (top, height, left, width) in the block frame, for text backgrounds.
    pub lines: Vec<[f64; 4]>,
}
