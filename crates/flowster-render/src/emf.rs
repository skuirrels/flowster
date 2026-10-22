//! Enhanced metafiles (EMF, [MS-EMF]) as display-list items, so pictures
//! pasted into Visio render as vectors in every backend instead of a placeholder.
//!
//! The interpreter keeps a GDI device context (world transform, mapping mode,
//! selected pen/brush/font, path bracket, saved states) and turns drawing
//! records into paths, text and images in page inches. Visio maps the header's
//! frame (converted to reference-device pixels) onto the image rectangle; that
//! is the `to_page` transform the caller passes.
//!
//! Not yet: clipping, ROP codes, the winding fill mode (paths fill even-odd),
//! hatched brushes (drawn solid), arc direction, and EMF+ records (a dual
//! EMF+ file still draws from its GDI records).

use std::collections::BTreeMap;

use base64::Engine as _;
use flowster_core::geom::Seg;
use flowster_core::transform::Affine;
use flowster_core::value::Rgba;
use libm::{cos, hypot, sin};

use crate::paint::Paint;
use crate::{Item, Paragraph, Run, Stroke};

/// Header facts needed to place the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Header {
    /// Picture frame in reference-device pixels: x0, y0, x1, y1.
    pub frame: [f64; 4],
}

fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn i16_at(b: &[u8], at: usize) -> Option<i16> {
    Some(i16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn f32_at(b: &[u8], at: usize) -> Option<f64> {
    Some(f64::from(f32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?)))
}

/// Reads the EMF header; `None` if `bytes` is not an EMF.
pub fn header(bytes: &[u8]) -> Option<Header> {
    if u32_at(bytes, 0)? != 1 || u32_at(bytes, 40)? != 0x464D_4520 {
        return None;
    }
    let frame = [i32_at(bytes, 24)?, i32_at(bytes, 28)?, i32_at(bytes, 32)?, i32_at(bytes, 36)?];
    let (dev_x, dev_y) = (f64::from(i32_at(bytes, 72)?), f64::from(i32_at(bytes, 76)?));
    let (mm_x, mm_y) = (f64::from(i32_at(bytes, 80)?), f64::from(i32_at(bytes, 84)?));
    if mm_x <= 0.0 || mm_y <= 0.0 {
        return None;
    }
    // Frame is in 0.01 mm; device pixels per mm from the reference device.
    let (kx, ky) = (dev_x / mm_x / 100.0, dev_y / mm_y / 100.0);
    Some(Header {
        frame: [f64::from(frame[0]) * kx, f64::from(frame[1]) * ky, f64::from(frame[2]) * kx, f64::from(frame[3]) * ky],
    })
}

fn colorref(v: u32) -> Rgba {
    Rgba((v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, ((v >> 16) & 0xFF) as u8, 255)
}

#[derive(Clone, Debug)]
struct Pen {
    color: Rgba,
    /// Logical units; 0 = one device pixel.
    width: f64,
    cosmetic: bool,
    style: u32,
    cap: &'static str,
}

#[derive(Clone, Debug)]
struct Brush {
    color: Option<Rgba>,
}

#[derive(Clone, Debug)]
struct Font {
    height: f64,
    escapement: f64,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    face: String,
}

#[derive(Clone, Debug)]
enum Object {
    Pen(Pen),
    Brush(Brush),
    Font(Font),
}

#[derive(Clone, Debug)]
struct Dc {
    world: Affine,
    map_mode: u32,
    window_org: (f64, f64),
    window_ext: (f64, f64),
    viewport_org: (f64, f64),
    viewport_ext: (f64, f64),
    pen: Pen,
    brush: Brush,
    font: Option<Font>,
    text_color: Rgba,
    text_align: u32,
    pos: (f64, f64),
}

impl Default for Dc {
    fn default() -> Self {
        Dc {
            world: Affine::IDENTITY,
            map_mode: 1,
            window_org: (0.0, 0.0),
            window_ext: (1.0, 1.0),
            viewport_org: (0.0, 0.0),
            viewport_ext: (1.0, 1.0),
            pen: Pen { color: Rgba::BLACK, width: 0.0, cosmetic: true, style: 0, cap: "round" },
            brush: Brush { color: Some(Rgba(255, 255, 255, 255)) },
            font: None,
            text_color: Rgba::BLACK,
            text_align: 0,
            pos: (0.0, 0.0),
        }
    }
}

struct Interp<'a> {
    shape: u32,
    to_page: Affine,
    /// Reference-device pixels per mm, for the metric mapping modes.
    dev_per_mm: (f64, f64),
    dc: Dc,
    saved: Vec<Dc>,
    objects: BTreeMap<u32, Object>,
    path: Option<Vec<Seg>>,
    items: &'a mut Vec<Item>,
}

impl Interp<'_> {
    /// Logical → page transform for the current state.
    fn transform(&self) -> Affine {
        let d = &self.dc;
        let page_to_device = match d.map_mode {
            // MM_ISOTROPIC / MM_ANISOTROPIC: window → viewport.
            7 | 8 => {
                let (sx, sy) = (d.viewport_ext.0 / nonzero(d.window_ext.0), d.viewport_ext.1 / nonzero(d.window_ext.1));
                let (sx, sy) = if d.map_mode == 7 {
                    let s = sx.abs().min(sy.abs());
                    (s * sx.signum(), s * sy.signum())
                } else {
                    (sx, sy)
                };
                Affine::translate(d.viewport_org.0, d.viewport_org.1)
                    .then(Affine::scale(sx, sy))
                    .then(Affine::translate(-d.window_org.0, -d.window_org.1))
            }
            // Metric and English modes: fixed units, y up.
            2..=6 => {
                let mm = match d.map_mode {
                    2 => 0.1,
                    3 => 0.01,
                    4 => 0.254,
                    5 => 0.0254,
                    _ => 25.4 / 1440.0,
                };
                Affine::translate(d.viewport_org.0, d.viewport_org.1)
                    .then(Affine::scale(mm * self.dev_per_mm.0, -mm * self.dev_per_mm.1))
                    .then(Affine::translate(-d.window_org.0, -d.window_org.1))
            }
            // MM_TEXT: logical units are device pixels.
            _ => Affine::translate(d.viewport_org.0 - d.window_org.0, d.viewport_org.1 - d.window_org.1),
        };
        self.to_page.then(page_to_device).then(d.world)
    }

    fn pt(&self, p: (f64, f64)) -> (f64, f64) {
        self.transform().apply(p)
    }

    fn stroke(&self) -> Option<Stroke> {
        let pen = &self.dc.pen;
        if pen.style & 0xF == 5 {
            return None;
        }
        let t = self.transform();
        // Cosmetic pens are one reference-device pixel wide.
        let width =
            if pen.cosmetic || pen.width <= 0.0 { self.to_page.mean_scale() } else { pen.width * t.mean_scale() };
        let unit = width.max(self.to_page.mean_scale());
        let dash: Vec<f64> = match pen.style & 0xF {
            1 => vec![3.0, 1.0],
            2 => vec![1.0, 1.0],
            3 => vec![3.0, 1.0, 1.0, 1.0],
            4 => vec![3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            _ => Vec::new(),
        }
        .into_iter()
        .map(|v| v * unit)
        .collect();
        Some(Stroke { color: pen.color, width, cap: if dash.is_empty() { pen.cap } else { "butt" }, dash })
    }

    fn fill(&self) -> Option<Paint> {
        self.dc.brush.color.map(|color| Paint::Solid { color })
    }

    fn emit(&mut self, d: Vec<Seg>, fill: bool, stroke: bool) {
        if d.len() < 2 {
            return;
        }
        if let Some(path) = &mut self.path {
            path.extend(d);
            return;
        }
        let fill = if fill { self.fill() } else { None };
        let stroke = if stroke { self.stroke() } else { None };
        if fill.is_some() || stroke.is_some() {
            self.items.push(Item::Path { shape: self.shape, d, fill, stroke, blur: 0.0 });
        }
    }

    fn poly(&self, pts: &[(f64, f64)], close: bool) -> Vec<Seg> {
        let mut d: Vec<Seg> = pts
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let (x, y) = self.pt(*p);
                if i == 0 { Seg::M { x, y } } else { Seg::L { x, y } }
            })
            .collect();
        if close && !d.is_empty() {
            d.push(Seg::Z);
        }
        d
    }

    /// Cubic Béziers: `pts` = start, then (c1, c2, end) triples.
    fn bezier(&self, start: Option<(f64, f64)>, pts: &[(f64, f64)]) -> Vec<Seg> {
        let mut d = Vec::new();
        let mut rest = pts;
        let first = match start {
            Some(s) => s,
            None => {
                let Some((&s, r)) = pts.split_first() else { return d };
                rest = r;
                s
            }
        };
        let (x, y) = self.pt(first);
        d.push(Seg::M { x, y });
        for [c1, c2, end] in rest.as_chunks::<3>().0 {
            let ((x1, y1), (x2, y2), (x, y)) = (self.pt(*c1), self.pt(*c2), self.pt(*end));
            d.push(Seg::C { x1, y1, x2, y2, x, y });
        }
        d
    }

    /// An ellipse inscribed in a box, as four cubic arcs.
    fn ellipse(&self, l: f64, t: f64, r: f64, b: f64) -> Vec<Seg> {
        const K: f64 = 0.552_284_749_830_793_4;
        let (cx, cy, rx, ry) = ((l + r) / 2.0, (t + b) / 2.0, (r - l) / 2.0, (b - t) / 2.0);
        let p = |x: f64, y: f64| self.pt((x, y));
        let mut d = Vec::new();
        let (x, y) = p(cx + rx, cy);
        d.push(Seg::M { x, y });
        for (c1, c2, e) in [
            ((cx + rx, cy + K * ry), (cx + K * rx, cy + ry), (cx, cy + ry)),
            ((cx - K * rx, cy + ry), (cx - rx, cy + K * ry), (cx - rx, cy)),
            ((cx - rx, cy - K * ry), (cx - K * rx, cy - ry), (cx, cy - ry)),
            ((cx + K * rx, cy - ry), (cx + rx, cy - K * ry), (cx + rx, cy)),
        ] {
            let ((x1, y1), (x2, y2), (x, y)) = (p(c1.0, c1.1), p(c2.0, c2.1), p(e.0, e.1));
            d.push(Seg::C { x1, y1, x2, y2, x, y });
        }
        d.push(Seg::Z);
        d
    }

    fn round_rect(&self, l: f64, t: f64, r: f64, b: f64, w: f64, h: f64) -> Vec<Seg> {
        const K: f64 = 0.552_284_749_830_793_4;
        let (rx, ry) = ((w / 2.0).min((r - l) / 2.0), (h / 2.0).min((b - t) / 2.0));
        let p = |x: f64, y: f64| self.pt((x, y));
        let mut d = Vec::new();
        let (x, y) = p(l + rx, t);
        d.push(Seg::M { x, y });
        let corners = [
            ((r - rx, t), (r - rx + K * rx, t), (r, t + ry - K * ry), (r, t + ry)),
            ((r, b - ry), (r, b - ry + K * ry), (r - rx + K * rx, b), (r - rx, b)),
            ((l + rx, b), (l + rx - K * rx, b), (l, b - ry + K * ry), (l, b - ry)),
            ((l, t + ry), (l, t + ry - K * ry), (l + rx - K * rx, t), (l + rx, t)),
        ];
        for (a, c1, c2, e) in corners {
            let (x, y) = p(a.0, a.1);
            d.push(Seg::L { x, y });
            let ((x1, y1), (x2, y2), (x, y)) = (p(c1.0, c1.1), p(c2.0, c2.1), p(e.0, e.1));
            d.push(Seg::C { x1, y1, x2, y2, x, y });
        }
        d.push(Seg::Z);
        d
    }

    /// Arc of the ellipse in a box, counter-clockwise (on screen) from the
    /// radial through `start` to the one through `end`; `kind` 0 arc, 1 chord, 2 pie.
    fn arc(&self, b: [f64; 4], start: (f64, f64), end: (f64, f64), kind: u8) -> Vec<Seg> {
        let (cx, cy, rx, ry) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0, (b[2] - b[0]) / 2.0, (b[3] - b[1]) / 2.0);
        // Angles measured with y up (screen counter-clockwise).
        let ang = |p: (f64, f64)| libm::atan2(-(p.1 - cy) / ry.abs().max(1e-9), (p.0 - cx) / rx.abs().max(1e-9));
        let (a0, mut a1) = (ang(start), ang(end));
        if a1 <= a0 {
            a1 += 2.0 * core::f64::consts::PI;
        }
        let at = |a: f64| self.pt((cx + rx * cos(a), cy - ry * sin(a)));
        let steps = (((a1 - a0) / (core::f64::consts::PI / 16.0)).ceil() as usize).max(2);
        let mut d = Vec::with_capacity(steps + 3);
        if kind == 2 {
            let (x, y) = self.pt((cx, cy));
            d.push(Seg::M { x, y });
        }
        for i in 0..=steps {
            let (x, y) = at(a0 + (a1 - a0) * i as f64 / steps as f64);
            d.push(if i == 0 && kind != 2 { Seg::M { x, y } } else { Seg::L { x, y } });
        }
        if kind > 0 {
            d.push(Seg::Z);
        }
        d
    }

    fn text(&mut self, reference: (f64, f64), s: &str) {
        let Some(font) = self.dc.font.clone() else { return };
        if s.trim().is_empty() {
            return;
        }
        let t = self.transform();
        let origin = t.apply(reference);
        // Baseline direction and "up", in page space (escapement is CCW on screen).
        let e = font.escapement.to_radians();
        let along = t.apply((reference.0 + cos(e), reference.1 - sin(e)));
        let above = t.apply((reference.0 - sin(e), reference.1 - cos(e)));
        let (dx, dy) = (along.0 - origin.0, along.1 - origin.1);
        let (ux, uy) = (above.0 - origin.0, above.1 - origin.1);
        let (ld, lu) = (hypot(dx, dy), hypot(ux, uy));
        if ld < 1e-12 || lu < 1e-12 {
            return;
        }
        // Negative heights are the em size; positive ones the cell (~1.15 em).
        let em = if font.height < 0.0 { -font.height } else { font.height / 1.15 };
        let size = em * lu;
        if size <= 0.0 {
            return;
        }
        let (dir, up) = ((dx / ld, dy / ld), (ux / lu, uy / lu));
        let width = size * (s.chars().count() as f64 + 2.0);
        let height = size;
        let align = match self.dc.text_align & 6 {
            6 => 1,
            2 => 2,
            _ => 0,
        };
        let ax = match align {
            1 => width / 2.0,
            2 => width,
            _ => 0.0,
        };
        // The layout puts the baseline ~0.78 em below the block top; GDI's
        // reference point is the baseline, the cell top (~0.9 em above it)
        // or the cell bottom (~0.21 em below it).
        let baseline = height - 0.78 * size;
        let ay = match self.dc.text_align & 24 {
            24 => baseline,
            8 => baseline - 0.21 * size,
            _ => baseline + 0.9 * size,
        };
        // Text follows the picture's stretch: the block is laid out at the
        // vertical size and scaled along the baseline by the aspect change.
        let k = ld / lu;
        let (bx, by) = (dir.0 * k, dir.1 * k);
        let transform = Affine([bx, by, up.0, up.1, origin.0 - bx * ax - up.0 * ay, origin.1 - by * ax - up.1 * ay]);
        self.items.push(Item::Text {
            shape: self.shape,
            transform,
            width,
            height,
            margins: [0.0; 4],
            valign: 0,
            background: None,
            paragraphs: vec![Paragraph {
                align,
                line_spacing: -1.0,
                space_before: 0.0,
                space_after: 0.0,
                indent_left: 0.0,
                indent_first: 0.0,
                bullet: None,
                text_after_bullet: 0.0,
                runs: vec![Run {
                    text: s.to_owned(),
                    font: if font.face.is_empty() { "Arial".into() } else { font.face },
                    size,
                    color: self.dc.text_color,
                    bold: font.bold,
                    italic: font.italic,
                    underline: font.underline,
                    strike: font.strike,
                }],
            }],
        });
    }

    /// A device-independent bitmap stretched over a destination rectangle.
    fn dib(&mut self, dest: [f64; 4], bmi: &[u8], bits: &[u8], src: Option<[i32; 4]>) {
        let Some(png) = dib_to_png(bmi, bits, src) else { return };
        let [x, y, w, h] = dest;
        // Local space: logical coordinates with y flipped, so the image's top
        // edge (smaller logical y) is its local top.
        let transform = self.transform().then(Affine([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]));
        self.items.push(Item::Image {
            shape: self.shape,
            transform,
            x,
            y: -(y + h),
            w,
            h,
            mime: "image/png".into(),
            data: Some(base64::engine::general_purpose::STANDARD.encode(png)),
        });
    }

    fn select(&mut self, ih: u32) {
        if ih & 0x8000_0000 != 0 {
            let stock = ih & 0x7FFF_FFFF;
            let grey = |v: u8| Some(Rgba(v, v, v, 255));
            match stock {
                0 => self.dc.brush = Brush { color: grey(255) },
                1 => self.dc.brush = Brush { color: grey(192) },
                2 => self.dc.brush = Brush { color: grey(128) },
                3 => self.dc.brush = Brush { color: grey(64) },
                4 => self.dc.brush = Brush { color: grey(0) },
                5 => self.dc.brush = Brush { color: None },
                6 | 7 => {
                    let color = if stock == 6 { Rgba(255, 255, 255, 255) } else { Rgba::BLACK };
                    self.dc.pen = Pen { color, width: 0.0, cosmetic: true, style: 0, cap: "round" };
                }
                8 => self.dc.pen.style = 5,
                _ => {}
            }
            return;
        }
        match self.objects.get(&ih) {
            Some(Object::Pen(p)) => self.dc.pen = p.clone(),
            Some(Object::Brush(b)) => self.dc.brush = b.clone(),
            Some(Object::Font(f)) => self.dc.font = Some(f.clone()),
            None => {}
        }
    }

    fn record(&mut self, kind: u32, r: &[u8]) -> Option<()> {
        let point16 = |n: usize, at: usize| -> Option<Vec<(f64, f64)>> {
            (0..n).map(|i| Some((f64::from(i16_at(r, at + i * 4)?), f64::from(i16_at(r, at + i * 4 + 2)?)))).collect()
        };
        let point32 = |n: usize, at: usize| -> Option<Vec<(f64, f64)>> {
            (0..n).map(|i| Some((f64::from(i32_at(r, at + i * 8)?), f64::from(i32_at(r, at + i * 8 + 4)?)))).collect()
        };
        let rect = |at: usize| -> Option<[f64; 4]> {
            Some([
                f64::from(i32_at(r, at)?),
                f64::from(i32_at(r, at + 4)?),
                f64::from(i32_at(r, at + 8)?),
                f64::from(i32_at(r, at + 12)?),
            ])
        };
        let pair =
            |at: usize| -> Option<(f64, f64)> { Some((f64::from(i32_at(r, at)?), f64::from(i32_at(r, at + 4)?))) };
        match kind {
            // POLYBEZIER, POLYGON, POLYLINE, POLYBEZIERTO, POLYLINETO (32- and 16-bit).
            2..=6 | 85..=89 => {
                let n = u32_at(r, 24)? as usize;
                let pts = if kind >= 85 { point16(n, 28)? } else { point32(n, 28)? };
                match kind {
                    2 | 85 => {
                        let d = self.bezier(None, &pts);
                        self.emit(d, false, true);
                    }
                    3 | 86 => {
                        let d = self.poly(&pts, true);
                        self.emit(d, true, true);
                    }
                    4 | 87 => {
                        let d = self.poly(&pts, false);
                        self.emit(d, false, true);
                    }
                    5 | 88 => {
                        let d = self.bezier(Some(self.dc.pos), &pts);
                        self.dc.pos = *pts.last()?;
                        self.emit(d, false, true);
                    }
                    _ => {
                        let mut all = vec![self.dc.pos];
                        all.extend(&pts);
                        self.dc.pos = *pts.last()?;
                        let d = self.poly(&all, false);
                        self.emit(d, false, true);
                    }
                }
            }
            // POLYPOLYLINE, POLYPOLYGON (32- and 16-bit).
            7 | 8 | 90 | 91 => {
                let polys = u32_at(r, 24)? as usize;
                let total = u32_at(r, 28)? as usize;
                let counts: Vec<usize> =
                    (0..polys).map(|i| u32_at(r, 32 + i * 4).map(|c| c as usize)).collect::<Option<_>>()?;
                let at = 32 + polys * 4;
                let pts = if kind >= 90 { point16(total, at)? } else { point32(total, at)? };
                let closed = kind == 8 || kind == 91;
                let mut d = Vec::new();
                let mut i = 0;
                for c in counts {
                    d.extend(self.poly(pts.get(i..i + c)?, closed));
                    i += c;
                }
                self.emit(d, closed, true);
            }
            9 => self.dc.window_ext = pair(8)?,
            10 => self.dc.window_org = pair(8)?,
            11 => self.dc.viewport_ext = pair(8)?,
            12 => self.dc.viewport_org = pair(8)?,
            17 => self.dc.map_mode = u32_at(r, 8)?,
            22 => self.dc.text_align = u32_at(r, 8)?,
            24 => self.dc.text_color = colorref(u32_at(r, 8)?),
            27 => self.dc.pos = pair(8)?,
            33 => self.saved.push(self.dc.clone()),
            34 => {
                let n = i32_at(r, 8)?;
                let depth = if n < 0 { self.saved.len().checked_sub(n.unsigned_abs() as usize)? } else { n as usize };
                if depth < self.saved.len() {
                    self.saved.truncate(depth + 1);
                    self.dc = self.saved.pop()?;
                }
            }
            35 | 36 => {
                let x = Affine([
                    f32_at(r, 8)?,
                    f32_at(r, 12)?,
                    f32_at(r, 16)?,
                    f32_at(r, 20)?,
                    f32_at(r, 24)?,
                    f32_at(r, 28)?,
                ]);
                let mode = if kind == 35 { 4 } else { u32_at(r, 32)? };
                self.dc.world = match mode {
                    1 => Affine::IDENTITY,
                    2 => self.dc.world.then(x),
                    3 => x.then(self.dc.world),
                    _ => x,
                };
            }
            37 => self.select(u32_at(r, 8)?),
            // CREATEPEN
            38 => {
                let style = u32_at(r, 12)?;
                let width = f64::from(i32_at(r, 16)?);
                let pen = Pen { color: colorref(u32_at(r, 24)?), width, cosmetic: width <= 1.0, style, cap: "round" };
                self.objects.insert(u32_at(r, 8)?, Object::Pen(pen));
            }
            // CREATEBRUSHINDIRECT
            39 => {
                let style = u32_at(r, 12)?;
                let color = (style != 1).then(|| colorref(u32_at(r, 16).unwrap_or(0)));
                self.objects.insert(u32_at(r, 8)?, Object::Brush(Brush { color }));
            }
            40 => {
                self.objects.remove(&u32_at(r, 8)?);
            }
            42 => {
                let b = rect(8)?;
                let d = self.ellipse(b[0], b[1], b[2], b[3]);
                self.emit(d, true, true);
            }
            43 => {
                let b = rect(8)?;
                let d = self.poly(&[(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])], true);
                self.emit(d, true, true);
            }
            44 => {
                let b = rect(8)?;
                let (w, h) = pair(24)?;
                let d = self.round_rect(b[0], b[1], b[2], b[3], w, h);
                self.emit(d, true, true);
            }
            // ARC, CHORD, PIE, ARCTO
            45..=47 | 55 => {
                let b = rect(8)?;
                let (s, e) = (pair(24)?, pair(32)?);
                let k = match kind {
                    46 => 1,
                    47 => 2,
                    _ => 0,
                };
                let d = self.arc(b, s, e, k);
                self.emit(d, k > 0, true);
            }
            54 => {
                let to = pair(8)?;
                let d = self.poly(&[self.dc.pos, to], false);
                self.dc.pos = to;
                self.emit(d, false, true);
            }
            59 => self.path = Some(Vec::new()),
            60 => {}
            61 => {
                if let Some(p) = &mut self.path {
                    p.push(Seg::Z);
                }
            }
            // FILLPATH, STROKEANDFILLPATH, STROKEPATH
            62..=64 => {
                if let Some(d) = self.path.take() {
                    self.emit(d, kind != 64, kind != 62);
                }
            }
            // BITBLT / STRETCHBLT / STRETCHDIBITS
            76 | 77 => {
                let dest = [
                    f64::from(i32_at(r, 24)?),
                    f64::from(i32_at(r, 28)?),
                    f64::from(i32_at(r, 32)?),
                    f64::from(i32_at(r, 36)?),
                ];
                let (off_bmi, cb_bmi, off_bits, cb_bits) = (
                    u32_at(r, 84)? as usize,
                    u32_at(r, 88)? as usize,
                    u32_at(r, 92)? as usize,
                    u32_at(r, 96)? as usize,
                );
                if cb_bmi == 0 {
                    // A pattern blit: fill the rectangle with the brush.
                    let [x, y, w, h] = dest;
                    let d = self.poly(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)], true);
                    self.emit(d, true, false);
                } else {
                    let src = (kind == 77)
                        .then(|| Some([i32_at(r, 44)?, i32_at(r, 48)?, i32_at(r, 100)?, i32_at(r, 104)?]))
                        .flatten();
                    let (bmi, bits) = (r.get(off_bmi..off_bmi + cb_bmi)?, r.get(off_bits..off_bits + cb_bits)?);
                    self.dib(dest, bmi, bits, src);
                }
            }
            81 => {
                let (x, y) = (f64::from(i32_at(r, 24)?), f64::from(i32_at(r, 28)?));
                let src = [i32_at(r, 32)?, i32_at(r, 36)?, i32_at(r, 40)?, i32_at(r, 44)?];
                let (off_bmi, cb_bmi, off_bits, cb_bits) = (
                    u32_at(r, 48)? as usize,
                    u32_at(r, 52)? as usize,
                    u32_at(r, 56)? as usize,
                    u32_at(r, 60)? as usize,
                );
                let (w, h) = (f64::from(i32_at(r, 72)?), f64::from(i32_at(r, 76)?));
                let (bmi, bits) = (r.get(off_bmi..off_bmi + cb_bmi)?, r.get(off_bits..off_bits + cb_bits)?);
                self.dib([x, y, w, h], bmi, bits, Some(src));
            }
            // EXTCREATEFONTINDIRECTW
            82 => {
                let face: Vec<u16> = (0..32).map_while(|i| u16_at(r, 40 + i * 2).filter(|c| *c != 0)).collect();
                let font = Font {
                    height: f64::from(i32_at(r, 12)?),
                    escapement: f64::from(i32_at(r, 20)?) / 10.0,
                    bold: i32_at(r, 28)? >= 600,
                    italic: *r.get(32)? != 0,
                    underline: *r.get(33)? != 0,
                    strike: *r.get(34)? != 0,
                    face: String::from_utf16_lossy(&face),
                };
                self.objects.insert(u32_at(r, 8)?, Object::Font(font));
            }
            // EXTTEXTOUTA / EXTTEXTOUTW
            83 | 84 => {
                let reference = pair(36)?;
                let n = u32_at(r, 44)? as usize;
                let off = u32_at(r, 48)? as usize;
                let s = if kind == 84 {
                    let units: Vec<u16> = (0..n).map(|i| u16_at(r, off + i * 2)).collect::<Option<_>>()?;
                    String::from_utf16_lossy(&units)
                } else {
                    r.get(off..off + n)?.iter().map(|b| char::from(*b)).collect()
                };
                let reference = if self.dc.text_align & 1 != 0 { self.dc.pos } else { reference };
                self.text(reference, &s);
            }
            // EXTCREATEPEN
            95 => {
                // LOGPENEX after the bitmap offsets: style, width, brush style, colour.
                let style = u32_at(r, 28)?;
                let width = f64::from(u32_at(r, 32)?);
                let brush_style = u32_at(r, 36)?;
                let cap = match style & 0xF00 {
                    0x100 => "square",
                    0x200 => "butt",
                    _ => "round",
                };
                let geometric = style & 0x0001_0000 != 0;
                let pen = Pen {
                    color: colorref(u32_at(r, 40)?),
                    width,
                    cosmetic: !geometric,
                    style: if brush_style == 1 { 5 } else { style },
                    cap,
                };
                self.objects.insert(u32_at(r, 8)?, Object::Pen(pen));
            }
            _ => {}
        }
        Some(())
    }
}

