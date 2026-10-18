//! Theme part (DrawingML `a:theme` + Visio `vt:` extensions) → [`Theme`].

use flowster_core::theme::{ColorBase, ColorSpec, FillStyle, FontStyle, LineStyle, StyleSet, Theme};
use flowster_core::value::Rgba;

use crate::dom::El;

const SCHEME_SLOTS: &[&str] = &[
    "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink",
];

fn hex(v: &str) -> Option<Rgba> {
    let n = u32::from_str_radix(v.trim(), 16).ok()?;
    (v.trim().len() == 6).then_some(Rgba((n >> 16) as u8, (n >> 8) as u8, n as u8, 255))
}

/// Depth-first search for the first descendant element named `name`.
fn find<'a>(el: &'a El, name: &str) -> Option<&'a El> {
    for c in el.children() {
        if c.name == name {
            return Some(c);
        }
        if let Some(f) = find(c, name) {
            return Some(f);
        }
    }
    None
}

fn pct(v: &str) -> Option<f64> {
    v.trim().parse::<f64>().ok().map(|n| n / 100_000.0)
}

/// A colour element: `srgbClr`, `sysClr`, `schemeClr`, `scrgbClr`, `prstClr` with modifiers.
fn color_el(el: &El) -> Option<ColorSpec> {
    let base = match el.name.as_str() {
        "srgbClr" => ColorBase::Rgb(hex(el.attr("val")?)?),
        "sysClr" => ColorBase::Rgb(hex(el.attr("lastClr").unwrap_or("000000"))?),
        "schemeClr" => ColorBase::Scheme(el.attr("val")?.to_owned()),
        "scrgbClr" => {
            let c = |k: &str| (pct(el.attr(k).unwrap_or("0")).unwrap_or(0.0).clamp(0.0, 1.0) * 255.0).round() as u8;
            ColorBase::Rgb(Rgba(c("r"), c("g"), c("b"), 255))
        }
        "prstClr" => ColorBase::Rgb(match el.attr("val")? {
            "white" => Rgba::WHITE,
            _ => Rgba::BLACK,
        }),
        _ => return None,
    };
    let mods = el
        .children()
        .map(|m| {
            let v = m.attr("val").and_then(pct).unwrap_or(0.0);
            // hueOff is in 60000ths of a degree, not percent.
            let v = if m.name == "hueOff" { v * 100_000.0 / 60_000.0 } else { v };
            (m.name.clone(), v)
        })
        .collect();
    Some(ColorSpec { base, mods })
}

/// The first colour inside `el` (any depth).
fn first_color(el: &El) -> Option<ColorSpec> {
    for c in el.children() {
        if let Some(spec) = color_el(c) {
            return Some(spec);
        }
        if let Some(spec) = first_color(c) {
            return Some(spec);
        }
    }
    None
}

fn fill_style(el: &El) -> FillStyle {
    let color = match el.name.as_str() {
        "solidFill" => first_color(el),
        // Gradients are drawn solid with their middle stop until gradient fills land.
        "gradFill" => el.child("gsLst").and_then(|l| {
            let stops: Vec<&El> = l.children_named("gs").collect();
            stops.get(stops.len() / 2).and_then(|s| first_color(s))
        }),
        _ => None,
    };
    FillStyle { color, pattern: if el.name == "noFill" { Some(0.0) } else { None } }
}

fn line_style(el: &El) -> LineStyle {
    LineStyle {
        width_emu: el.attr("w").and_then(|w| w.parse().ok()),
        color: el.child("solidFill").and_then(first_color),
        pattern: el.child("noFill").map(|_| 0.0),
        ..Default::default()
    }
}

fn font_style(el: &El) -> FontStyle {
    FontStyle { color: el.child("color").and_then(first_color), style: el.attr("style").and_then(|s| s.parse().ok()) }
}

