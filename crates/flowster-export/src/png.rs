//! PNG export through tiny-skia.

use base64::Engine as _;
use flowster_core::geom::Seg;
use flowster_core::transform::Affine;
use flowster_core::value::Rgba;
use flowster_render::paint::Paint;
use flowster_render::text_layout::TextLayout;
use flowster_render::{DisplayList, Item, Stop, Stroke};
use tiny_skia as sk;

use crate::{GlyphCache, bow_tie, decode_image, mirrored};

/// Largest raster dimension we produce (pixels), to bound memory.
const MAX_SIDE: f64 = 16_384.0;

fn ts(a: &Affine) -> sk::Transform {
    let [a, b, c, d, e, f] = a.0;
    sk::Transform::from_row(a as f32, b as f32, c as f32, d as f32, e as f32, f as f32)
}

fn color(c: Rgba) -> sk::Color {
    sk::Color::from_rgba8(c.0, c.1, c.2, c.3)
}

fn path(segs: &[Seg]) -> Option<sk::Path> {
    let mut pb = sk::PathBuilder::new();
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

/// Per-pixel foreground coverage of a hatch tile. Visio draws the single-pixel
/// diagonals (patterns 2 and 5) as anti-aliased lines: 0.92 on the line and
/// 0.25 on each horizontal neighbour; every other tile is binary.
fn hatch_coverage(bits: &[u8; 8]) -> [[f32; 8]; 8] {
    let on = |row: usize, col: usize| bits[row % 8] & (0x80 >> (col % 8)) != 0;
    // One bit per row, each row the previous one moved a column sideways.
    let steps = |f: fn(u8, u32) -> u8| (0..8).all(|r| bits[(r + 1) % 8] == f(bits[r], 1));
    let diagonal = bits.iter().all(|b| b.count_ones() == 1) && (steps(u8::rotate_left) || steps(u8::rotate_right));
    std::array::from_fn(|r| {
        std::array::from_fn(|c| match (on(r, c), diagonal) {
            (true, true) => 0.92,
            (true, false) => 1.0,
            (false, true) if on(r, c + 1) || on(r, c + 7) => 0.25,
            _ => 0.0,
        })
    })
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Rgba(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2), m(a.3, b.3))
}

fn stops(s: &[Stop]) -> Vec<sk::GradientStop> {
    s.iter().map(|st| sk::GradientStop::new(st.offset as f32, color(st.color))).collect()
}

fn pt(x: f64, y: f64) -> sk::Point {
    sk::Point::from_xy(x as f32, y as f32)
}

struct Raster {
    pixmap: sk::Pixmap,
    /// Page inches → pixels.
    page: Affine,
    glyphs: GlyphCache,
}

