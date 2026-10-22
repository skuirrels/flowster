//! SVG export of a display list (SPEC §19.3). Coordinates are page inches with
//! +Y up, so the whole drawing sits in a group that flips Y.
//!
//! Text uses an estimated advance width (no font metrics in core yet), so line
//! breaks can differ from the canvas until core text layout lands.

use std::fmt::Write as _;

use flowster_core::geom::Seg;
use flowster_core::transform::Affine;
use flowster_core::value::Rgba;

use crate::paint::{Paint, Stop};
use crate::text_layout::TextLayout;
use crate::{DisplayList, Item, Paragraph};

/// Output pixels per inch for the SVG's width/height attributes.
const PX_PER_IN: f64 = 96.0;
/// Rough average advance of proportional fonts, as a fraction of the font size.
const AVG_ADVANCE: f64 = 0.5;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn color(c: Rgba) -> String {
    c.hex()
}

fn opacity(c: Rgba) -> Option<String> {
    (c.3 != 255).then(|| format!("{:.3}", f64::from(c.3) / 255.0))
}

fn path_d(segs: &[Seg]) -> String {
    let mut d = String::new();
    for s in segs {
        match *s {
            Seg::M { x, y } => write!(d, "M{x:.5} {y:.5}"),
            Seg::L { x, y } => write!(d, "L{x:.5} {y:.5}"),
            Seg::C { x1, y1, x2, y2, x, y } => write!(d, "C{x1:.5} {y1:.5} {x2:.5} {y2:.5} {x:.5} {y:.5}"),
            Seg::Z => write!(d, "Z"),
        }
        .ok();
    }
    d
}

fn matrix(a: &Affine) -> String {
    let [a, b, c, d, e, f] = a.0;
    format!("matrix({a:.6} {b:.6} {c:.6} {d:.6} {e:.6} {f:.6})")
}

/// Greedy word wrap with estimated widths. Returns lines of (run index, text).
fn wrap(p: &Paragraph, max_w: f64) -> Vec<Vec<(usize, String)>> {
    let mut lines: Vec<Vec<(usize, String)>> = vec![Vec::new()];
    let mut width = 0.0;
    for (ri, run) in p.runs.iter().enumerate() {
        let adv = run.size * AVG_ADVANCE;
        for (wi, word) in run.text.split(' ').enumerate() {
            let token = if wi == 0 { word.to_owned() } else { format!(" {word}") };
            let tw = token.chars().count() as f64 * adv;
            if width + tw > max_w && width > 0.0 {
                lines.push(Vec::new());
                width = 0.0;
                let t = token.trim_start().to_owned();
                width += t.chars().count() as f64 * adv;
                lines.last_mut().unwrap().push((ri, t));
            } else {
                width += tw;
                lines.last_mut().unwrap().push((ri, token));
            }
        }
    }
    lines
}

