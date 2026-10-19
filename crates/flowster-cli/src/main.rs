//! `flowster`: the headless CLI (SPEC §17.3). M0 commands:
//!
//! * `flowster info <file> [--json]`: parts and relationships of a package;
//! * `flowster export <file> <out.svg|.png|.pdf|.json> [--page N] [--dpi D]`: render a page
//!   (PDF: every foreground page unless --page is given) or dump its display list;
//! * `flowster conform <dir> [--diff out] [--json]`: SSIM of every corpus page against the oracle's PNG;
//! * `flowster save-check <files> [--out dir]`: write back to .vsdx, read again, compare;
//! * `flowster ssim <a.png> <b.png>`: content-aligned SSIM of two images;
//! * `flowster crdt-check <files>`: import into the CRDT, reload from a snapshot, compare the model;
//! * `flowster recalc-check <file> [--json]`: recompute every formula and compare with Visio's cached values;
//! * `flowster determinism [--json]`: derived-state hash (compared against WASM in CI);
//! * `flowster bench crdt [--shapes N] [--library loro|yrs|all] [--json]`: the CRDT spike;
//! * `flowster fixture <out.vsdx>`: writes the minimal test package.

mod alloc_stats;

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "flowster", version, about = "Flowster command-line tools")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List the parts, content types and relationships of a .vsdx/.vssx/.vstx package.
    Info {
        file: PathBuf,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Render to SVG, PNG or PDF, or dump the display list as JSON (by output extension).
    Export {
        file: PathBuf,
        out: PathBuf,
        /// 1-based page number; defaults to the first foreground page (PDF: all of them).
        #[arg(long)]
        page: Option<usize>,
        /// Raster resolution for PNG.
        #[arg(long, default_value_t = 192.0)]
        dpi: f64,
    },
    /// Recompute every formula and compare the results with the values cached in the file.
    RecalcCheck {
        file: PathBuf,
        #[arg(long)]
        json: bool,
        /// Mismatches to list (all with --json).
        #[arg(long, default_value_t = 20)]
        show: usize,
    },
    /// Compare our renderings with the oracle's (`<name>-p<N>.png` next to each `<name>.vsdx` in a folder).
    Conform {
        dir: PathBuf,
        /// SSIM a page needs to pass.
        #[arg(long, default_value_t = 0.95)]
        threshold: f64,
        /// Write our aligned rendering and the SSIM map of every page here.
        #[arg(long)]
        diff: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Import drawings into the CRDT, reload them from a snapshot and check the model reads back unchanged.
    CrdtCheck { files: Vec<PathBuf> },
    /// Write drawings back to .vsdx, read them again and check the model is unchanged.
    SaveCheck {
        files: Vec<PathBuf>,
        /// Keep the written files here (for opening them in Visio).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// SSIM of two images (content-aligned), e.g. two Visio exports.
    Ssim { reference: PathBuf, other: PathBuf },
    /// Print the derived-state determinism hash.
    Determinism {
        #[arg(long)]
        json: bool,
    },
    /// Benchmarks.
    Bench {
        #[command(subcommand)]
        what: Bench,
    },
    /// Write the minimal test package to a file.
    Fixture { out: PathBuf },
}

#[derive(Subcommand)]
enum Bench {
    /// CRDT spike (docs/adr/0001-crdt-library.md).
    Crdt {
        #[arg(long, default_value_t = 10_000)]
        shapes: usize,
        #[arg(long, value_enum, default_value_t = Library::All)]
        library: Library,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Library {
    Loro,
    Yrs,
    All,
}

struct NativeProbe(Instant);

impl flowster_crdt::spike::Probe for NativeProbe {
    fn now_ms(&self) -> f64 {
        self.0.elapsed().as_secs_f64() * 1000.0
    }
    fn live_bytes(&self) -> usize {
        alloc_stats::live()
    }
    fn reset_peak(&self) {
        alloc_stats::reset_peak()
    }
    fn peak_bytes(&self) -> usize {
        alloc_stats::peak()
    }
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Info { file, json } => info(&file, json),
        Cmd::Export { file, out, page, dpi } => export(&file, &out, page, dpi),
        Cmd::RecalcCheck { file, json, show } => recalc_check(&file, json, show),
        Cmd::Conform { dir, threshold, diff, json } => conform(&dir, threshold, diff.as_ref(), json),
        Cmd::CrdtCheck { files } => crdt_check(&files),
        Cmd::SaveCheck { files, out } => save_check(&files, out.as_ref()),
        Cmd::Ssim { reference, other } => {
            use flowster_export::compare::{Gray, compare};
            let load = |p: &PathBuf| -> Result<Gray> {
                Gray::decode_png(&std::fs::read(p)?).with_context(|| format!("decoding {}", p.display()))
            };
            let c = compare(&load(&reference)?, &load(&other)?).context("an image is blank")?;
            println!("ssim {:.4}", c.ssim);
            Ok(())
        }
        Cmd::Determinism { json } => {
            let r = flowster_core::determinism::default_report();
            if json {
                println!("{}", serde_json::to_string(&r)?);
            } else {
                println!("{} ({} values, {} shapes)", r.hash, r.values, r.shapes);
            }
            Ok(())
        }
        Cmd::Bench { what: Bench::Crdt { shapes, library, json } } => bench_crdt(shapes, library, json),
        Cmd::Fixture { out } => {
            std::fs::write(&out, flowster_vsdx::fixture::minimal_vsdx())
                .with_context(|| format!("writing {}", out.display()))?;
            println!("wrote {}", out.display());
            Ok(())
        }
    }
}

fn info(file: &PathBuf, json: bool) -> Result<()> {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if matches!(ext.as_str(), "vsd" | "vss" | "vst" | "vsw") {
        bail!("legacy binary .{ext} files are imported by flowster-vsd, which is not part of M0");
    }
    let f = std::fs::File::open(file).with_context(|| format!("opening {}", file.display()))?;
    let pkg = flowster_vsdx::read_package_with(std::io::BufReader::new(f), &flowster_vsdx::Limits::default())
        .with_context(|| format!("reading {}", file.display()))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&pkg)?);
        return Ok(());
    }
    println!("{}  ({:?}, {} parts, {} bytes uncompressed)", file.display(), pkg.kind, pkg.parts.len(), pkg.total_size);
    if let Some(m) = &pkg.main_part {
        println!("main part: {m}");
    }
    println!("\nPARTS");
    for p in &pkg.parts {
        println!("  {:>9}  {:<48}  {}", p.size, p.name, p.content_type.as_deref().unwrap_or("?"));
    }
    println!("\nRELATIONSHIPS");
    for r in &pkg.relationships {
        let ty = r.rel_type.rsplit('/').next().unwrap_or(&r.rel_type);
        let target =
            if r.external { format!("{} (external)", r.target) } else { r.target_part.clone().unwrap_or_default() };
        println!("  {:<32} {:<6} {:<22} → {}", r.source, r.id, ty, target);
    }
    for w in &pkg.warnings {
        eprintln!("warning: {w}");
    }
    Ok(())
}

