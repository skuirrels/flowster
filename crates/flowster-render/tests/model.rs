//! Display-list behaviour on small hand-built documents (SPEC §6.3, §8, §11).

use std::collections::BTreeMap;

use flowster_core::geom::Seg;
use flowster_core::model::{Cell, Document, Master, Page, Row, Section, Shape, Sheet, StyleSheet, TextPiece};
use flowster_core::theme::{ColorBase, ColorSpec, FillStyle, LineStyle, StyleSet, Theme};
use flowster_core::value::Rgba;
use flowster_render::{Item, build_page};

fn cells(pairs: &[(&str, &str)]) -> BTreeMap<String, Cell> {
    pairs.iter().map(|(k, v)| (k.to_string(), Cell::value(*v))).collect()
}

fn rect_geometry(w: f64, h: f64) -> Section {
    let row = |t: &str, ix: u32, x: f64, y: f64| Row {
        t: Some(t.into()),
        ix: Some(ix),
        cells: cells(&[("X", &x.to_string()), ("Y", &y.to_string())]),
        ..Default::default()
    };
    Section {
        n: "Geometry".into(),
        ix: Some(0),
        rows: vec![
            row("MoveTo", 1, 0.0, 0.0),
            row("LineTo", 2, w, 0.0),
            row("LineTo", 3, w, h),
            row("LineTo", 4, 0.0, h),
            row("LineTo", 5, 0.0, 0.0),
        ],
        ..Default::default()
    }
}

fn box_shape(id: u32, pin: (f64, f64)) -> Shape {
    Shape {
        id,
        sheet: Sheet {
            cells: cells(&[
                ("PinX", &pin.0.to_string()),
                ("PinY", &pin.1.to_string()),
                ("Width", "2"),
                ("Height", "1"),
            ]),
            sections: vec![rect_geometry(2.0, 1.0)],
        },
        ..Default::default()
    }
}

fn page(id: u32, shapes: Vec<Shape>) -> Page {
    Page {
        id,
        sheet: Sheet { cells: cells(&[("PageWidth", "10"), ("PageHeight", "8")]), ..Default::default() },
        shapes,
        ..Default::default()
    }
}

fn first_path(items: &[Item]) -> (&Vec<Seg>, Option<Rgba>, Option<Rgba>) {
    items
        .iter()
        .find_map(|i| match i {
            Item::Path { d, fill, stroke, .. } => {
                Some((d, fill.as_ref().map(|f| f.representative()), stroke.as_ref().map(|s| s.color)))
            }
            _ => None,
        })
        .expect("a path")
}

#[test]
fn instance_overrides_single_cells_of_master_rows() {
    let mut master_shape = box_shape(5, (1.0, 0.5));
    master_shape.sheet.cells.insert("FillForegnd".into(), Cell::value("#4472c4"));
    let master = Master { id: 2, shapes: vec![master_shape], ..Default::default() };

    // The instance moves the pin and overrides only Y of geometry row 3 (top-right corner).
    let instance = Shape {
        id: 1,
        master: Some(2),
        sheet: Sheet {
            cells: cells(&[("PinX", "5"), ("PinY", "4")]),
            sections: vec![Section {
                n: "Geometry".into(),
                ix: Some(0),
                rows: vec![Row { ix: Some(3), cells: cells(&[("Y", "1.5")]), ..Default::default() }],
                ..Default::default()
            }],
        },
        ..Default::default()
    };
    let doc =
        Document { pages: vec![page(0, vec![instance])], masters: BTreeMap::from([(2, master)]), ..Default::default() };
    let dl = build_page(&doc, 0).unwrap();
    let (d, fill, _) = first_path(&dl.items);
    // Width/Height/LocPin come from the master; X of row 3 too, its Y from the instance.
    assert_eq!(d[0], Seg::M { x: 4.0, y: 3.5 });
    assert_eq!(d[2], Seg::L { x: 6.0, y: 5.0 });
    assert_eq!(fill, Some(Rgba(0x44, 0x72, 0xc4, 255)));
}

