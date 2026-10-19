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
    Alignment, AlignmentOptions, FontContext, FontFamily, FontFamilyName, FontFeatures, FontStyle, FontWeight,
    GenericFamily, IndentOptions, LayoutContext, LineHeight, PositionedLayoutItem, StyleProperty,
};

/// Points per inch: layout runs in points, results are returned in inches.
const PT: f64 = 72.0;

/// Fonts and scratch space; create once and reuse for every block.
pub struct TextEngine {
    fonts: FontContext,
    layout: LayoutContext<u16>,
    faces: HashMap<(u64, u32), FontFace>,
    /// Baseline position within a line box, per face (see [`baseline_ratio`]).
    ratios: HashMap<u64, Option<f64>>,
}

impl TextEngine {
    /// An engine over the host's installed fonts.
    #[cfg(feature = "system-fonts")]
    pub fn system() -> Self {
        TextEngine {
            fonts: FontContext::new(),
            layout: LayoutContext::new(),
            faces: HashMap::new(),
            ratios: HashMap::new(),
        }
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
        TextEngine { fonts, layout: LayoutContext::new(), faces: HashMap::new(), ratios: HashMap::new() }
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
        // Paragraph spacing only separates paragraphs: Visio ignores the first
        // paragraph's SpBefore and the last one's SpAfter.
        let last = paragraphs.len().saturating_sub(1);
        for (n, para) in paragraphs.iter().enumerate() {
            if n > 0 {
                y += para.space_before;
            }
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
            y += laid.height;
            if n < last {
                y += para.space_after;
            }
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
        let Some(first) = para.runs.first() else { return laid };
        if text.is_empty() {
            // An empty paragraph still takes a line.
            laid.height = pitch(para.line_spacing, first.size * PT) / PT;
            return laid;
        }
        let indent = para.indent_left.max(0.0);
        let max_w = ((inner_w - indent) * PT).max(1.0) as f32;
        let mut builder = self.layout.ranged_builder(&mut self.fonts, &text, 1.0, false);
        builder.push_default(StyleProperty::FontFamily(family(&first.font)));
        // Visio does not apply the fonts' pair kerning (measured: runs without
        // kerning pairs match its widths exactly, sentences only without `kern`).
        builder.push_default(StyleProperty::FontFeatures(FontFeatures::Source(Cow::Borrowed("\"kern\" off"))));
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
            builder.push(StyleProperty::FontFamily(family(&run.font)), range.clone());
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
        // IndFirst moves the first line relative to IndLeft (negative: a hanging indent).
        layout.set_text_indent((para.indent_first * PT) as f32, IndentOptions::default());
        layout.break_all_lines(Some(max_w));
        let alignment = match para.align {
            0 => Alignment::Left,
            2 => Alignment::Right,
            3 | 4 => Alignment::Justify,
            _ => Alignment::Center,
        };
        layout.align(alignment, AlignmentOptions { align_when_overflowing: true, ..Default::default() });

        // parley breaks and positions lines horizontally. Vertically Visio sets
        // each line in a box of SpLine × its largest font size, with the
        // baseline at ascent / (ascent + descent) of that box.
        let mut top = 0.0;
        for line in layout.lines() {
            let (mut size, mut ratio) = (0.0f64, 0.75);
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(gr) = item else { continue };
                let run = gr.run();
                if f64::from(run.font_size()) > size {
                    size = f64::from(run.font_size());
                    let face = self.face(run.font());
                    let typo = *self.ratios.entry(face.key).or_insert_with(|| baseline_ratio(&face));
                    let m = run.font_metrics();
                    ratio = typo.unwrap_or(if m.ascent + m.descent > 0.0 {
                        f64::from(m.ascent / (m.ascent + m.descent))
                    } else {
                        0.75
                    });
                }
            }
            let line_pitch = pitch(para.line_spacing, size);
            let baseline = (top + line_pitch * ratio) / PT;
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
                        baseline,
                        width: f64::from(gr.advance()) / PT,
                        text: text.get(range).unwrap_or_default().to_owned(),
                        glyphs,
                    },
                ));
            }
            laid.lines.push((top / PT, line_pitch / PT, runs));
            top += line_pitch;
        }
        laid.height = top / PT;
        laid.width = f64::from(layout.width()) / PT + indent;
        laid
    }

    /// Lays out every text item of a display list (indexed like `dl.items`).
    pub fn layout_display_list(&mut self, dl: &DisplayList) -> Vec<Option<TextLayout>> {
        dl.items.iter().map(|i| self.layout_item(i)).collect()
    }
}