fn stops_xml(stops: &[Stop]) -> String {
    let mut out = String::new();
    for st in stops {
        write!(out, r#"<stop offset="{:.4}" stop-color="{}""#, st.offset, color(st.color)).ok();
        if let Some(o) = opacity(st.color) {
            write!(out, r#" stop-opacity="{o}""#).ok();
        }
        out.push_str("/>");
    }
    out
}

/// Writes the paint server for `paint` into `defs` and returns the fill attributes.
fn paint_attrs(paint: &Paint, id: usize, d: &str, defs: &mut String) -> (String, Option<String>) {
    match paint {
        Paint::Solid { color: c } => {
            let mut a = format!(r#" fill="{}" fill-rule="evenodd""#, color(*c));
            if let Some(o) = opacity(*c) {
                write!(a, r#" fill-opacity="{o}""#).ok();
            }
            (a, None)
        }
        Paint::Linear { space, x0, y0, x1, y1, stops } => {
            writeln!(
                defs,
                r#"<linearGradient id="p{id}" gradientUnits="userSpaceOnUse" x1="{x0}" y1="{y0}" x2="{x1}" y2="{y1}" gradientTransform="{}">{}</linearGradient>"#,
                matrix(space),
                stops_xml(stops)
            )
            .ok();
            (format!(r#" fill="url(#p{id})" fill-rule="evenodd""#), None)
        }
        Paint::Radial { space, cx, cy, r, stops } => {
            writeln!(
                defs,
                r#"<radialGradient id="p{id}" gradientUnits="userSpaceOnUse" cx="{cx}" cy="{cy}" r="{r}" fx="{cx}" fy="{cy}" gradientTransform="{}">{}</radialGradient>"#,
                matrix(space),
                stops_xml(stops)
            )
            .ok();
            (format!(r#" fill="url(#p{id})" fill-rule="evenodd""#), None)
        }
        Paint::Rect { space, ox, oy, extent, stops } => {
            // No rectangular gradients in SVG. The whole shape gets the gradient along
            // X (reflected about the origin), then the "bow tie" where Y dominates gets
            // the gradient along Y. Both agree on the diagonals, so no seam shows.
            let st = stops_xml(stops);
            let m = matrix(space);
            write!(
                defs,
                r#"<linearGradient id="p{id}x" gradientUnits="userSpaceOnUse" spreadMethod="reflect" x1="{ox}" y1="{oy}" x2="{}" y2="{oy}" gradientTransform="{m}">{st}</linearGradient><linearGradient id="p{id}y" gradientUnits="userSpaceOnUse" spreadMethod="reflect" x1="{ox}" y1="{oy}" x2="{ox}" y2="{}" gradientTransform="{m}">{st}</linearGradient>"#,
                ox + extent,
                oy + extent,
            )
            .ok();
            let bow = format!(
                "M{ox} {oy}L{} {}L{} {}ZM{ox} {oy}L{} {}L{} {}Z",
                ox + 4.0,
                oy + 4.0,
                ox - 4.0,
                oy + 4.0,
                ox + 4.0,
                oy - 4.0,
                ox - 4.0,
                oy - 4.0
            );
            writeln!(
                defs,
                r#"<clipPath id="c{id}y" clipPathUnits="userSpaceOnUse"><path transform="{m}" d="{bow}"/></clipPath>"#
            )
            .ok();
            let body = format!(
                r#"<path d="{d}" fill="url(#p{id}x)" fill-rule="evenodd"/><g clip-path="url(#c{id}y)"><path d="{d}" fill="url(#p{id}y)" fill-rule="evenodd"/></g>"#
            );
            (String::from(r#" fill="none""#), Some(body))
        }
        Paint::Pattern { space, bits, fg, bg } => {
            let mut cells = format!(r#"<rect width="8" height="8" fill="{}"/>"#, color(*bg));
            for (row, b) in bits.iter().enumerate() {
                for col in 0..8 {
                    if b & (0x80 >> col) != 0 {
                        write!(
                            cells,
                            r#"<rect x="{col}" y="{}" width="1" height="1" fill="{}"/>"#,
                            7 - row,
                            color(*fg)
                        )
                        .ok();
                    }
                }
            }
            writeln!(
                defs,
                r#"<pattern id="p{id}" patternUnits="userSpaceOnUse" width="8" height="8" patternTransform="{}">{cells}</pattern>"#,
                matrix(space)
            )
            .ok();
            (format!(r#" fill="url(#p{id})" fill-rule="evenodd""#), None)
        }
    }
}

/// SVG with text placed by estimated widths (no fonts available).
pub fn to_svg(dl: &DisplayList) -> String {
    to_svg_with_layouts(dl, &[])
}

/// SVG with text from `layouts` (indexed like `dl.items`), as laid out by `flowster-text`.
pub fn to_svg_with_layouts(dl: &DisplayList, layouts: &[Option<TextLayout>]) -> String {
    let (w, h) = (dl.page.width, dl.page.height);
    let mut out = String::new();
    writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{:.0}" height="{:.0}" viewBox="0 0 {w} {h}">"#,
        w * PX_PER_IN,
        h * PX_PER_IN
    )
    .ok();
    writeln!(out, r##"<rect width="{w}" height="{h}" fill="#ffffff"/>"##).ok();
    writeln!(out, r#"<g transform="matrix(1 0 0 -1 0 {h})">"#).ok();
    let mut defs = String::new();
    let mut body = String::new();
    for (id, item) in dl.items.iter().enumerate() {
        let out = &mut body;
        match item {
            Item::Path { d, fill, stroke, blur, .. } => {
                let path = path_d(d);
                let mut attrs = String::new();
                let mut extra = None;
                match fill {
                    Some(f) => {
                        let (a, e) = paint_attrs(f, id, &path, &mut defs);
                        attrs.push_str(&a);
                        extra = e;
                    }
                    None => attrs.push_str(r#" fill="none""#),
                }
                if *blur > 0.0 {
                    writeln!(defs, r#"<filter id="b{id}" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="{:.5}"/></filter>"#, blur / 2.0).ok();
                    write!(attrs, r#" filter="url(#b{id})""#).ok();
                }
                if let Some(e) = extra {
                    out.push_str(&e);
                }
                if let Some(s) = stroke {
                    write!(
                        attrs,
                        r#" stroke="{}" stroke-width="{:.5}" stroke-linecap="{}" stroke-linejoin="round""#,
                        color(s.color),
                        s.width.max(0.001),
                        s.cap
                    )
                    .ok();
                    if let Some(o) = opacity(s.color) {
                        write!(attrs, r#" stroke-opacity="{o}""#).ok();
                    }
                    if !s.dash.is_empty() {
                        let dash: Vec<String> = s.dash.iter().map(|d| format!("{d:.4}")).collect();
                        write!(attrs, r#" stroke-dasharray="{}""#, dash.join(" ")).ok();
                    }
                }
                writeln!(out, r#"<path d="{path}"{attrs}/>"#).ok();
            }
            Item::Image { transform, x, y, w, h, mime, data, .. } => {
                // Images are stored top-down; flip back inside the shape frame.
                let t = transform.then(Affine::translate(*x, *y + *h)).then(Affine::scale(1.0, -1.0));
                match data {
                    Some(b64) => writeln!(
                        out,
                        r#"<image transform="{}" width="{w}" height="{h}" preserveAspectRatio="none" href="data:{mime};base64,{b64}"/>"#,
                        matrix(&t)
                    ),
                    None => writeln!(
                        out,
                        r##"<rect transform="{}" width="{w}" height="{h}" fill="#eeeeee" stroke="#999999" stroke-width="0.01"/>"##,
                        matrix(&t)
                    ),
                }
                .ok();
            }
            Item::Text { transform, height, .. } if matches!(layouts.get(id), Some(Some(_))) => {
                let Some(Some(l)) = layouts.get(id) else { continue };
                let frame = transform.then(Affine::translate(0.0, *height)).then(Affine::scale(1.0, -1.0));
                writeln!(out, r#"<g transform="{}">"#, matrix(&frame)).ok();
                for run in &l.runs {
                    let family = item_font(item, &run.text).unwrap_or_default();
                    writeln!(
                        out,
                        r#"<text x="{:.5}" y="{:.5}" font-family="{}" font-size="{:.5}" fill="{}" textLength="{:.5}" lengthAdjust="spacingAndGlyphs" xml:space="preserve">{}</text>"#,
                        run.x,
                        run.baseline,
                        esc(&family),
                        run.size,
                        color(run.color),
                        run.width,
                        esc(&run.text)
                    )
                    .ok();
                }
                writeln!(out, "</g>").ok();
            }
            Item::Text { transform, width, height, margins, valign, paragraphs, .. } => {
                let [ml, mr, mt, mb] = *margins;
                let inner_w = (width - ml - mr).max(0.01);
                // (paragraph, line pieces as (run index, text), line height, first line)
                type Line<'p> = (&'p Paragraph, Vec<(usize, String)>, f64, bool);
                let mut lines: Vec<Line<'_>> = Vec::new();
                let mut total = 0.0;
                for p in paragraphs {
                    for (i, line) in wrap(p, inner_w).into_iter().enumerate() {
                        let size = line.iter().map(|(ri, _)| p.runs[*ri].size).fold(0.0, f64::max);
                        let lh = if p.line_spacing < 0.0 { size * -p.line_spacing } else { p.line_spacing.max(size) };
                        total += lh;
                        lines.push((p, line, lh, i == 0));
                    }
                }
                // Text frame: +Y down from the block's top-left.
                let frame = transform.then(Affine::translate(0.0, *height)).then(Affine::scale(1.0, -1.0));
                let avail = height - mt - mb;
                let mut y = mt
                    + match valign {
                        0 => 0.0,
                        2 => avail - total,
                        _ => (avail - total) / 2.0,
                    };
                writeln!(out, r#"<g transform="{}">"#, matrix(&frame)).ok();
                for (p, mut line, lh, first) in lines {
                    // Estimated text has no indents: a bullet just leads its first line.
                    if let (true, Some(b), Some(_)) = (first, &p.bullet, p.runs.first()) {
                        line.insert(0, (0, format!("{b} ")));
                    }
                    let line_w: f64 =
                        line.iter().map(|(ri, t)| t.chars().count() as f64 * p.runs[*ri].size * AVG_ADVANCE).sum();
                    let x = match p.align {
                        0 | 3 => ml,
                        2 => width - mr - line_w,
                        _ => ml + (inner_w - line_w) / 2.0,
                    };
                    let baseline = y + lh * 0.8;
                    write!(out, r#"<text x="{x:.5}" y="{baseline:.5}" xml:space="preserve">"#).ok();
                    for (ri, t) in &line {
                        let r = &p.runs[*ri];
                        write!(
                            out,
                            r#"<tspan font-family="{}" font-size="{:.5}" fill="{}"{}{}{}>{}</tspan>"#,
                            esc(&r.font),
                            r.size,
                            color(r.color),
                            if r.bold { r#" font-weight="bold""# } else { "" },
                            if r.italic { r#" font-style="italic""# } else { "" },
                            if r.underline { r#" text-decoration="underline""# } else { "" },
                            esc(t)
                        )
                        .ok();
                    }
                    writeln!(out, "</text>").ok();
                    y += lh;
                }
                writeln!(out, "</g>").ok();
            }
        }
    }
    if !defs.is_empty() {
        writeln!(out, "<defs>\n{defs}</defs>").ok();
    }
    out.push_str(&body);
    writeln!(out, "</g>\n</svg>").ok();
    out
}

/// The font family of the run in `item` whose text contains `text` (for SVG output).
fn item_font(item: &Item, text: &str) -> Option<String> {
    let Item::Text { paragraphs, .. } = item else { return None };
    let mut first = None;
    for r in paragraphs.iter().flat_map(|p| p.runs.iter()) {
        first.get_or_insert_with(|| r.font.clone());
        if r.text.contains(text.trim()) {
            return Some(r.font.clone());
        }
    }
    first
}