#[test]
fn themed_cells_use_the_theme_and_fall_back_without_one() {
    let themed_style = StyleSheet {
        id: 6,
        sheet: Sheet {
            cells: cells(&[
                ("FillForegnd", "Themed"),
                ("LineColor", "Themed"),
                ("LineWeight", "Themed"),
                ("QuickStyleFillColor", "2"),
                ("QuickStyleLineColor", "2"),
                ("QuickStyleFillMatrix", "1"),
                ("QuickStyleLineMatrix", "1"),
            ]),
            ..Default::default()
        },
        fill_style: Some(0),
        line_style: Some(0),
        ..Default::default()
    };
    let no_style = StyleSheet {
        id: 0,
        sheet: Sheet {
            cells: cells(&[("FillForegnd", "1"), ("LineColor", "0"), ("LineWeight", "0.01")]),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut shape = box_shape(1, (5.0, 4.0));
    shape.line_style = Some(6);
    shape.fill_style = Some(6);
    // The same shape with "no theme" (ThemeIndex 0) set on itself.
    let mut unthemed = shape.clone();
    unthemed.id = 2;
    unthemed.sheet.cells.insert("ThemeIndex".into(), Cell::value("0"));
    let ph = |mods: Vec<(String, f64)>| ColorSpec { base: ColorBase::Scheme("phClr".into()), mods };
    let theme = Theme {
        ids: vec![34],
        scheme: BTreeMap::from([("accent1".to_string(), Rgba(0x10, 0x80, 0xf0, 255))]),
        shapes: StyleSet {
            fills: vec![FillStyle { color: Some(ph(vec![])), ..Default::default() }],
            lines: vec![LineStyle {
                width_emu: Some(25_400.0),
                color: Some(ph(vec![("lumMod".into(), 0.5)])),
                ..Default::default()
            }],
            fonts: vec![],
            effects: vec![],
        },
        ..Default::default()
    };
    // The page applies the theme (ThemeIndex 34, the theme's scheme id).
    let mut themed_page = page(0, vec![shape, unthemed]);
    themed_page.sheet.cells.insert("ThemeIndex".into(), Cell::value("34"));
    let mut doc = Document {
        pages: vec![themed_page],
        styles: BTreeMap::from([(6, themed_style), (0, no_style)]),
        theme: Some(theme.clone()),
        themes: vec![theme],
        ..Default::default()
    };
    let paths = |doc: &Document| -> Vec<(Option<Rgba>, Option<Rgba>)> {
        build_page(doc, 0)
            .unwrap()
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Path { fill, stroke, .. } => {
                    let fill = match fill {
                        Some(flowster_render::Paint::Solid { color }) => Some(*color),
                        _ => None,
                    };
                    Some((fill, stroke.as_ref().map(|s| s.color)))
                }
                _ => None,
            })
            .collect()
    };

    let dl = build_page(&doc, 0).unwrap();
    let (_, fill, stroke) = first_path(&dl.items);
    assert_eq!(fill, Some(Rgba(0x10, 0x80, 0xf0, 255)));
    assert_eq!(stroke, Some(Rgba(0x08, 0x40, 0x78, 255)));
    let Item::Path { stroke: Some(s), .. } = &dl.items[0] else { panic!() };
    assert!((s.width - 25_400.0 / 914_400.0).abs() < 1e-12);
    // A shape that sets "no theme" on itself falls through to No Style.
    assert_eq!(paths(&doc)[1], (Some(Rgba::WHITE), Some(Rgba::BLACK)));

    // A page with no ThemeIndex has no theme, though the file has theme parts
    // (Visio draws such pages unthemed).
    doc.pages[0].sheet.cells.remove("ThemeIndex");
    assert_eq!(paths(&doc), vec![(Some(Rgba::WHITE), Some(Rgba::BLACK)); 2]);

    // Without a theme, "Themed" falls through to the next style (No Style).
    doc.pages[0].sheet.cells.insert("ThemeIndex".into(), Cell::value("34"));
    doc.theme = None;
    doc.themes.clear();
    let dl = build_page(&doc, 0).unwrap();
    let (_, fill, stroke) = first_path(&dl.items);
    assert_eq!((fill, stroke), (Some(Rgba::WHITE), Some(Rgba::BLACK)));
}