fn export(file: &PathBuf, out: &PathBuf, page: Option<usize>, dpi: f64) -> Result<()> {
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let pkg = flowster_vsdx::OpcPackage::open(bytes).with_context(|| format!("opening {}", file.display()))?;
    let doc = flowster_vsdx::visio::read_document(&pkg)?;
    let index = match page {
        Some(n) if n >= 1 && n <= doc.pages.len() => n - 1,
        Some(n) => bail!("page {n} does not exist ({} pages)", doc.pages.len()),
        None => doc.pages.iter().position(|p| !p.background).unwrap_or(0),
    };
    let ext = out.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "vsdx" {
        let title = out.file_stem().and_then(|s| s.to_str()).unwrap_or("Drawing");
        let bytes = flowster_vsdx::writer::write_vsdx(&doc, flowster_vsdx::visio::theme_xml(&pkg).as_deref(), title)
            .map_err(anyhow::Error::msg)?;
        std::fs::write(out, bytes).with_context(|| format!("writing {}", out.display()))?;
        println!("{} → {} ({} pages)", file.display(), out.display(), doc.pages.len());
        return Ok(());
    }
    let mut text = flowster_text::TextEngine::system();
    if ext == "pdf" {
        let indices: Vec<usize> = match page {
            Some(_) => vec![index],
            None => (0..doc.pages.len()).filter(|&i| !doc.pages[i].background).collect(),
        };
        let lists: Vec<_> = indices.iter().filter_map(|&i| flowster_render::build_page(&doc, i)).collect();
        let layouts: Vec<_> = lists.iter().map(|dl| text.layout_display_list(dl)).collect();
        let pages: Vec<_> = lists.iter().zip(&layouts).map(|(dl, l)| (dl, l.as_slice())).collect();
        let pdf = flowster_export::pdf::pdf(&pages).map_err(anyhow::Error::msg)?;
        std::fs::write(out, pdf).with_context(|| format!("writing {}", out.display()))?;
        println!("{} → {} ({} pages)", file.display(), out.display(), pages.len());
        return Ok(());
    }
    let dl = flowster_render::build_page(&doc, index).context("document has no pages")?;
    let layouts = text.layout_display_list(&dl);
    let body: Vec<u8> = match ext.as_str() {
        "json" => serde_json::to_vec_pretty(&dl)?,
        "png" => flowster_export::png::png(&dl, &layouts, dpi).context("rendering PNG")?,
        _ => flowster_render::svg::to_svg_with_layouts(&dl, &layouts).into_bytes(),
    };
    std::fs::write(out, body).with_context(|| format!("writing {}", out.display()))?;
    for w in &dl.warnings {
        eprintln!("warning: {w}");
    }
    println!(
        "{} → {} (page {} \"{}\", {} items)",
        file.display(),
        out.display(),
        index + 1,
        dl.page.name,
        dl.items.len()
    );
    Ok(())
}