impl Raster {
    fn fill(&mut self, segs: &[Seg], fill: &Paint, mask: Option<&sk::Mask>) {
        let Some(p) = path(segs) else { return };
        let page = ts(&self.page);
        let mut paint = sk::Paint { anti_alias: true, ..Default::default() };
        let mut set = |shader: Option<sk::Shader<'static>>, pixmap: &mut sk::Pixmap, mask: Option<&sk::Mask>| {
            if let Some(shader) = shader {
                paint.shader = shader;
                pixmap.fill_path(&p, &paint, sk::FillRule::EvenOdd, page, mask);
            }
        };
        match fill {
            Paint::Solid { color: c } => set(Some(sk::Shader::SolidColor(color(*c))), &mut self.pixmap, mask),
            Paint::Linear { space, x0, y0, x1, y1, stops: s } => set(
                sk::LinearGradient::new(pt(*x0, *y0), pt(*x1, *y1), stops(s), sk::SpreadMode::Pad, ts(space)),
                &mut self.pixmap,
                mask,
            ),
            Paint::Radial { space, cx, cy, r, stops: s } => set(
                sk::RadialGradient::new(
                    pt(*cx, *cy),
                    0.0,
                    pt(*cx, *cy),
                    *r as f32,
                    stops(s),
                    sk::SpreadMode::Pad,
                    ts(space),
                ),
                &mut self.pixmap,
                mask,
            ),
            Paint::Rect { space, ox, oy, extent, stops: s } => {
                let m = mirrored(s);
                let gx = sk::LinearGradient::new(
                    pt(ox - extent, *oy),
                    pt(ox + extent, *oy),
                    stops(&m),
                    sk::SpreadMode::Pad,
                    ts(space),
                );
                set(gx, &mut self.pixmap, mask);
                // Y-dominant region: a mask from the bow tie in paint space.
                let (w, h) = (self.pixmap.width(), self.pixmap.height());
                let (Some(mut bow), Some(bp)) = (sk::Mask::new(w, h), path(&bow_tie(*ox, *oy))) else { return };
                bow.fill_path(&bp, sk::FillRule::Winding, true, ts(&self.page.then(*space)));
                let gy = sk::LinearGradient::new(
                    pt(*ox, oy - extent),
                    pt(*ox, oy + extent),
                    stops(&m),
                    sk::SpreadMode::Pad,
                    ts(space),
                );
                set(gy, &mut self.pixmap, Some(&bow));
            }
            Paint::Pattern { bits, fg, bg, .. } => {
                // Visio rasterises hatches at one bit per output pixel (the
                // 8×8 tile repeats every 8 px at any export resolution).
                let Some(mut tile) = sk::Pixmap::new(8, 8) else { return };
                let cov = hatch_coverage(bits);
                for (i, px) in tile.pixels_mut().iter_mut().enumerate() {
                    let c = mix(*bg, *fg, cov[i / 8][i % 8]);
                    *px = color(c).premultiply().to_color_u8();
                }
                let device = self.page.invert().map_or(sk::Transform::identity(), |m| ts(&m));
                let shader =
                    sk::Pattern::new(tile.as_ref(), sk::SpreadMode::Repeat, sk::FilterQuality::Nearest, 1.0, device);
                paint.shader = shader;
                self.pixmap.fill_path(&p, &paint, sk::FillRule::EvenOdd, page, mask);
            }
        }
    }

    fn stroke(&mut self, segs: &[Seg], s: &Stroke) {
        let Some(p) = path(segs) else { return };
        let scale = self.page.mean_scale();
        let width = s.width.max(1.0 / scale) as f32;
        let mut st = sk::Stroke {
            width,
            line_cap: match s.cap {
                "square" => sk::LineCap::Square,
                "butt" => sk::LineCap::Butt,
                _ => sk::LineCap::Round,
            },
            line_join: sk::LineJoin::Round,
            ..Default::default()
        };
        if s.dash.len() >= 2 {
            let mut d: Vec<f32> = s.dash.iter().map(|v| *v as f32).collect();
            if d.len() % 2 == 1 {
                d.extend_from_within(..);
            }
            st.dash = sk::StrokeDash::new(d, 0.0);
        }
        let paint =
            sk::Paint { shader: sk::Shader::SolidColor(color(s.color)), anti_alias: true, ..Default::default() };
        self.pixmap.stroke_path(&p, &paint, &st, ts(&self.page), None);
    }

    fn text(&mut self, transform: &Affine, height: f64, background: Option<Rgba>, layout: &TextLayout) {
        // Block frame: origin top-left, +Y down.
        let frame = self.page.then(*transform).then(Affine([1.0, 0.0, 0.0, -1.0, 0.0, height]));
        if let Some(bg) = background {
            for l in &layout.lines {
                let [top, h, left, w] = *l;
                if let Some(r) = sk::Rect::from_xywh(left as f32 - 0.02, top as f32, w as f32 + 0.04, h as f32) {
                    let p =
                        sk::Paint { shader: sk::Shader::SolidColor(color(bg)), anti_alias: true, ..Default::default() };
                    self.pixmap.fill_rect(r, &p, ts(&frame), None);
                }
            }
        }
        for run in &layout.runs {
            let face = &layout.fonts[run.font];
            let paint =
                sk::Paint { shader: sk::Shader::SolidColor(color(run.color)), anti_alias: true, ..Default::default() };
            for g in &run.glyphs {
                let Some((outline, upem)) = self.glyphs.outline(face, g.id) else { continue };
                let Some(p) = path(outline) else { continue };
                // Outline is in font units, +Y up; the frame is +Y down.
                let k = run.size / upem;
                let at = frame.then(Affine([k, 0.0, 0.0, -k, run.x + g.x, run.baseline + g.y]));
                self.pixmap.fill_path(&p, &paint, sk::FillRule::Winding, ts(&at), None);
            }
            if run.underline || run.strike {
                let t = run.size * 0.06;
                let y = if run.underline { run.baseline + run.size * 0.12 } else { run.baseline - run.size * 0.3 };
                if let Some(r) = sk::Rect::from_xywh(run.x as f32, y as f32, run.width as f32, t as f32) {
                    self.pixmap.fill_rect(r, &paint, ts(&frame), None);
                }
            }
        }
    }

    fn image(&mut self, item: &Item) {
        let Item::Image { transform, x, y, w, h, mime, data, .. } = item else { return };
        let (x, y, w, h, data) = (*x, *y, *w, *h, data.as_deref());
        let decoded = data
            .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
            .and_then(|bytes| decode_image(mime, &bytes));
        let Some(img) = decoded else {
            let r = [
                Seg::M { x, y },
                Seg::L { x: x + w, y },
                Seg::L { x: x + w, y: y + h },
                Seg::L { x, y: y + h },
                Seg::Z,
            ];
            let segs: Vec<Seg> = r.iter().map(|s| s.map(|p| transform.apply(p))).collect();
            self.fill(&segs, &Paint::Solid { color: Rgba(238, 238, 238, 255) }, None);
            return;
        };
        let (iw, ih) = (f64::from(img.width()), f64::from(img.height()));
        let at = self.page.then(*transform).then(Affine::translate(x, y + h)).then(Affine::scale(w / iw, -h / ih));
        let paint = sk::PixmapPaint { quality: sk::FilterQuality::Bicubic, ..Default::default() };
        self.pixmap.draw_pixmap(0, 0, img.as_ref(), &paint, ts(&at), None);
    }
}

