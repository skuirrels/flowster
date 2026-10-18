//! `flowster-text`: lays out display-list text blocks with parley (SPEC §9).
//!
//! Given the same font bytes, layout is identical on every platform: parley
//! shapes with a pure-Rust shaper and breaks lines with UAX #14. Native builds
//! use the host's installed fonts; a font the document names but the host lacks
//! falls back to the generic sans-serif face (the deterministic font pack for
//! shared documents, §20.3, plugs in here).
//!
//! Visio paragraphs are laid out one by one: each has its own alignment,
//! indents, spacing and line height (`SpLine`); vertical alignment and margins
//! then place the whole stack inside the text block.

#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use flowster_render::text_layout::{FontFace, GlyphRun, PlacedGlyph, TextLayout};
use flowster_render::{DisplayList, Item, Paragraph};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontFamilyName, FontStyle, FontWeight, GenericFamily,
    LayoutContext, LineHeight, PositionedLayoutItem, StyleProperty,
};

/// Points per inch: layout runs in points, results are returned in inches.
const PT: f64 = 72.0;

/// Fonts and scratch space; create once and reuse for every block.
pub struct TextEngine {
    fonts: FontContext,
    layout: LayoutContext<u16>,
    faces: HashMap<(u64, u32), FontFace>,
}

impl TextEngine {
    /// An engine over the host's installed fonts.
    #[cfg(feature = "system-fonts")]
    pub fn system() -> Self {
        TextEngine { fonts: FontContext::new(), layout: LayoutContext::new(), faces: HashMap::new() }
    }

    /// An engine with no fonts but those registered with [`TextEngine::register_font`].
    pub fn empty() -> Self {
        let fonts = FontContext {
            collection: parley::fontique::Collection::new(parley::fontique::CollectionOptions {
                shared: false,
                system_fonts: false,
            }),
            ..Default::default()
        };
        TextEngine { fonts, layout: LayoutContext::new(), faces: HashMap::new() }
    }

    /// Adds font bytes (a font file or collection) to the engine.
    pub fn register_font(&mut self, bytes: Vec<u8>) {
        self.fonts.collection.register_fonts(parley::fontique::Blob::new(Arc::new(bytes)), None);
    }

    fn face(&mut self, data: &parley::FontData) -> FontFace {
        let key = (data.data.id(), data.index);
        self.faces
            .entry(key)
            .or_insert_with(|| FontFace {
                data: Arc::new(data.data.as_ref().to_vec()),
                index: data.index,
                key: key.0 ^ u64::from(key.1),
            })
            .clone()
    }

    /// Lays out one text item; `None` for anything that is not text.
    pub fn layout_item(&mut self, item: &Item) -> Option<TextLayout> {
        let Item::Text { width, height, margins, valign, paragraphs, .. } = item else { return None };
        let [ml, mr, mt, mb] = *margins;
        let inner_w = (width - ml - mr).max(0.01);
        let mut out = TextLayout::default();
        let mut y = 0.0;
        let mut face_index: HashMap<u64, usize> = HashMap::new();
        for para in paragraphs {
            y += para.space_before;
            let laid = self.layout_paragraph(para, inner_w);
            for (line_top, line_h, runs) in laid.lines {
                let top = y + line_top;
                let mut left = f64::MAX;
                let mut right = f64::MIN;
                for mut run in runs {
                    let idx = *face_index.entry(run.0.key).or_insert_with(|| {
                        out.fonts.push(run.0.clone());
                        out.fonts.len() - 1
                    });
                    run.1.font = idx;
                    run.1.x += ml;
                    run.1.baseline += y;
                    left = left.min(run.1.x);
                    right = right.max(run.1.x + run.1.width);
                    out.runs.push(run.1);
                }
                if left <= right {
                    out.lines.push([top, line_h, left, right - left]);
                }
            }
            y += laid.height + para.space_after;
            out.width = out.width.max(laid.width);
        }
        out.height = y;
        // Vertical alignment inside the block (between the top and bottom margins).
        let avail = height - mt - mb;
        let dy = mt
            + match valign {
                0 => 0.0,
                2 => avail - y,
                _ => (avail - y) / 2.0,
            };
        for r in &mut out.runs {
            r.baseline += dy;
        }
        for l in &mut out.lines {
            l[0] += dy;
        }
        Some(out)
    }

