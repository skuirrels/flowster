//! Glyph outlines from font bytes (skrifa), cached per font, glyph and size.

use std::collections::HashMap;

use flowster_core::geom::Seg;
use flowster_render::text_layout::FontFace;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, GlyphId, MetadataProvider};

/// Outlines in font units (TrueType scalers work in 26.6 fixed point, so
/// asking for tiny sizes such as 0.17 "pixels" would wreck the shapes).
#[derive(Default)]
pub struct GlyphCache {
    outlines: HashMap<(u64, u32), Vec<Seg>>,
    units_per_em: HashMap<u64, f64>,
}

struct Pen(Vec<Seg>);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(Seg::M { x: f64::from(x), y: f64::from(y) });
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push(Seg::L { x: f64::from(x), y: f64::from(y) });
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        // Elevate to a cubic using the previous point.
        let (px, py) = self.0.iter().rev().find_map(Seg::end).unwrap_or((0.0, 0.0));
        let (cx, cy, x, y) = (f64::from(cx), f64::from(cy), f64::from(x), f64::from(y));
        self.0.push(Seg::C {
            x1: px + 2.0 / 3.0 * (cx - px),
            y1: py + 2.0 / 3.0 * (cy - py),
            x2: x + 2.0 / 3.0 * (cx - x),
            y2: y + 2.0 / 3.0 * (cy - y),
            x,
            y,
        });
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.push(Seg::C {
            x1: f64::from(cx0),
            y1: f64::from(cy0),
            x2: f64::from(cx1),
            y2: f64::from(cy1),
            x: f64::from(x),
            y: f64::from(y),
        });
    }
    fn close(&mut self) {
        self.0.push(Seg::Z);
    }
}

impl GlyphCache {
    /// The outline of `glyph` in font units (y up, origin on the baseline) and
    /// the face's units per em: scale by `size / units_per_em` to place it.
    pub fn outline(&mut self, face: &FontFace, glyph: u32) -> Option<(&[Seg], f64)> {
        let key = (face.key, glyph);
        if !self.outlines.contains_key(&key) {
            let font = FontRef::from_index(&face.data, face.index).ok()?;
            let upem = f64::from(font.metrics(Size::unscaled(), LocationRef::default()).units_per_em).max(1.0);
            self.units_per_em.insert(face.key, upem);
            let g = font.outline_glyphs().get(GlyphId::new(glyph))?;
            let mut pen = Pen(Vec::new());
            g.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).ok()?;
            self.outlines.insert(key, pen.0);
        }
        let upem = *self.units_per_em.get(&face.key)?;
        self.outlines.get(&key).map(|o| (o.as_slice(), upem))
    }
}
