//! `flowster-export`: raster and PDF output of display lists (SPEC §19.3).
//!
//! Both exporters consume the same display list and the same text layouts
//! (`flowster-text`), so geometry, paints and line breaks match the canvas.
//! PNG goes through tiny-skia, PDF through krilla (embedded, subset fonts with
//! real, searchable text).

#![forbid(unsafe_code)]

mod glyphs;
pub mod pdf;
pub mod png;

use flowster_core::geom::Seg;
use flowster_core::value::Rgba;
use flowster_render::Stop;

pub use glyphs::GlyphCache;

/// Stops reflected about the origin of a rectangular gradient: offset s → 0.5 ± s/2.
pub(crate) fn mirrored(stops: &[Stop]) -> Vec<Stop> {
    let mut out: Vec<Stop> =
        stops.iter().rev().map(|s| Stop { offset: 0.5 - s.offset / 2.0, color: s.color }).collect();
    out.extend(stops.iter().map(|s| Stop { offset: 0.5 + s.offset / 2.0, color: s.color }));
    out
}

/// The "bow tie" where |dy| ≥ |dx| around `(ox, oy)` (rectangular gradients).
pub(crate) fn bow_tie(ox: f64, oy: f64) -> Vec<Seg> {
    vec![
        Seg::M { x: ox, y: oy },
        Seg::L { x: ox + 4.0, y: oy + 4.0 },
        Seg::L { x: ox - 4.0, y: oy + 4.0 },
        Seg::Z,
        Seg::M { x: ox, y: oy },
        Seg::L { x: ox + 4.0, y: oy - 4.0 },
        Seg::L { x: ox - 4.0, y: oy - 4.0 },
        Seg::Z,
    ]
}

pub(crate) fn alpha(c: Rgba) -> f32 {
    f32::from(c.3) / 255.0
}

/// Decodes an embedded image (PNG or JPEG) to premultiplied RGBA.
pub(crate) fn decode_image(mime: &str, bytes: &[u8]) -> Option<tiny_skia::Pixmap> {
    match mime {
        "image/png" => tiny_skia::Pixmap::decode_png(bytes).ok(),
        "image/jpeg" => {
            let opts = zune_jpeg::zune_core::options::DecoderOptions::default()
                .jpeg_set_out_colorspace(zune_jpeg::zune_core::colorspace::ColorSpace::RGBA);
            let mut dec = zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(bytes), opts);
            let px = dec.decode().ok()?;
            let info = dec.info()?;
            tiny_skia::Pixmap::from_vec(px, tiny_skia::IntSize::from_wh(u32::from(info.width), u32::from(info.height))?)
        }
        _ => None,
    }
}
