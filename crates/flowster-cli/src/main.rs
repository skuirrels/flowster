//! `flowster`: the headless CLI (SPEC §17.3). M0 commands:
//!
//! * `flowster info <file> [--json]`: parts and relationships of a package;
//! * `flowster export <file> <out.svg|out.json> [--page N]`: render a page to SVG (or its display list);
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
    /// Render a page to SVG, or dump its display list as JSON (by output extension).
    Export {
        file: PathBuf,
        out: PathBuf,
        /// 1-based page number; defaults to the first foreground page.
        #[arg(long)]
        page: Option<usize>,
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
        Cmd::Export { file, out, page } => export(&file, &out, page),
        Cmd::RecalcCheck { file, json, show } => recalc_check(&file, json, show),
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

fn export(file: &PathBuf, out: &PathBuf, page: Option<usize>) -> Result<()> {
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let pkg = flowster_vsdx::OpcPackage::open(bytes).with_context(|| format!("opening {}", file.display()))?;
    let doc = flowster_vsdx::visio::read_document(&pkg)?;
    let index = match page {
        Some(n) if n >= 1 && n <= doc.pages.len() => n - 1,
        Some(n) => bail!("page {n} does not exist ({} pages)", doc.pages.len()),
        None => doc.pages.iter().position(|p| !p.background).unwrap_or(0),
    };
    let dl = flowster_render::build_page(&doc, index).context("document has no pages")?;
    let is_json = out.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let body = if is_json { serde_json::to_string_pretty(&dl)? } else { flowster_render::svg::to_svg(&dl) };
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
