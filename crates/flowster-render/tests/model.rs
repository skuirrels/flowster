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
            Item::Path { d, fill, stroke, .. } => Some((d, *fill, stroke.as_ref().map(|s| s.color))),
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
    let ph = |mods: Vec<(String, f64)>| ColorSpec { base: ColorBase::Scheme("phClr".into()), mods };
    let theme = Theme {
        scheme: BTreeMap::from([("accent1".to_string(), Rgba(0x10, 0x80, 0xf0, 255))]),
        shapes: StyleSet {
            fills: vec![FillStyle { color: Some(ph(vec![])), pattern: None }],
            lines: vec![LineStyle {
                width_emu: Some(25_400.0),
                color: Some(ph(vec![("lumMod".into(), 0.5)])),
                ..Default::default()
            }],
            fonts: vec![],
        },
        ..Default::default()
    };
    let mut doc = Document {
        pages: vec![page(0, vec![shape])],
        styles: BTreeMap::from([(6, themed_style), (0, no_style)]),
        theme: Some(theme),
        ..Default::default()
    };

    let dl = build_page(&doc, 0).unwrap();
    let (_, fill, stroke) = first_path(&dl.items);
    assert_eq!(fill, Some(Rgba(0x10, 0x80, 0xf0, 255)));
    assert_eq!(stroke, Some(Rgba(0x08, 0x40, 0x78, 255)));
    let Item::Path { stroke: Some(s), .. } = &dl.items[0] else { panic!() };
    assert!((s.width - 25_400.0 / 914_400.0).abs() < 1e-12);

    // Without a theme, "Themed" falls through to the next style (No Style).
    doc.theme = None;
    let dl = build_page(&doc, 0).unwrap();
    let (_, fill, stroke) = first_path(&dl.items);
    assert_eq!((fill, stroke), (Some(Rgba::WHITE), Some(Rgba::BLACK)));
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
