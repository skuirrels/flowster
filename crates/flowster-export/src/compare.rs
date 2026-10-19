//! Image comparison for the conformance harness (SPEC §22.3, M1 exit
//! criterion: corpus pages at SSIM ≥ 0.95 against the oracle's rendering).
//!
//! Visio's `Page.Export` crops to the drawing's extents, so pages are compared
//! by content: both images are reduced to luma on white, our rendering is cropped
//! to its content box and placed over the reference's content box, a small shift
//! search absorbs sub-pixel differences in where the extents round, and SSIM is
//! taken over the reference frame.

use tiny_skia as sk;

/// A single-channel image, luma in 0..=1 composited over white.
#[derive(Clone)]
pub struct Gray {
    pub width: usize,
    pub height: usize,
    pub px: Vec<f32>,
}

impl Gray {
    pub fn from_pixmap(p: &sk::Pixmap) -> Gray {
        let px = p
            .pixels()
            .iter()
            .map(|c| {
                // Premultiplied over white: c + (1 - a).
                let a = f32::from(c.alpha()) / 255.0;
                let ch = |v: u8| f32::from(v) / 255.0 + (1.0 - a);
                0.299 * ch(c.red()) + 0.587 * ch(c.green()) + 0.114 * ch(c.blue())
            })
            .collect();
        Gray { width: p.width() as usize, height: p.height() as usize, px }
    }

    pub fn decode_png(bytes: &[u8]) -> Option<Gray> {
        sk::Pixmap::decode_png(bytes).ok().map(|p| Gray::from_pixmap(&p))
    }

    pub fn encode_png(&self) -> Option<Vec<u8>> {
        self.to_pixmap()?.encode_png().ok()
    }

    pub fn white(width: usize, height: usize) -> Gray {
        Gray { width, height, px: vec![1.0; width * height] }
    }

    fn at(&self, x: isize, y: isize) -> f32 {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            1.0
        } else {
            self.px[y as usize * self.width + x as usize]
        }
    }

    /// The bounding box of non-white pixels: (x0, y0, x1, y1), exclusive ends.
    pub fn content_box(&self) -> Option<(usize, usize, usize, usize)> {
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for y in 0..self.height {
            for x in 0..self.width {
                if self.px[y * self.width + x] < 0.98 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        (x0 < x1).then_some((x0, y0, x1, y1))
    }

    /// The `w × h` region at `(x, y)` (out-of-range pixels are white).
    pub fn crop(&self, x: isize, y: isize, w: usize, h: usize) -> Gray {
        let mut out = Gray::white(w, h);
        for j in 0..h {
            for i in 0..w {
                out.px[j * w + i] = self.at(x + i as isize, y + j as isize);
            }
        }
        out
    }

    /// Back to an opaque grey pixmap (for diff images).
    pub fn to_pixmap(&self) -> Option<sk::Pixmap> {
        let mut p = sk::Pixmap::new(self.width as u32, self.height as u32)?;
        for (d, &v) in p.pixels_mut().iter_mut().zip(&self.px) {
            let g = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            *d = sk::PremultipliedColorU8::from_rgba(g, g, g, 255).unwrap_or(*d);
        }
        Some(p)
    }
}

/// Separable Gaussian blur (σ = 1.5, 11 taps), clamped at the edges.
fn gauss(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    const R: isize = 5;
    let sigma = 1.5f32;
    let k: Vec<f32> = (-R..=R).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f32 = k.iter().sum();
    let k: Vec<f32> = k.iter().map(|v| v / sum).collect();
    let mut tmp = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (t, kv) in k.iter().enumerate() {
                let xx = (x as isize + t as isize - R).clamp(0, w as isize - 1) as usize;
                acc += kv * src[y * w + xx];
            }
            tmp[y * w + x] = acc;
        }
    }
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (t, kv) in k.iter().enumerate() {
                let yy = (y as isize + t as isize - R).clamp(0, h as isize - 1) as usize;
                acc += kv * tmp[yy * w + x];
            }
            out[y * w + x] = acc;
        }
    }
    out
}