fn nonzero(v: f64) -> f64 {
    if v == 0.0 { 1.0 } else { v }
}

/// Draws an EMF into `items`. `to_page` maps reference-device pixels (the
/// header frame's units) to page inches.
pub fn render(bytes: &[u8], shape: u32, to_page: Affine, items: &mut Vec<Item>) -> bool {
    let Some(_) = header(bytes) else { return false };
    let (dev_x, dev_y, mm_x, mm_y) = (
        f64::from(i32_at(bytes, 72).unwrap_or(1)),
        f64::from(i32_at(bytes, 76).unwrap_or(1)),
        f64::from(i32_at(bytes, 80).unwrap_or(1)).max(1.0),
        f64::from(i32_at(bytes, 84).unwrap_or(1)).max(1.0),
    );
    let mut it = Interp {
        shape,
        to_page,
        dev_per_mm: (dev_x / mm_x, dev_y / mm_y),
        dc: Dc::default(),
        saved: Vec::new(),
        objects: BTreeMap::new(),
        path: None,
        items,
    };
    let mut at = 0usize;
    while let (Some(kind), Some(size)) = (u32_at(bytes, at), u32_at(bytes, at + 4)) {
        let size = size as usize;
        if size < 8 || at + size > bytes.len() {
            break;
        }
        let _ = it.record(kind, &bytes[at..at + size]);
        if kind == 14 {
            break;
        }
        at += size;
    }
    true
}