#[test]
fn a_group_passes_its_theme_to_its_members() {
    let theme = Theme {
        ids: vec![37],
        scheme: BTreeMap::from([("accent1".to_string(), Rgba(0x10, 0x80, 0xf0, 255))]),
        shapes: StyleSet {
            fills: vec![FillStyle {
                color: Some(ColorSpec { base: ColorBase::Scheme("phClr".into()), mods: vec![] }),
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    let themed_style = StyleSheet {
        id: 6,
        sheet: Sheet { cells: cells(&[("FillForegnd", "Themed")]), ..Default::default() },
        fill_style: Some(0),
        ..Default::default()
    };
    let no_style = StyleSheet {
        id: 0,
        sheet: Sheet { cells: cells(&[("FillForegnd", "1")]), ..Default::default() },
        ..Default::default()
    };
    let mut member = box_shape(2, (1.0, 0.5));
    member.fill_style = Some(6);
    let mut group = box_shape(1, (5.0, 4.0));
    group.kind = flowster_core::model::ShapeType::Group;
    group.sheet.sections.clear();
    group.sheet.cells.insert("ThemeIndex".into(), Cell::value("37"));
    group.shapes = vec![member];
    // The page has no theme: only the group's member is themed.
    let doc = Document {
        pages: vec![page(0, vec![group])],
        styles: BTreeMap::from([(6, themed_style), (0, no_style)]),
        theme: Some(theme.clone()),
        themes: vec![theme],
        ..Default::default()
    };
    let (_, fill, _) = first_path(&build_page(&doc, 0).unwrap().items);
    assert_eq!(fill, Some(Rgba(0x10, 0x80, 0xf0, 255)));
}

#[test]
fn hidden_layers_and_background_pages() {
    let mut fg = page(
        0,
        vec![box_shape(1, (2.0, 2.0)), {
            let mut s = box_shape(2, (6.0, 2.0));
            s.sheet.cells.insert("LayerMember".into(), Cell::value("1"));
            s
        }],
    );
    fg.back_page = Some(9);
    fg.sheet.sections.push(Section {
        n: "Layer".into(),
        rows: vec![
            Row { ix: Some(0), cells: cells(&[("Visible", "1")]), ..Default::default() },
            Row { ix: Some(1), cells: cells(&[("Visible", "0")]), ..Default::default() },
        ],
        ..Default::default()
    });
    let mut bg = page(9, vec![box_shape(7, (5.0, 7.0))]);
    bg.background = true;
    let doc = Document { pages: vec![bg, fg], ..Default::default() };

    let dl = build_page(&doc, 1).unwrap();
    let shapes: Vec<u32> = dl
        .items
        .iter()
        .map(|i| match i {
            Item::Path { shape, .. } | Item::Text { shape, .. } | Item::Image { shape, .. } => *shape,
        })
        .collect();
    // Background first; the shape on the hidden layer is skipped.
    assert_eq!(shapes, vec![7, 1]);
}

#[test]
fn text_runs_and_paragraphs() {
    let mut s = box_shape(1, (5.0, 4.0));
    s.sheet.sections.push(Section {
        n: "Character".into(),
        rows: vec![
            Row { ix: Some(0), cells: cells(&[("Style", "1"), ("Color", "2")]), ..Default::default() },
            Row { ix: Some(1), cells: cells(&[("Size", "0.25")]), ..Default::default() },
        ],
        ..Default::default()
    });
    s.text = Some(vec![
        TextPiece::Cp(0),
        TextPiece::Text("Bold".into()),
        TextPiece::Cp(1),
        TextPiece::Text(" big\nSecond line\n".into()),
    ]);
    let doc = Document { pages: vec![page(0, vec![s])], ..Default::default() };
    let dl = build_page(&doc, 0).unwrap();
    let Some(Item::Text { paragraphs, .. }) = dl.items.iter().find(|i| matches!(i, Item::Text { .. })) else {
        panic!("no text")
    };
    assert_eq!(paragraphs.len(), 2);
    let r = &paragraphs[0].runs;
    assert_eq!((r[0].text.as_str(), r[0].bold, r[0].color), ("Bold", true, Rgba(255, 0, 0, 255)));
    assert_eq!((r[1].text.as_str(), r[1].size), (" big", 0.25));
    assert_eq!(paragraphs[1].runs[0].text, "Second line");
}

#[test]
fn formulas_without_cached_values_are_computed() {
    // Written by a generator that stores formulas only: no V anywhere in the geometry.
    let fcell = |f: &str| Cell { v: None, f: Some(f.into()), u: None };
    let row = |t: &str, ix: u32, x: &str, y: &str| Row {
        t: Some(t.into()),
        ix: Some(ix),
        cells: [("X".to_string(), fcell(x)), ("Y".to_string(), fcell(y))].into(),
        ..Default::default()
    };
    let shape = Shape {
        id: 1,
        sheet: Sheet {
            cells: [
                ("PinX".to_string(), fcell("1 in+1 in")),
                ("PinY".to_string(), Cell::value("2")),
                ("Width".to_string(), fcell("2*1.5")),
                ("Height".to_string(), Cell::value("1")),
                ("LocPinX".to_string(), fcell("Width*0.5")),
                ("LocPinY".to_string(), fcell("Height*0.5")),
            ]
            .into(),
            sections: vec![Section {
                n: "Geometry".into(),
                ix: Some(0),
                rows: vec![
                    row("MoveTo", 1, "Width*0", "Height*0"),
                    row("LineTo", 2, "Width*1", "Height*0"),
                    row("LineTo", 3, "Width*1", "Height*1"),
                    row("LineTo", 4, "Geometry1.X1", "Geometry1.Y1"),
                ],
                ..Default::default()
            }],
        },
        ..Default::default()
    };
    let doc = Document { pages: vec![page(0, vec![shape])], ..Default::default() };
    let dl = build_page(&doc, 0).unwrap();
    let (d, fill, _) = first_path(&dl.items);
    // Width 3 centred on PinX 2: x from 0.5 to 3.5; the last row refers back to row 1.
    assert_eq!(d[0], Seg::M { x: 0.5, y: 1.5 });
    assert_eq!(d[1], Seg::L { x: 3.5, y: 1.5 });
    assert_eq!(d[2], Seg::L { x: 3.5, y: 2.5 });
    assert_eq!(d[3], Seg::L { x: 0.5, y: 1.5 });
    assert!(fill.is_some(), "closed triangle is filled");
}

#[test]
fn gradients_patterns_and_shadows() {
    use flowster_render::Paint;
    let mut g = box_shape(1, (2.0, 2.0));
    for (k, v) in [("FillGradientEnabled", "1"), ("FillGradientDir", "3"), ("FillForegnd", "#ff0000")] {
        g.sheet.cells.insert(k.into(), Cell::value(v));
    }
    let stop = |ix: u32, c: &str, p: &str| Row {
        ix: Some(ix),
        cells: cells(&[("GradientStopColor", c), ("GradientStopPosition", p), ("GradientStopColorTrans", "0")]),
        ..Default::default()
    };
    g.sheet.sections.push(Section {
        n: "FillGradient".into(),
        rows: vec![stop(0, "#ffffff", "0"), stop(1, "#0000ff", "1")],
        ..Default::default()
    });

    let mut hatch = box_shape(2, (5.0, 2.0));
    for (k, v) in [("FillPattern", "2"), ("FillForegnd", "#000000"), ("FillBkgnd", "#ffffff")] {
        hatch.sheet.cells.insert(k.into(), Cell::value(v));
    }
    let mut shadowed = box_shape(3, (8.0, 2.0));
    for (k, v) in [
        ("ShdwPattern", "1"),
        ("ShapeShdwType", "1"),
        ("ShapeShdwOffsetX", "0.1"),
        ("ShapeShdwOffsetY", "-0.1"),
        ("ShdwForegnd", "#000000"),
        ("ShapeShdwBlur", "0.05"),
    ] {
        shadowed.sheet.cells.insert(k.into(), Cell::value(v));
    }
    let doc = Document { pages: vec![page(0, vec![g, hatch, shadowed])], ..Default::default() };
    let dl = build_page(&doc, 0).unwrap();
    let fills: Vec<(u32, &Paint, f64)> = dl
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Path { shape, fill: Some(f), blur, .. } => Some((*shape, f, *blur)),
            _ => None,
        })
        .collect();
    // Gradient stops win over FillForegnd; radial from centre in the unit box.
    assert!(matches!(fills[0], (1, Paint::Radial { cx, cy, .. }, _) if *cx == 0.5 && *cy == 0.5));
    assert!(matches!(fills[1], (2, Paint::Pattern { .. }, _)));
    // The shadow is drawn first, offset on the page and blurred, then the shape.
    let (id, Paint::Solid { color }, blur) = fills[2] else { panic!("{:?}", fills[2]) };
    assert_eq!((id, color.3, blur), (3, 255, 0.05));
    let Item::Path { d, .. } =
        dl.items.iter().find(|i| matches!(i, Item::Path { shape: 3, blur, .. } if *blur > 0.0)).unwrap()
    else {
        panic!()
    };
    assert_eq!(d[0], Seg::M { x: 7.1, y: 1.4 });
}

#[test]
fn svg_export_anchors_lines_by_their_alignment() {
    // A centred pill label, a right-aligned cell and a left-aligned one. The SVG
    // export only estimates text widths, so it must leave the placement of
    // centred and right-aligned lines to the viewer's font.
    let labelled = |id: u32, pin: (f64, f64), align: &str, text: &str| {
        let mut s = box_shape(id, pin);
        for (k, v) in [("LeftMargin", "0.1"), ("RightMargin", "0.1")] {
            s.sheet.cells.insert(k.into(), Cell::value(v));
        }
        s.sheet.sections.push(Section {
            n: "Paragraph".into(),
            rows: vec![Row { ix: Some(0), cells: cells(&[("HorzAlign", align)]), ..Default::default() }],
            ..Default::default()
        });
        s.text = Some(vec![TextPiece::Text(text.into())]);
        s
    };
    let shapes = vec![
        labelled(1, (2.0, 6.0), "1", "Colibra (tbc)"),
        labelled(2, (5.0, 6.0), "2", "CargoWrite"),
        labelled(3, (8.0, 6.0), "0", "Value Stream"),
    ];
    let doc = Document { pages: vec![page(0, shapes)], ..Default::default() };
    let svg = flowster_render::svg::to_svg(&build_page(&doc, 0).unwrap());
    let text_line = |needle: &str| {
        let line = svg.lines().find(|l| l.contains(needle)).unwrap_or_else(|| panic!("no {needle} in\n{svg}"));
        let x = line.split("<text x=\"").nth(1).and_then(|s| s.split('"').next()).unwrap().to_owned();
        (x, line.to_owned())
    };
    // Block frame: the 2 in wide box with 0.1 in margins.
    let (x, line) = text_line("Colibra");
    assert_eq!((x.as_str(), line.contains(r#"text-anchor="middle""#)), ("1.00000", true), "{line}");
    let (x, line) = text_line("CargoWrite");
    assert_eq!((x.as_str(), line.contains(r#"text-anchor="end""#)), ("1.90000", true), "{line}");
    let (x, line) = text_line("Value");
    assert_eq!((x.as_str(), line.contains("text-anchor")), ("0.10000", false), "{line}");
}
