//! PDF export through krilla: vector paths and gradients, embedded subset fonts
//! with real (searchable, selectable) text, and images.
//!
//! Gaps for now: hatch patterns draw as their foreground colour, and shadows are
//! not blurred.

use std::collections::HashMap;

use base64::Engine as _;
use flowster_core::geom::Seg;
use flowster_core::transform::Affine;
use flowster_core::value::Rgba;
use flowster_render::paint::Paint;
use flowster_render::text_layout::TextLayout;
use flowster_render::{DisplayList, Item, Stop, Stroke};
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Size, Transform};
use krilla::num::NormalizedF32;
use krilla::paint::{Fill, FillRule, LineCap, LineJoin, LinearGradient, RadialGradient, SpreadMethod, StrokeDash};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use krilla::{Data, Document};

use crate::{alpha, bow_tie, mirrored};

const PT: f64 = 72.0;

fn nf(v: f32) -> NormalizedF32 {
    NormalizedF32::new(v.clamp(0.0, 1.0)).unwrap_or(NormalizedF32::ONE)
}

fn tf(a: &Affine) -> Transform {
    let [a, b, c, d, e, f] = a.0;
    Transform::from_row(a as f32, b as f32, c as f32, d as f32, e as f32, f as f32)
}

fn rgb(c: Rgba) -> rgb::Color {
    rgb::Color::new(c.0, c.1, c.2)
}

fn path(segs: &[Seg]) -> Option<krilla::geom::Path> {
    let mut pb = PathBuilder::new();
    for s in segs {
        match *s {
            Seg::M { x, y } => pb.move_to(x as f32, y as f32),
            Seg::L { x, y } => pb.line_to(x as f32, y as f32),
            Seg::C { x1, y1, x2, y2, x, y } => {
                pb.cubic_to(x1 as f32, y1 as f32, x2 as f32, y2 as f32, x as f32, y as f32)
            }
            Seg::Z => pb.close(),
        }
    }
    pb.finish()
}

fn stops(s: &[Stop]) -> Vec<krilla::paint::Stop> {
    s.iter()
        .map(|st| krilla::paint::Stop {
            offset: nf(st.offset as f32),
            color: rgb(st.color).into(),
            opacity: nf(alpha(st.color)),
        })
        .collect()
}

fn solid(c: Rgba) -> Fill {
    Fill { paint: rgb(c).into(), opacity: nf(alpha(c)), rule: FillRule::EvenOdd }
}

struct Writer {
    fonts: HashMap<u64, Font>,
}

impl Writer {
    fn fill(&mut self, s: &mut Surface<'_>, segs: &[Seg], paint: &Paint) {
        let Some(p) = path(segs) else { return };
        let gradient_fill =
            |paint: krilla::paint::Paint| Fill { paint, opacity: NormalizedF32::ONE, rule: FillRule::EvenOdd };
        s.set_stroke(None);
        match paint {
            Paint::Solid { color } => s.set_fill(Some(solid(*color))),
            Paint::Linear { space, x0, y0, x1, y1, stops: st } => s.set_fill(Some(gradient_fill(
                LinearGradient {
                    x1: *x0 as f32,
                    y1: *y0 as f32,
                    x2: *x1 as f32,
                    y2: *y1 as f32,
                    transform: tf(space),
                    spread_method: SpreadMethod::Pad,
                    stops: stops(st),
                    anti_alias: true,
                }
                .into(),
            ))),
            Paint::Radial { space, cx, cy, r, stops: st } => s.set_fill(Some(gradient_fill(
                RadialGradient {
                    fx: *cx as f32,
                    fy: *cy as f32,
                    fr: 0.0,
                    cx: *cx as f32,
                    cy: *cy as f32,
                    cr: *r as f32,
                    transform: tf(space),
                    spread_method: SpreadMethod::Pad,
                    stops: stops(st),
                    anti_alias: true,
                }
                .into(),
            ))),
            Paint::Rect { space, ox, oy, extent, stops: st } => {
                let m = mirrored(st);
                let lin = |x1: f64, y1: f64, x2: f64, y2: f64| {
                    gradient_fill(
                        LinearGradient {
                            x1: x1 as f32,
                            y1: y1 as f32,
                            x2: x2 as f32,
                            y2: y2 as f32,
                            transform: tf(space),
                            spread_method: SpreadMethod::Pad,
                            stops: stops(&m),
                            anti_alias: true,
                        }
                        .into(),
                    )
                };
                s.set_fill(Some(lin(ox - extent, *oy, ox + extent, *oy)));
                s.draw_path(&p);
                // The Y-dominant "bow tie", mapped to page coordinates.
                let bow: Vec<Seg> = bow_tie(*ox, *oy).iter().map(|g| g.map(|q| space.apply(q))).collect();
                if let Some(clip) = path(&bow) {
                    s.push_clip_path(&clip, &FillRule::NonZero);
                    s.set_fill(Some(lin(*ox, oy - extent, *ox, oy + extent)));
                    s.draw_path(&p);
                    s.pop();
                }
                return;
            }
            Paint::Pattern { fg, .. } => s.set_fill(Some(solid(*fg))),
        }
        s.draw_path(&p);
    }