fn recalc_check(file: &PathBuf, json: bool, show: usize) -> Result<()> {
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let pkg = flowster_vsdx::OpcPackage::open(bytes).with_context(|| format!("opening {}", file.display()))?;
    let doc = flowster_vsdx::visio::read_document(&pkg)?;
    let r = flowster_core::calc::recalc_check(&doc);
    if json {
        println!("{}", serde_json::to_string_pretty(&r)?);
        return Ok(());
    }
    let pct = |n: usize| if r.formulas == 0 { 100.0 } else { n as f64 * 100.0 / r.formulas as f64 };
    println!("{}: {} formulas", file.display(), r.formulas);
    println!("  matched cached value  {:>6}  ({:.1}%)", r.matched, pct(r.matched));
    println!("  kept cached value     {:>6}  ({:.1}%)", r.kept_cached, pct(r.kept_cached));
    println!("  mismatched            {:>6}  ({:.1}%)", r.mismatched.len(), pct(r.mismatched.len()));
    if !r.unsupported_functions.is_empty() {
        let list: Vec<String> = r.unsupported_functions.iter().map(|(k, v)| format!("{k}×{v}")).collect();
        println!("  not yet evaluated: {}", list.join(", "));
    }
    for m in r.mismatched.iter().take(show) {
        println!("  {} shape {} {}: {}  →  {} (cached {})", m.page, m.shape, m.cell, m.formula, m.computed, m.cached);
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct PageScore {
    file: String,
    page: usize,
    ssim: f64,
    dpi: f64,
    ref_box: (usize, usize),
    our_box: (usize, usize),
}

fn conform(dir: &PathBuf, threshold: f64, diff: Option<&PathBuf>, json: bool) -> Result<()> {
    use flowster_export::compare::{Gray, compare};
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("vsdx")))
        .collect();
    files.sort();
    if let Some(d) = diff {
        std::fs::create_dir_all(d)?;
    }
    let mut text = flowster_text::TextEngine::system();
    let mut scores = Vec::new();
    for file in &files {
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_owned();
        let bytes = std::fs::read(file)?;
        let pkg = flowster_vsdx::OpcPackage::open(bytes).with_context(|| format!("opening {}", file.display()))?;
        let doc = flowster_vsdx::visio::read_document(&pkg)?;
        // Visio's Pages collection (and so the oracle's numbering) lists foreground pages first.
        let order = (0..doc.pages.len()).filter(|&i| !doc.pages[i].background);
        let order: Vec<usize> = order.chain((0..doc.pages.len()).filter(|&i| doc.pages[i].background)).collect();
        for (n, &index) in order.iter().enumerate() {
            let ref_path = dir.join(format!("{stem}-p{}.png", n + 1));
            let Ok(ref_bytes) = std::fs::read(&ref_path) else { continue };
            let reference = Gray::decode_png(&ref_bytes).with_context(|| format!("decoding {}", ref_path.display()))?;
            let Some(dl) = flowster_render::build_page(&doc, index) else { continue };
            let layouts = text.layout_display_list(&dl);
            // Visio exports the drawing's extents, which may reach past the page.
            let Some([x0, y0, x1, y1]) = dl.extents() else { continue };
            let area = [x0 - 0.25, y0 - 0.25, x1 + 0.25, y1 + 0.25];
            let render = |dpi: f64, area: [f64; 4]| {
                flowster_export::png::render_area(&dl, &layouts, dpi, area).map(|p| Gray::from_pixmap(&p))
            };
            // The oracle's export resolution is not recorded: match content-box sizes.
            let (Some(probe), Some((rx0, ry0, rx1, ry1))) = (render(96.0, area), reference.content_box()) else {
                continue;
            };
            let Some((ox0, oy0, ox1, oy1)) = probe.content_box() else { continue };
            let estimate = 96.0 * ((rx1 - rx0) + (ry1 - ry0)) as f64 / ((ox1 - ox0) + (oy1 - oy0)) as f64;
            // Content boxes are skewed by anything that differs at the edges,
            // and Visio's scale is not exactly the display's: refine around the
            // estimate by the best geometric fit.
            let candidates: Vec<_> = std::thread::scope(|scope| {
                let jobs: Vec<_> = (-6..=6)
                    .map(|k| {
                        let dpi = estimate + f64::from(k) * 0.2;
                        let (render, reference) = (&render, &reference);
                        scope.spawn(move || render(dpi, area).and_then(|g| compare(reference, &g)).map(|c| (dpi, c)))
                    })
                    .collect();
                jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect()
            });
            let Some((dpi, c)) = candidates.into_iter().min_by(|a, b| a.1.mad.total_cmp(&b.1.mad)) else { continue };
            // Render again with our image origin where the oracle's is, so that
            // pixel-anchored hatch tiles line up as well.
            let (dx, dy) = (c.offset.0 as f64 / dpi, c.offset.1 as f64 / dpi);
            let ours = render(dpi, [area[0] + dx, area[1], area[2], area[3] - dy]).context("rendering")?;
            let Some(c) = compare(&reference, &ours) else { continue };
            if let Some(d) = diff {
                if let Some(png) = c.aligned.encode_png() {
                    std::fs::write(d.join(format!("{stem}-p{}-ours.png", n + 1)), png)?;
                }
                let map = Gray { width: reference.width, height: reference.height, px: c.map.clone() };
                if let Some(png) = map.encode_png() {
                    std::fs::write(d.join(format!("{stem}-p{}-ssim.png", n + 1)), png)?;
                }
            }
            scores.push(PageScore {
                file: stem.clone(),
                page: n + 1,
                ssim: c.ssim,
                dpi,
                ref_box: c.ref_box,
                our_box: c.our_box,
            });
        }
    }
    let passed = scores.iter().filter(|s| s.ssim >= threshold).count();
    if json {
        println!("{}", serde_json::to_string_pretty(&scores)?);
        return Ok(());
    }
    for s in &scores {
        let mark = if s.ssim >= threshold { "ok  " } else { "FAIL" };
        println!(
            "{mark} {:<12} p{:<2} ssim {:.4}  dpi {:>6.1}  box {}×{} vs ours {}×{}",
            s.file, s.page, s.ssim, s.dpi, s.ref_box.0, s.ref_box.1, s.our_box.0, s.our_box.1
        );
    }
    let pct = if scores.is_empty() { 0.0 } else { passed as f64 * 100.0 / scores.len() as f64 };
    println!("{passed}/{} pages at SSIM ≥ {threshold} ({pct:.0}%)", scores.len());
    Ok(())
}