/// Separable box blur (three passes ≈ Gaussian) of premultiplied RGBA.
fn blur(pixmap: &mut sk::Pixmap, sigma: f64) {
    let r = ((((12.0 * sigma * sigma / 3.0) + 1.0).sqrt() - 1.0) / 2.0).round().max(1.0) as usize;
    let (w, h) = (pixmap.width() as usize, pixmap.height() as usize);
    let data = pixmap.data_mut();
    let mut tmp = vec![0u8; data.len()];
    let pass = |src: &[u8], dst: &mut [u8], horizontal: bool| {
        let (outer, inner) = if horizontal { (h, w) } else { (w, h) };
        let at = |o: usize, i: usize| if horizontal { (o * w + i) * 4 } else { (i * w + o) * 4 };
        let win = (2 * r + 1) as u32;
        for o in 0..outer {
            for c in 0..4 {
                let mut sum: u32 = 0;
                for i in 0..=r.min(inner - 1) {
                    sum += u32::from(src[at(o, i) + c]);
                }
                for i in 0..inner {
                    dst[at(o, i) + c] = (sum / win) as u8;
                    let add = i + r + 1;
                    if add < inner {
                        sum += u32::from(src[at(o, add) + c]);
                    }
                    if i >= r {
                        sum -= u32::from(src[at(o, i - r) + c]);
                    }
                }
            }
        }
    };
    for _ in 0..3 {
        pass(data, &mut tmp, true);
        pass(&tmp, data, false);
    }
}

/// Renders a page at `dpi` pixels per inch. `layouts` are the text layouts
/// indexed like `dl.items` (`None` skips that text).
pub fn render(dl: &DisplayList, layouts: &[Option<TextLayout>], dpi: f64) -> Option<sk::Pixmap> {
    render_area(dl, layouts, dpi, [0.0, 0.0, dl.page.width, dl.page.height])
}