    fn stroke(&mut self, s: &mut Surface<'_>, segs: &[Seg], st: &Stroke) {
        let Some(p) = path(segs) else { return };
        s.set_fill(None);
        s.set_stroke(Some(krilla::paint::Stroke {
            paint: rgb(st.color).into(),
            width: st.width.max(0.25 / PT) as f32,
            miter_limit: 4.0,
            line_cap: match st.cap {
                "square" => LineCap::Square,
                "butt" => LineCap::Butt,
                _ => LineCap::Round,
            },
            line_join: LineJoin::Round,
            opacity: nf(alpha(st.color)),
            dash: (st.dash.len() >= 2)
                .then(|| StrokeDash { array: st.dash.iter().map(|d| *d as f32).collect(), offset: 0.0 }),
        }));
        s.draw_path(&p);
        s.set_stroke(None);
    }

    fn font(&mut self, layout: &TextLayout, index: usize) -> Option<Font> {
        let face = layout.fonts.get(index)?;
        if let Some(f) = self.fonts.get(&face.key) {
            return Some(f.clone());
        }
        let font = Font::new(Data::from(face.data.clone()), face.index)?;
        self.fonts.insert(face.key, font.clone());
        Some(font)
    }

    fn text(
        &mut self,
        s: &mut Surface<'_>,
        transform: &Affine,
        height: f64,
        background: Option<Rgba>,
        layout: &TextLayout,
    ) {
        // Block frame: origin top-left, +Y down (the same orientation as PDF text).
        let frame = transform.then(Affine([1.0, 0.0, 0.0, -1.0, 0.0, height]));
        s.push_transform(&tf(&frame));
        if let Some(bg) = background {
            for l in &layout.lines {
                let [top, h, left, w] = *l;
                let r = [
                    Seg::M { x: left - 0.02, y: top },
                    Seg::L { x: left + w + 0.02, y: top },
                    Seg::L { x: left + w + 0.02, y: top + h },
                    Seg::L { x: left - 0.02, y: top + h },
                    Seg::Z,
                ];
                self.fill(s, &r, &Paint::Solid { color: bg });
            }
        }
        for run in &layout.runs {
            let Some(font) = self.font(layout, run.font) else { continue };
            let size = run.size.max(1e-6);
            let chars: Vec<(usize, char)> = run.text.char_indices().collect();
            let per_char = chars.len() == run.glyphs.len();
            let mut pen = 0.0;
            let glyphs: Vec<KrillaGlyph> = run
                .glyphs
                .iter()
                .enumerate()
                .map(|(i, g)| {
                    let range = if per_char {
                        let start = chars[i].0;
                        start..chars.get(i + 1).map_or(run.text.len(), |c| c.0)
                    } else {
                        0..run.text.len()
                    };
                    let k = KrillaGlyph::new(
                        GlyphId::new(g.id),
                        (g.advance / size) as f32,
                        ((g.x - pen) / size) as f32,
                        (g.y / size) as f32,
                        0.0,
                        range,
                        None,
                    );
                    pen += g.advance;
                    k
                })
                .collect();
            s.set_stroke(None);
            s.set_fill(Some(solid(run.color)));
            s.draw_glyphs(
                Point::from_xy(run.x as f32, run.baseline as f32),
                &glyphs,
                font,
                &run.text,
                size as f32,
                false,
            );
            if run.underline || run.strike {
                let t = run.size * 0.06;
                let y = if run.underline { run.baseline + run.size * 0.12 } else { run.baseline - run.size * 0.3 };
                let r = [
                    Seg::M { x: run.x, y },
                    Seg::L { x: run.x + run.width, y },
                    Seg::L { x: run.x + run.width, y: y + t },
                    Seg::L { x: run.x, y: y + t },
                    Seg::Z,
                ];
                self.fill(s, &r, &Paint::Solid { color: run.color });
            }
        }
        s.pop();
    }