/// The first place two models differ, for `crdt-check` reports.
fn first_difference(a: &flowster_core::model::Document, b: &flowster_core::model::Document) -> Option<String> {
    use flowster_core::model::Shape;
    fn shapes(a: &[Shape], b: &[Shape], at: &str) -> Option<String> {
        if a.len() != b.len() {
            return Some(format!("{at}: {} shapes vs {}", a.len(), b.len()));
        }
        for (x, y) in a.iter().zip(b) {
            if x != y {
                let inner = shapes(&x.shapes, &y.shapes, &format!("{at} shape {}", x.id));
                return inner.or_else(|| {
                    let field = if x.sheet != y.sheet {
                        "sheet"
                    } else if x.text != y.text {
                        "text"
                    } else {
                        "props"
                    };
                    Some(format!("{at} shape {} {field}", x.id))
                });
            }
        }
        None
    }
    if a.pages.len() != b.pages.len() {
        return Some(format!("{} pages vs {}", a.pages.len(), b.pages.len()));
    }
    for (x, y) in a.pages.iter().zip(&b.pages) {
        if x != y {
            let at = format!("page {}", x.id);
            return shapes(&x.shapes, &y.shapes, &at).or(Some(format!("{at} props/sheet/connects")));
        }
    }
    for (id, m) in &a.masters {
        let Some(n) = b.masters.get(id) else { return Some(format!("master {id} missing")) };
        if m != n {
            return shapes(&m.shapes, &n.shapes, &format!("master {id}")).or(Some(format!("master {id} props")));
        }
    }
    let parts = [
        ("masters", a.masters.len() != b.masters.len()),
        ("styles", a.styles != b.styles),
        ("colors", a.colors != b.colors),
        ("default styles", a.default_styles != b.default_styles),
        ("theme", a.theme != b.theme),
        ("media", a.media != b.media),
    ];
    parts.iter().find(|(_, differs)| *differs).map(|(what, _)| (*what).to_owned())
}

