# Flowster

A cross-platform diagramming application (Tauri 2) that reads and writes Visio files. See [docs/SPEC.md](docs/SPEC.md).

Status: **M0 Foundations** is done; **M1 Viewer** is in progress (SPEC §24). The app opens `.vsdx` files and draws their pages on a zoomable canvas.

## Layout

| Path | What |
|---|---|
| `crates/flowster-core` | Engine core: document model, inheritance, transforms, geometry, themes, command bus, engine ↔ UI protocol, determinism probe |
| `crates/flowster-vsdx` | OPC/VSDX package reader (parts, content types, relationships), hardened against zip bombs, path traversal and DTDs |
| `crates/flowster-render` | Display-list builder (paths, text blocks, images) and SVG export |
| `crates/flowster-crdt` | §20.3 CRDT schema on Loro (plus yrs for the spike) and the live `Document` |
| `crates/flowster-sync-proto` | Binary sync wire frames (§20.8) |
| `crates/flowster-wasm` | wasm-bindgen facade the Engine Worker loads |
| `crates/flowster-cli` | `flowster info`, `flowster export <file> <out.svg>`, `flowster determinism`, `flowster bench crdt` |
| `src-tauri` | Desktop host with least-privilege capabilities |
| `ui` | React 19 + TypeScript + Vite; the engine runs in a Web Worker (`ui/src/engine`) |
| `docs/adr` | Architecture decisions (`0001`: CRDT library) |
| `tools/visio-oracle` | Windows-only: builds test drawings through Visio's COM API and exports Visio's reference renders (SPEC §22.3) |

## Prerequisites

- Rust stable (the toolchain file adds the `wasm32-unknown-unknown` target)
- Node ≥ 22 and pnpm 12
- `wasm-bindgen-cli`, pinned to the `wasm-bindgen` version in `Cargo.toml`:

  ```bash
  cargo install wasm-bindgen-cli --version 0.2.129 --locked
  ```

- Linux only: the WebKitGTK 4.1 development packages (see `.github/workflows/ci.yml`)

## Common commands

```bash
pnpm install
```

```bash
pnpm tauri dev
```

This builds the WASM engine, starts Vite and opens the desktop window.

```bash
cargo test
```

```bash
pnpm determinism
```

This checks that the native and WASM derived-state hashes match each other and the golden value.

```bash
pnpm spike
```

This runs the CRDT spike on 10,000 shapes, native and WASM. Run `pnpm build:wasm` first.

```bash
cargo run -p flowster-cli -- info fixtures/minimal.vsdx --json
```

```bash
cargo run -p flowster-cli -- export fixtures/minimal.vsdx target/minimal.svg
```

On a machine with Visio installed, this builds the local comparison corpus in `target/corpus` (never committed):

```bash
pwsh tools/visio-oracle/make-corpus.ps1
```

## Releases

Flowster is built and published on your own machines, the same way as Mapmeister and Tessera. The
process, commands and machine setup are in
[scripts/release-kit/README.md](scripts/release-kit/README.md); Flowster's settings are in
[release.config.mjs](release.config.mjs).

1. On either machine:
   ```bash
   make bump
   ```
   Then commit and push it. `make bump` sets the version in `src-tauri/tauri.conf.json`, `Cargo.toml`, `ui/package.json` and `Cargo.lock`.
2. On the Windows PC:
   ```bash
   git pull && make release
   ```
   That builds Windows, then Linux in WSL Ubuntu-22.04, then publishes both.
3. On the Mac:
   ```bash
   git pull && make release
   ```
   That builds Apple Silicon and Intel, then publishes them.

The download site is https://skuirrels.github.io/flowster-releases/ (GitHub Pages of the public
`skuirrels/flowster-releases` repo).

Release builds check the site's signed update feed at start-up, and ask in a native dialog before
installing. The check runs in the host (`src-tauri/src/lib.rs`); the webview has no updater
permission. Updates are signed with `~/.tauri/flowster-updater.key` (password in
`flowster-updater.key.password`). Its public half is `plugins.updater.pubkey` in
`src-tauri/tauri.conf.json`. Copy both files to every build machine and back them up: installed
copies only accept updates signed with this key.

## M1 viewer: what renders today

- **Shapes:** pages, background pages, layers (hidden layers), groups, masters with row-level inheritance, and styles with `BasedOn` chains.
- **Geometry:** every geometry row type — lines, arcs, elliptical arcs, ellipses, Béziers, polylines, NURBS — plus rounded corners.
- **Lines and fills:** line weight, colour, dash pattern, transparency and arrowheads; solid fills with transparency.
- **Text:** rich text with run and paragraph formatting, alignment, margins and the text-block transform.
- **Themes:** theme colours, variations and the line/fill/font styles behind `THEMEVAL()`.
- **Images:** embedded PNG, JPEG, GIF, BMP and SVG.

Not yet:
- **Formulas:** the formula engine. Cached `V` values are drawn, which is what Visio saved.
- **Fills and effects:** gradient fills (drawn solid), fill patterns, shadows and effects.
- **Text:** core text layout, so wrapping can differ slightly from Visio.
- **Images:** EMF/WMF images (shown as placeholders).
- **Other:** PDF/PNG export and `.vsd` import.

A plain `cargo test` skips `src-tauri`, because it embeds `ui/dist` at compile time. Run `pnpm build:wasm && pnpm build` first, then `cargo test --workspace`, as CI does.