    fn image(&mut self, s: &mut Surface<'_>, item: &Item) {
        let Item::Image { transform, x, y, w, h, mime, data, .. } = item else { return };
        let (x, y, w, h, data, mime) = (*x, *y, *w, *h, data.as_deref(), mime.as_str());
        let bytes = data.and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok());
        let image = bytes.and_then(|b| match mime {
            "image/png" => krilla::image::Image::from_png(Data::from(b), true).ok(),
            "image/jpeg" => krilla::image::Image::from_jpeg(Data::from(b), true).ok(),
            _ => None,
        });
        match (image, Size::from_wh(w as f32, h as f32)) {
            (Some(img), Some(size)) => {
                // Images are top-down: flip inside the shape frame.
                s.push_transform(&tf(&transform.then(Affine::translate(x, y + h)).then(Affine::scale(1.0, -1.0))));
                s.draw_image(img, size);
                s.pop();
            }
            _ => {
                let r = [
                    Seg::M { x, y },
                    Seg::L { x: x + w, y },
                    Seg::L { x: x + w, y: y + h },
                    Seg::L { x, y: y + h },
                    Seg::Z,
                ];
                let segs: Vec<Seg> = r.iter().map(|g| g.map(|p| transform.apply(p))).collect();
                self.fill(s, &segs, &Paint::Solid { color: Rgba(238, 238, 238, 255) });
            }
        }
    }
}

/// A PDF with one page per display list, each with its text layouts (indexed like the items).
pub fn pdf(pages: &[(&DisplayList, &[Option<TextLayout>])]) -> Result<Vec<u8>, String> {
    let mut doc = Document::new();
    let mut w = Writer { fonts: HashMap::new() };
    for (dl, layouts) in pages {
        let (pw, ph) = (dl.page.width, dl.page.height);
        let settings = krilla::page::PageSettings::from_wh((pw * PT) as f32, (ph * PT) as f32)
            .ok_or_else(|| format!("bad page size {pw} x {ph}"))?;
        let mut page = doc.start_page_with(settings);
        let mut s = page.surface();
        // Page inches (+Y up) → PDF points (krilla: +Y down from the top-left).
        s.push_transform(&tf(&Affine([PT, 0.0, 0.0, -PT, 0.0, ph * PT])));
        for (i, item) in dl.items.iter().enumerate() {
            match item {
                Item::Path { d, fill, stroke, .. } => {
                    if let Some(f) = fill {
                        w.fill(&mut s, d, f);
                    }
                    if let Some(st) = stroke {
                        w.stroke(&mut s, d, st);
                    }
                }
                Item::Text { transform, height, background, .. } => {
                    if let Some(Some(l)) = layouts.get(i) {
                        w.text(&mut s, transform, *height, *background, l);
                    }
                }
                Item::Image { .. } => w.image(&mut s, item),
            }
        }
        s.pop();
        s.finish();
        page.finish();
    }
    doc.finish().map_err(|e| format!("{e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_pdf_with_embedded_text() {
        let pkg = flowster_vsdx::OpcPackage::open(flowster_vsdx::fixture::minimal_vsdx()).unwrap();
        let doc = flowster_vsdx::visio::read_document(&pkg).unwrap();
        let dl = flowster_render::build_page(&doc, 0).unwrap();
        let layouts = flowster_text::TextEngine::system().layout_display_list(&dl);
        let bytes = pdf(&[(&dl, layouts.as_slice())]).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        // With fonts on the machine, the text is embedded as a font, not outlines.
        if layouts.iter().flatten().any(|l| !l.fonts.is_empty()) {
            assert!(bytes.windows(8).any(|w| w == b"FontFile"), "no embedded font program");
        }
    }
}