/// Where Visio puts the baseline in a line box, as a fraction of its height:
/// the OS/2 typographic ascent over ascent + descent (measured against Visio
/// with Calibri and Times New Roman, whose hhea metrics differ).
fn baseline_ratio(face: &FontFace) -> Option<f64> {
    use skrifa::raw::TableProvider as _;
    let font = skrifa::FontRef::from_index(&face.data, face.index).ok()?;
    let os2 = font.os2().ok()?;
    let (asc, desc) = (f64::from(os2.s_typo_ascender()), -f64::from(os2.s_typo_descender()));
    (asc > 0.0 && asc + desc > 0.0).then(|| asc / (asc + desc))
}

/// Line pitch in points: `SpLine` < 0 is a multiple of the font size, > 0 an
/// absolute height in inches, 0 single spacing (120 %).
fn pitch(line_spacing: f64, size_pt: f64) -> f64 {
    if line_spacing < 0.0 {
        -line_spacing * size_pt
    } else if line_spacing > 0.0 {
        line_spacing * PT
    } else {
        1.2 * size_pt
    }
}

fn family(name: &str) -> FontFamily<'static> {
    FontFamily::List(Cow::Owned(vec![
        FontFamilyName::Named(Cow::Owned(name.to_owned())),
        FontFamilyName::Generic(GenericFamily::SansSerif),
    ]))
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

    fn para(text: &str, size_pt: f64, line_spacing: f64, before_pt: f64, after_pt: f64) -> Paragraph {
        Paragraph {
            align: 0,
            line_spacing,
            space_before: before_pt / PT,
            space_after: after_pt / PT,
            indent_left: 0.0,
            indent_first: 0.0,
            runs: vec![flowster_render::Run {
                text: text.into(),
                font: "Calibri".into(),
                size: size_pt / PT,
                color: flowster_core::value::Rgba::BLACK,
                bold: false,
                italic: false,
                underline: false,
                strike: false,
            }],
        }
    }

    fn block(paragraphs: Vec<Paragraph>) -> Item {
        Item::Text {
            shape: 1,
            transform: flowster_core::transform::Affine::IDENTITY,
            width: 4.0,
            height: 3.0,
            margins: [0.0; 4],
            valign: 0,
            background: None,
            paragraphs,
        }
    }

    #[test]
    fn lines_follow_visio_pitch_and_paragraph_spacing() {
        let mut engine = TextEngine::system();
        // Two 36 pt paragraphs at 150 %: each line is 54 pt; the first
        // paragraph's SpBefore and the last one's SpAfter are ignored.
        let item = block(vec![para("HHH", 36.0, -1.5, 30.0, 6.0), para("HHH", 36.0, -1.5, 12.0, 30.0)]);
        let layout = engine.layout_item(&item).unwrap();
        if layout.fonts.is_empty() {
            return;
        }
        let pt = |v: f64| (v * PT * 1000.0).round() / 1000.0;
        assert_eq!(pt(layout.height), 54.0 + 6.0 + 12.0 + 54.0);
        assert_eq!(pt(layout.lines[0][0]), 0.0);
        assert_eq!(pt(layout.lines[1][0]), 54.0 + 18.0);
        // The baseline sits at ascent / (ascent + descent) of the line box.
        let ratio = layout.runs[0].baseline * PT / 54.0;
        assert!(ratio > 0.6 && ratio < 0.9, "{ratio}");
        // An absolute SpLine (inches) is used as is.
        let item = block(vec![para("HHH", 36.0, 24.0 / PT, 0.0, 0.0)]);
        assert_eq!(pt(engine.layout_item(&item).unwrap().height), 24.0);
    }
}