/// Mean SSIM and the per-pixel SSIM map of two same-sized images.
pub fn ssim(a: &Gray, b: &Gray) -> (f64, Vec<f32>) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let (w, h) = (a.width, a.height);
    let (c1, c2) = (0.01f32 * 0.01, 0.03f32 * 0.03);
    let mul = |p: &[f32], q: &[f32]| p.iter().zip(q).map(|(x, y)| x * y).collect::<Vec<f32>>();
    let ma = gauss(&a.px, w, h);
    let mb = gauss(&b.px, w, h);
    let saa = gauss(&mul(&a.px, &a.px), w, h);
    let sbb = gauss(&mul(&b.px, &b.px), w, h);
    let sab = gauss(&mul(&a.px, &b.px), w, h);
    let mut map = Vec::with_capacity(w * h);
    let mut total = 0.0f64;
    for i in 0..w * h {
        let (mx, my) = (ma[i], mb[i]);
        let vx = saa[i] - mx * mx;
        let vy = sbb[i] - my * my;
        let cxy = sab[i] - mx * my;
        let s = ((2.0 * mx * my + c1) * (2.0 * cxy + c2)) / ((mx * mx + my * my + c1) * (vx + vy + c2));
        map.push(s);
        total += f64::from(s);
    }
    (total / (w * h).max(1) as f64, map)
}

/// An 8-pixel box average: flattens 8 × 8 hatch tiles to their mean so that
/// alignment follows the geometry rather than the texture.
fn box8(g: &Gray) -> Gray {
    let (w, h) = (g.width, g.height);
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            tmp[y * w + x] = (0..8).map(|k| g.at(x as isize + k - 4, y as isize)).sum::<f32>() / 8.0;
        }
    }
    let t = Gray { width: w, height: h, px: tmp };
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = (0..8).map(|k| t.at(x as isize, y as isize + k - 4)).sum::<f32>() / 8.0;
        }
    }
    Gray { width: w, height: h, px: out }
}

/// Mean absolute difference of `ours` shifted by (dx, dy) against `reference`.
fn mad(reference: &Gray, ours: &Gray, dx: isize, dy: isize) -> f64 {
    let mut acc = 0.0f64;
    for y in 0..reference.height {
        for x in 0..reference.width {
            acc += f64::from((reference.px[y * reference.width + x] - ours.at(x as isize - dx, y as isize - dy)).abs());
        }
    }
    acc / (reference.width * reference.height).max(1) as f64
}

/// The result of comparing a rendering with a reference image.
pub struct Comparison {
    pub ssim: f64,
    /// Our rendering aligned to the reference frame.
    pub aligned: Gray,
    /// Per-pixel SSIM (1 = identical neighbourhood).
    pub map: Vec<f32>,
    /// Mean absolute difference after alignment, with hatch texture averaged
    /// out: the geometric fit, for choosing between candidate scales.
    pub mad: f64,
    /// Where the reference's top-left pixel falls in `ours`.
    pub offset: (isize, isize),
    /// Content-box sizes in pixels: reference, ours.
    pub ref_box: (usize, usize),
    pub our_box: (usize, usize),
}

/// Aligns `ours` (a full-page rendering) with `reference` (a cropped export)
/// by their content boxes and compares them.
pub fn compare(reference: &Gray, ours: &Gray) -> Option<Comparison> {
    let (rx0, ry0, rx1, ry1) = reference.content_box()?;
    let (ox0, oy0, ox1, oy1) = ours.content_box()?;
    // Place our content box's origin on the reference's, then let the
    // geometry (hatches averaged out) settle the last pixels.
    let (px, py) = (ox0 as isize - rx0 as isize, oy0 as isize - ry0 as isize);
    let placed = ours.crop(px, py, reference.width, reference.height);
    let (rb, pb) = (box8(reference), box8(&placed));
    let mut best = (f64::MAX, 0, 0);
    for dy in -3..=3 {
        for dx in -3..=3 {
            let m = mad(&rb, &pb, dx, dy);
            if m < best.0 {
                best = (m, dx, dy);
            }
        }
    }
    let aligned = placed.crop(-best.1, -best.2, reference.width, reference.height);
    let (s, map) = ssim(reference, &aligned);
    Some(Comparison {
        ssim: s,
        aligned,
        map,
        mad: best.0,
        offset: (px - best.1, py - best.2),
        ref_box: (rx1 - rx0, ry1 - ry0),
        our_box: (ox1 - ox0, oy1 - oy0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_score_one_and_shifts_are_absorbed() {
        let mut a = Gray::white(64, 48);
        for y in 10..30 {
            for x in 12..40 {
                a.px[y * 64 + x] = if (x + y) % 7 == 0 { 0.0 } else { 0.5 };
            }
        }
        assert!((ssim(&a, &a).0 - 1.0).abs() < 1e-6);
        // `ours` is the same picture on a bigger page at another offset.
        let ours = a.crop(-20, -15, 120, 90);
        let c = compare(&a, &ours).unwrap();
        assert!(c.ssim > 0.999, "{}", c.ssim);
        let blank = Gray::white(64, 48);
        assert!(ssim(&a, &blank).0 < 0.9);
    }
}