/// Merges Visio's `vt:lineEx` attributes into the DrawingML line styles.
fn apply_line_ex(lines: &mut [LineStyle], list: Option<&El>) {
    let Some(list) = list else { return };
    for (i, ls) in list.children_named("lineStyle").enumerate() {
        let (Some(slot), Some(ex)) = (lines.get_mut(i), ls.child("lineEx")) else { continue };
        let num = |k: &str| ex.attr(k).and_then(|v| v.parse::<f64>().ok());
        slot.pattern = slot.pattern.or(num("pattern"));
        slot.rounding_emu = num("rndg");
        slot.begin_arrow = num("start");
        slot.begin_arrow_size = num("startSize");
        slot.end_arrow = num("end");
        slot.end_arrow_size = num("endSize");
    }
}

fn style_set(fmt: &El) -> StyleSet {
    StyleSet {
        fills: fmt.child("fillStyleLst").map(|l| l.children().map(fill_style).collect()).unwrap_or_default(),
        lines: fmt.child("lnStyleLst").map(|l| l.children_named("ln").map(line_style).collect()).unwrap_or_default(),
        fonts: Vec::new(),
    }
}

pub fn parse_theme(root: &El) -> Theme {
    let mut theme = Theme { name: root.attr("name").unwrap_or("").to_owned(), ..Default::default() };
    let Some(elements) = root.child("themeElements") else { return theme };

    if let Some(cs) = elements.child("clrScheme") {
        for slot in SCHEME_SLOTS {
            if let Some(ColorSpec { base: ColorBase::Rgb(rgb), .. }) = cs.child(slot).and_then(first_color) {
                theme.scheme.insert((*slot).to_owned(), rgb);
            }
        }
        if let Some(ColorSpec { base: ColorBase::Rgb(rgb), .. }) = find(cs, "bkgnd").and_then(first_color) {
            theme.scheme.insert("bkgnd".into(), rgb);
        }
    }
    if let Some(fs) = elements.child("fontScheme") {
        let face = |n: &str| {
            fs.child(n)
                .and_then(|f| f.child("latin"))
                .and_then(|l| l.attr("typeface"))
                .filter(|t| !t.is_empty())
                .map(str::to_owned)
        };
        theme.major_font = face("majorFont");
        theme.minor_font = face("minorFont");
    }
    if let Some(fmt) = elements.child("fmtScheme") {
        theme.shapes = style_set(fmt);
    }

    // Visio extensions live in a:extLst under themeElements (and the colour scheme).
    if let Some(conn) = find(elements, "fmtConnectorScheme") {
        theme.connectors = style_set(conn);
    }
    if let Some(fill_props) = find(elements, "fillStyles") {
        for (i, fp) in fill_props.children_named("fillProps").enumerate() {
            if let Some(slot) = theme.shapes.fills.get_mut(i) {
                slot.pattern = slot.pattern.or(fp.attr("pattern").and_then(|p| p.parse().ok()));
            }
        }
    }
    if let Some(ls) = find(elements, "lineStyles") {
        apply_line_ex(&mut theme.shapes.lines, ls.child("fmtSchemeLineStyles"));
        apply_line_ex(&mut theme.connectors.lines, ls.child("fmtConnectorSchemeLineStyles"));
    }
    if let Some(fg) = find(elements, "fontStylesGroup") {
        theme.shapes.fonts =
            fg.child("fontStyles").map(|l| l.children_named("fontProps").map(font_style).collect()).unwrap_or_default();
        theme.connectors.fonts = fg
            .child("connectorFontStyles")
            .map(|l| l.children_named("fontProps").map(font_style).collect())
            .unwrap_or_default();
    }
    if let Some(vc) = find(elements, "variationClrSchemeLst") {
        theme.variation_colors = vc
            .children_named("variationClrScheme")
            .map(|s| (1..=7).filter_map(|k| s.child(&format!("varColor{k}")).and_then(first_color)).collect())
            .collect();
    }
    if let Some(vs) = find(elements, "variationStyleSchemeLst") {
        theme.variation_styles = vs
            .children_named("variationStyleScheme")
            .map(|s| {
                s.children_named("varStyle")
                    .map(|v| {
                        let idx = |k: &str| v.attr(k).and_then(|x| x.parse().ok()).unwrap_or(1);
                        [idx("fillIdx"), idx("lineIdx"), idx("effectIdx"), idx("fontIdx")]
                    })
                    .collect()
            })
            .collect();
    }
    theme
}