fn crdt_check(files: &[PathBuf]) -> Result<()> {
    use flowster_crdt::document::Document;
    use flowster_crdt::model_io::{canonical_model, seed_of};
    let mut failed = 0;
    for file in files {
        let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        let pkg =
            flowster_vsdx::OpcPackage::open(bytes.clone()).with_context(|| format!("opening {}", file.display()))?;
        let model = flowster_vsdx::visio::read_document(&pkg)?;
        let theme = flowster_vsdx::visio::theme_xml(&pkg);
        let parse = |b: &[u8]| flowster_vsdx::visio::parse_theme_xml(b);
        let t0 = Instant::now();
        let mut doc = Document::new();
        doc.import_model(&model, theme.as_deref(), seed_of(&bytes));
        let import_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let snapshot = doc.snapshot();
        let t1 = Instant::now();
        let copy = Document::from_snapshot(&snapshot, doc.assets().clone()).map_err(anyhow::Error::msg)?;
        let back = copy.read_model(parse);
        let read_ms = t1.elapsed().as_secs_f64() * 1000.0;
        let expected = canonical_model(&model);
        fn count(s: &[flowster_core::model::Shape]) -> usize {
            s.iter().map(|x| 1 + count(&x.shapes)).sum()
        }
        let shapes: usize = model.pages.iter().map(|p| count(&p.shapes)).sum::<usize>()
            + model.masters.values().map(|m| count(&m.shapes)).sum::<usize>();
        let verdict = match first_difference(&expected, &back) {
            None if expected == back => "ok".to_owned(),
            None => "DIFF (unlocated)".to_owned(),
            Some(at) => format!("DIFF at {at}"),
        };
        if verdict != "ok" {
            failed += 1;
        }
        println!(
            "{:<40} {} pages, {} shapes, snapshot {:.1} KB, import {:.1} ms, reload+read {:.1} ms: {verdict}",
            file.display(),
            model.pages.len(),
            shapes,
            snapshot.len() as f64 / 1024.0,
            import_ms,
            read_ms
        );
    }
    if failed > 0 {
        bail!("{failed} of {} files did not round-trip", files.len());
    }
    Ok(())
}