/// Renders the page region `[x0, y0, x1, y1]` (page inches, y up), such as
/// [`DisplayList::extents`] for the whole drawing including off-page shapes.
pub fn render_area(dl: &DisplayList, layouts: &[Option<TextLayout>], dpi: f64, area: [f64; 4]) -> Option<sk::Pixmap> {
    let (pw, ph) = (area[2] - area[0], area[3] - area[1]);
    let dpi = dpi.min(MAX_SIDE / pw.max(ph).max(0.01));
    let (w, h) = ((pw * dpi).round().max(1.0) as u32, (ph * dpi).round().max(1.0) as u32);
    let mut pixmap = sk::Pixmap::new(w, h)?;
    pixmap.fill(sk::Color::WHITE);
    let page = Affine([dpi, 0.0, 0.0, -dpi, -area[0] * dpi, area[3] * dpi]);
    let mut r = Raster { pixmap, page, glyphs: GlyphCache::default() };
    for (i, item) in dl.items.iter().enumerate() {
        match item {
            Item::Path { d, fill, stroke, blur: b, .. } if *b > 0.0 => {
                // Draw the shadow alone, blur it, then composite.
                let mut layer = Raster { pixmap: sk::Pixmap::new(w, h)?, page: r.page, glyphs: GlyphCache::default() };
                if let Some(f) = fill {
                    layer.fill(d, f, None);
                }
                if let Some(s) = stroke {
                    layer.stroke(d, s);
                }
                blur(&mut layer.pixmap, b / 2.0 * dpi);
                r.pixmap.draw_pixmap(
                    0,
                    0,
                    layer.pixmap.as_ref(),
                    &sk::PixmapPaint::default(),
                    sk::Transform::identity(),
                    None,
                );
            }
            Item::Path { d, fill, stroke, .. } => {
                if let Some(f) = fill {
                    r.fill(d, f, None);
                }
                if let Some(s) = stroke {
                    r.stroke(d, s);
                }
            }
            Item::Text { transform, height, background, .. } => {
                if let Some(Some(l)) = layouts.get(i) {
                    r.text(transform, *height, *background, l);
                }
            }
            Item::Image { .. } => r.image(item),
        }
    }
    Some(r.pixmap)
}

/// PNG bytes of a page.
pub fn png(dl: &DisplayList, layouts: &[Option<TextLayout>], dpi: f64) -> Option<Vec<u8>> {
    render(dl, layouts, dpi)?.encode_png().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_pixel_diagonal_hatches_are_antialiased() {
        let up = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80];
        let c = hatch_coverage(&up);
        assert_eq!((c[0][7], c[0][6], c[0][0], c[0][3]), (0.92, 0.25, 0.25, 0.0));
        // Other tiles are binary.
        let grid = [0xFF, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80];
        let c = hatch_coverage(&grid);
        assert!(c.iter().flatten().all(|v| *v == 0.0 || *v == 1.0));
    }

    #[test]
    fn renders_the_fixture() {
        let pkg = flowster_vsdx::OpcPackage::open(flowster_vsdx::fixture::minimal_vsdx()).unwrap();
        let doc = flowster_vsdx::visio::read_document(&pkg).unwrap();
        let dl = flowster_render::build_page(&doc, 0).unwrap();
        let mut engine = flowster_text::TextEngine::system();
        let layouts = engine.layout_display_list(&dl);
        let pm = render(&dl, &layouts, 48.0).unwrap();
        assert_eq!((pm.width(), pm.height()), (408, 528));
        // The box's left edge (x = 3.25 in) is drawn across its row (y = 5.5 in from the bottom).
        let row = 528 - (5.5 * 48.0) as u32;
        let dark = (150..162).any(|x| pm.pixel(x, row).is_some_and(|p| p.red() < 200));
        assert!(dark, "box outline not drawn");
        assert!(png(&dl, &layouts, 24.0).unwrap().starts_with(b"\x89PNG"));
    }
}