    fn layout_paragraph(&mut self, para: &Paragraph, inner_w: f64) -> LaidParagraph {
        let text: String = para.runs.iter().map(|r| r.text.as_str()).collect();
        let mut laid = LaidParagraph::default();
        if text.is_empty() {
            return laid;
        }
        let first = &para.runs[0];
        let indent = para.indent_left.max(0.0);
        let max_w = ((inner_w - indent) * PT).max(1.0) as f32;
        let mut builder = self.layout.ranged_builder(&mut self.fonts, &text, 1.0, false);
        builder.push_default(StyleProperty::FontSize((first.size * PT) as f32));
        builder.push_default(StyleProperty::LineHeight(if para.line_spacing < 0.0 {
            LineHeight::FontSizeRelative(-para.line_spacing as f32)
        } else {
            LineHeight::Absolute((para.line_spacing * PT) as f32)
        }));
        let mut start = 0;
        for (i, run) in para.runs.iter().enumerate() {
            let range = start..start + run.text.len();
            start = range.end;
            let family = FontFamily::List(Cow::Owned(vec![
                FontFamilyName::Named(Cow::Owned(run.font.clone())),
                FontFamilyName::Generic(GenericFamily::SansSerif),
            ]));
            builder.push(StyleProperty::FontFamily(family), range.clone());
            builder.push(StyleProperty::FontSize((run.size * PT) as f32), range.clone());
            if run.bold {
                builder.push(StyleProperty::FontWeight(FontWeight::BOLD), range.clone());
            }
            if run.italic {
                builder.push(StyleProperty::FontStyle(FontStyle::Italic), range.clone());
            }
            builder.push(StyleProperty::Brush(i as u16), range);
        }
        let mut layout = builder.build(&text);
        layout.break_all_lines(Some(max_w));
        let alignment = match para.align {
            0 => Alignment::Left,
            2 => Alignment::Right,
            3 | 4 => Alignment::Justify,
            _ => Alignment::Center,
        };
        layout.align(alignment, AlignmentOptions { align_when_overflowing: true, ..Default::default() });

        // parley positions lines within `max_w`; centring is relative to the paragraph box.
        for line in layout.lines() {
            let m = line.metrics();
            let mut runs = Vec::new();
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(gr) = item else { continue };
                let run = gr.run();
                let style = &para.runs[usize::from(gr.style().brush).min(para.runs.len() - 1)];
                let face = self.face(run.font());
                let origin_x = f64::from(gr.offset());
                let glyphs: Vec<PlacedGlyph> = gr
                    .positioned_glyphs()
                    .map(|g| PlacedGlyph {
                        id: g.id,
                        x: (f64::from(g.x) - origin_x) / PT,
                        y: (f64::from(g.y) - f64::from(gr.baseline())) / PT,
                        advance: f64::from(g.advance) / PT,
                    })
                    .collect();
                let range = run.text_range();
                runs.push((
                    face,
                    GlyphRun {
                        font: 0,
                        size: f64::from(run.font_size()) / PT,
                        color: style.color,
                        underline: style.underline,
                        strike: style.strike,
                        x: indent + origin_x / PT,
                        baseline: f64::from(gr.baseline()) / PT,
                        width: f64::from(gr.advance()) / PT,
                        text: text.get(range).unwrap_or_default().to_owned(),
                        glyphs,
                    },
                ));
            }
            laid.lines.push((f64::from(m.block_min_coord) / PT, f64::from(m.line_height) / PT, runs));
        }
        laid.height = f64::from(layout.height()) / PT;
        laid.width = f64::from(layout.width()) / PT + indent;
        laid
    }

    /// Lays out every text item of a display list (indexed like `dl.items`).
    pub fn layout_display_list(&mut self, dl: &DisplayList) -> Vec<Option<TextLayout>> {
        dl.items.iter().map(|i| self.layout_item(i)).collect()
    }
}

/// A laid-out line: (top, height, runs with their faces).
type LaidLine = (f64, f64, Vec<(FontFace, GlyphRun)>);

#[derive(Default)]
struct LaidParagraph {
    lines: Vec<LaidLine>,
    height: f64,
    width: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lays_out_the_fixture_text_with_system_fonts() {
        let pkg = flowster_vsdx::OpcPackage::open(flowster_vsdx::fixture::minimal_vsdx()).unwrap();
        let doc = flowster_vsdx::visio::read_document(&pkg).unwrap();
        let dl = flowster_render::build_page(&doc, 0).unwrap();
        let mut engine = TextEngine::system();
        let layouts = engine.layout_display_list(&dl);
        let Some(layout) = layouts.iter().flatten().next() else { panic!("no text layout") };
        if layout.fonts.is_empty() {
            // No fonts at all on this machine (minimal CI image): nothing to check.
            return;
        }
        let text: String = layout.runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "Hello from Flowster");
        assert!(layout.width > 0.5 && layout.width < 2.0, "width {}", layout.width);
        assert!(layout.runs.iter().all(|r| !r.glyphs.is_empty()));
    }
}