fn save_check(files: &[PathBuf], out: Option<&PathBuf>) -> Result<()> {
    if let Some(d) = out {
        std::fs::create_dir_all(d)?;
    }
    let mut failed = 0;
    for file in files {
        let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        let pkg = flowster_vsdx::OpcPackage::open(bytes).with_context(|| format!("opening {}", file.display()))?;
        let mut model = flowster_vsdx::visio::read_document(&pkg)?;
        let theme = flowster_vsdx::visio::theme_xml(&pkg);
        let title = file.file_stem().and_then(|s| s.to_str()).unwrap_or("Drawing");
        let saved = flowster_vsdx::writer::write_vsdx(&model, theme.as_deref(), title).map_err(anyhow::Error::msg)?;
        let pkg2 = flowster_vsdx::OpcPackage::open(saved.clone())?;
        let mut back = flowster_vsdx::visio::read_document(&pkg2)?;
        model.warnings.clear();
        back.warnings.clear();
        // Media part names follow the writer's layout; compare payloads by name.
        let rename = |m: &mut flowster_core::model::Document| {
            m.media = std::mem::take(&mut m.media)
                .into_iter()
                .map(|(k, v)| (k.rsplit('/').next().unwrap_or(&k).to_owned(), v))
                .collect();
        };
        rename(&mut model);
        rename(&mut back);
        let verdict = match first_difference(&model, &back) {
            None if model.pages == back.pages && model.masters == back.masters => "ok".to_owned(),
            None => "DIFF (unlocated)".to_owned(),
            Some(at) => format!("DIFF at {at}"),
        };
        if verdict != "ok" {
            failed += 1;
        }
        if let Some(d) = out {
            std::fs::write(d.join(file.file_name().unwrap_or_default()), &saved)?;
        }
        println!("{:<40} {} KB: {verdict}", file.display(), saved.len() / 1024);
    }
    if failed > 0 {
        bail!("{failed} of {} files did not round-trip", files.len());
    }
    Ok(())
}

fn bench_crdt(shapes: usize, library: Library, json: bool) -> Result<()> {
    use flowster_crdt::spike::{SpikeConfig, run};
    let probe = NativeProbe(Instant::now());
    let cfg = SpikeConfig { shapes, ..SpikeConfig::default() };
    let mut reports = Vec::new();
    if library != Library::Yrs {
        reports.push(run::<flowster_crdt::loro_store::LoroStore>(&cfg, &probe, "native"));
    }
    if library != Library::Loro {
        reports.push(run::<flowster_crdt::yrs_store::YrsStore>(&cfg, &probe, "native"));
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
    } else {
        for r in &reports {
            println!("{}", serde_json::to_string_pretty(r)?);
        }
    }
    Ok(())
}