// ------------------------------------------------------------------- DIBs

/// Decodes a DIB (BITMAPINFO + bits; BI_RGB/BI_BITFIELDS at 1, 4, 8, 16, 24
/// or 32 bits) to PNG, cropped to `src` (x, y, w, h) when given.
pub fn dib_to_png(bmi: &[u8], bits: &[u8], src: Option<[i32; 4]>) -> Option<Vec<u8>> {
    let header = u32_at(bmi, 0)? as usize;
    let (w, h) = (i32_at(bmi, 4)?, i32_at(bmi, 8)?);
    let bpp = u16_at(bmi, 14)?;
    let compression = u32_at(bmi, 16)?;
    if w <= 0 || h == 0 || w > 1 << 14 || h.unsigned_abs() > 1 << 14 || !matches!(compression, 0 | 3) {
        return None;
    }
    let (w, top_down, h) = (w as usize, h < 0, h.unsigned_abs() as usize);
    let used = u32_at(bmi, 32).unwrap_or(0) as usize;
    let palette_len = if bpp <= 8 { if used > 0 { used } else { 1 << bpp } } else { 0 };
    let masks_len = if compression == 3 && header == 40 { 12 } else { 0 };
    let pal_at = header + masks_len;
    let palette: Vec<[u8; 3]> = (0..palette_len)
        .map(|i| bmi.get(pal_at + i * 4..pal_at + i * 4 + 3).map(|c| [c[2], c[1], c[0]]))
        .collect::<Option<_>>()?;
    let stride = (w * usize::from(bpp)).div_ceil(32) * 4;
    let mut rgba = vec![0u8; w * h * 4];
    for row in 0..h {
        let src_row = if top_down { row } else { h - 1 - row };
        let line = bits.get(src_row * stride..src_row * stride + stride)?;
        for x in 0..w {
            let px = match bpp {
                1 | 4 | 8 => {
                    let per = 8 / usize::from(bpp);
                    let byte = line[x / per];
                    let shift = 8 - usize::from(bpp) * (x % per + 1);
                    let ix = usize::from((byte >> shift) & ((1u16 << bpp) - 1) as u8);
                    let c = palette.get(ix)?;
                    [c[0], c[1], c[2], 255]
                }
                16 => {
                    let v = u16::from_le_bytes([line[x * 2], line[x * 2 + 1]]);
                    let c5 = |s: u16| ((((v >> s) & 31) * 255 + 15) / 31) as u8;
                    [c5(10), c5(5), c5(0), 255]
                }
                24 => [line[x * 3 + 2], line[x * 3 + 1], line[x * 3], 255],
                32 => [line[x * 4 + 2], line[x * 4 + 1], line[x * 4], 255],
                _ => return None,
            };
            rgba[(row * w + x) * 4..(row * w + x) * 4 + 4].copy_from_slice(&px);
        }
    }
    let (cx, cy, cw, ch) = match src {
        Some([x, y, sw, sh]) if sw > 0 && sh > 0 => {
            let (x, y) = (x.max(0) as usize, y.max(0) as usize);
            (x.min(w), y.min(h), (sw as usize).min(w.saturating_sub(x)), (sh as usize).min(h.saturating_sub(y)))
        }
        _ => (0, 0, w, h),
    };
    if cw == 0 || ch == 0 {
        return None;
    }
    let mut cropped = Vec::with_capacity(cw * ch * 4);
    for row in cy..cy + ch {
        cropped.extend_from_slice(&rgba[(row * w + cx) * 4..(row * w + cx + cw) * 4]);
    }
    Some(encode_png(cw, ch, &cropped))
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// A PNG of RGBA pixels with stored (uncompressed) deflate blocks: small
/// metafile bitmaps need no compressor.
fn encode_png(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(h * (w * 4 + 1));
    for row in rgba.chunks_exact(w * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let n = block.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        png.extend_from_slice(&body);
        png.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    png
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_png_signature() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        let png = encode_png(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]);
        assert_eq!(&png[1..4], b"PNG");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn a_tiny_metafile_draws_a_rectangle() {
        // HEADER (108 bytes) + RECTANGLE + EOF, frame 0..1000 x 0..1000 (0.01 mm), 100 px per 10 mm.
        let mut emf = Vec::new();
        let mut rec = |kind: u32, body: &[i32]| {
            emf.extend_from_slice(&kind.to_le_bytes());
            emf.extend_from_slice(&((8 + body.len() * 4) as u32).to_le_bytes());
            for v in body {
                emf.extend_from_slice(&v.to_le_bytes());
            }
        };
        let mut h = vec![0i32; 25];
        h[4..8].copy_from_slice(&[0, 0, 1000, 1000]);
        h[8] = 0x464D_4520;
        h[16..20].copy_from_slice(&[100, 100, 10, 10]);
        rec(1, &h);
        rec(43, &[10, 10, 90, 50]);
        rec(14, &[0, 16, 20]);
        let hd = header(&emf).unwrap();
        assert_eq!(hd.frame, [0.0, 0.0, 100.0, 100.0]);
        let mut items = Vec::new();
        // One device pixel = 0.01 in, y flipped.
        assert!(render(&emf, 1, Affine([0.01, 0.0, 0.0, -0.01, 0.0, 1.0]), &mut items));
        let Item::Path { d, fill, stroke, .. } = &items[0] else { panic!("{items:?}") };
        assert!(fill.is_some() && stroke.is_some());
        assert_eq!(d[0], Seg::M { x: 0.1, y: 0.9 });
        assert_eq!(d[2], Seg::L { x: 0.9, y: 0.5 });
    }
}
