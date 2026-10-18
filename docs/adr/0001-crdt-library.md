# ADR 0001: CRDT library for the persistent document state

| | |
|---|---|
| Status | **Accepted** (M0 spike) |
| Date | 2026-10-18 |
| Decides | SPEC §5.3 "CRDT" row, §20.2, risk R9 |
| Decision | **Loro** (`loro` 1.16). `yrs` 0.28 stays the documented fallback behind `flowster-crdt/yrs`. |

## Context

The persistent half of the model (SPEC §6.9) is a CRDT document with the schema in §20.3. It must run identically in the WASM Engine Worker and natively (CLI, server). It must also meet the §20.10 budgets:

- open a 10,000-shape document from snapshot in under 3 s;
- a typical drag commit under 1 KB;
- apply a remote update in under 2 ms in WASM;
- CRDT memory at most 1.5× the plain model.

The spec prefers Loro and names yrs (Y-CRDT) as the fallback. This spike modelled the §20.3 schema in both libraries and measured them on the same synthetic document.

## What was measured

**Schema.** Both libraries hold the same §20.3 structure:

- the `meta`, `pages`, `page/<id>`, `masters` and `styles` roots;
- a `shape` root map of uuid → shape map, holding props, `sheet` and `text`;
- `Sheet` = map of sections:
  - single-row sections are maps;
  - named-row sections (`Prop`) are maps of row maps;
  - indexed sections (`Geometry1`, `Character`) are lists of row maps;
- every `CellEntry` and every atomic group (Position, Size, Rotation, Flip, Endpoints) is written as **one** LWW value;
- shape text is a rich-text container with a `bold` mark.

The two differ only where a library lacks a type:

| §20.3 element | Loro | yrs |
|---|---|---|
| `shapes/<page>` tree, z-order | `LoroTree` with fractional index; node meta holds the uuid | **No tree type.** Emulated with `parent` + `z` (fractional key) registers on each shape map. |
| Indexed sections (`MovableList<Row>`) | `LoroMovableList` | `ArrayRef`. **No move**, so a row reorder is delete + insert. |
| Rich text | `LoroText`, Peritext-style marks with per-key expand rules | `TextRef` with formatting attributes |
| Per-user undo | `UndoManager` (local origin, exclude prefixes) | `UndoManager` |
| Shallow snapshot | `ExportMode::shallow_snapshot` | none (GC only) |
| Presence | `EphemeralStore` | `Awareness` |

Code:

- schema: `crates/flowster-crdt/src/{loro_store,yrs_store}.rs`;
- library-neutral generator: `crates/flowster-crdt/src/synth.rs`;
- harness: `crates/flowster-crdt/src/spike.rs`;
- §20.5 conflict tests run against both: `crates/flowster-crdt/tests/schema.rs`.

**Document.** 10,000 shapes on 4 pages, which comes to 328,000 `CellEntry` values:

- 1 group per 50 shapes, each holding 4 children;
- 1 in 10 shapes is a 1-D connector whose `BeginX…EndY` formulas reference neighbours by canonical `{s:uuid}` tokens;
- 2-D shapes carry XForm (4 atomic groups), Line, Fill, Character, a 6-row Geometry1 and 2 `Prop` rows;
- text such as "Step 123 of the process", with one shape in 5 having a bold run.

The content is invented. Nothing comes from Visio.

**Metrics** (all per library and per target):

- **Memory:** live Rust heap bytes from a counting global allocator. This is requested bytes, not RSS.
- **Plain model:** the heap held by the library-neutral `SynthDoc`, a naive Rust model (strings for every key). The ratios are indicative, not a spec-grade baseline.
- **Load:** a fresh replica imports the snapshot; "read all" then deep-reads every shape. Each figure is the median of 3 runs.
- **Update sizes:** bytes that would go on the wire for one transaction:
  - Loro: `export(updates(vv))`;
  - yrs: `encode_state_as_update_v1(sv)`.
- **Remote apply:** the mean of 100 single-shape moves, then 100 keystrokes, applied to a replica loaded from the snapshot.
- **Convergence:** after all updates, the author and the remote replica must have the same digest and z-order.

**Environment.** Intel Core i7-8700, 32 GB, Windows 11, Rust 1.99.1 (release, thin LTO). WASM ran under Node 24.11 (V8, the engine inside WebView2), built with the `wasm-release` profile (fat LTO, `opt-level=3`).

Reproduce:

```bash
node scripts/build-wasm.mjs
node scripts/crdt-spike.mjs --shapes 10000 --json docs/adr/0001-crdt-spike-results.json
```

## Results

10,000 shapes, 4 pages. Raw data: [`0001-crdt-spike-results.json`](0001-crdt-spike-results.json).

| Metric | Loro native | yrs native | **Loro WASM** | yrs WASM | Budget (§20.10) |
|---|---:|---:|---:|---:|---|
| Plain model heap | 74.6 MiB | 74.6 MiB | 45.4 MiB | 45.4 MiB | |
| Build from scratch (≈ VSDX import) | 1,367 ms | 469 ms | **1,106 ms** | 255 ms | |
| Heap of the doc built from scratch (with history) | 349.5 MiB | 311.9 MiB | 309.5 MiB | 227.6 MiB | |
| Snapshot size | 14.5 MiB | 17.8 MiB | 14.9 MiB | 17.8 MiB | |
| Shallow snapshot (Loro) / v2 encoding (yrs) | 11.1 MiB | 14.9 MiB | 11.2 MiB | 14.9 MiB | |
| Snapshot export | 530 ms | 92 ms | 483 ms | 74 ms | |
| Snapshot import | 11 ms | 656 ms | **9 ms** | 422 ms | |
| Import + read every cell | 2,938 ms | 951 ms | **1,893 ms** | 607 ms | < 3 s to interactive |
| **Heap of loaded replica** | **57.5 MiB** | 342.7 MiB | **54.1 MiB** | 248.1 MiB | |
| Peak while loading | 59.3 MiB | 357.4 MiB | 55.8 MiB | 262.6 MiB | |
| **Memory vs plain model** | **0.77×** | 4.59× | **1.19×** | 5.47× | ≤ 1.5× |
| Update: move 1 shape | 194 B | 105 B | 194 B | 105 B | |
| **Update: drag commit, 10 shapes** | **944 B** | 991 B | **944 B** | 991 B | < 1 KB |
| Update: resize | 213 B | 155 B | 213 B | 155 B | |
| Update: type 1 character | 85 B | 53 B | 85 B | 53 B | |
| Update: type an 8-character word | 92 B | 57 B | 92 B | 57 B | |
| Update: bold a range | 93 B | 75 B | 93 B | 75 B | |
| Update: z-order change | 160 B | 62 B | 160 B | 62 B | |
| Update: create a shape | 1,699 B | 1,830 B | 1,699 B | 1,830 B | |
| Update: delete a shape | 196 B | 45 B | 196 B | 45 B | |
| **Remote apply: move** | 75 µs | 23 µs | **98 µs** | 21 µs | < 2 ms |
| Remote apply: keystroke | 20 µs | 22 µs | 53 µs | 16 µs | < 2 ms |
| Converged | yes | yes | yes | yes | |

**WASM binary size** (`wasm-release`, before `wasm-opt`):

- engine with Loro only: 3.44 MiB raw, 1.10 MiB gzip;
- adding yrs costs another 0.77 MiB raw, 0.24 MiB gzip.

The §5.3 budget is ≤ 6 MB compressed.

## Findings

1. **Memory decides it.** A replica loaded from snapshot costs 54 MiB with Loro against 248 MiB with yrs in WASM, and 57 against 343 MiB natively.
   - Only Loro meets the ≤ 1.5× budget.
   - yrs keeps every item as a linked block struct with string keys per map entry. At 328k cells that is about 5× the plain model.
   - Inside a ~2 GB practical WASM heap (R7), yrs would cap documents at a few tens of thousands of shapes.
2. **Loro opens lazily.** Snapshot import takes 9 ms; containers decode when first touched.
   - A deep read of all 328k cells takes 1.9 s in WASM. That is inside the 3 s budget but without much headroom.
   - The engine should materialise the visible page first and the rest in idle time.
   - The deep read here uses `get_deep_value()`, which allocates a throwaway value tree. Engine-style reads straight into derived caches should be cheaper. Re-measure in M1 with the real ShapeSheet loader.
   - yrs reads everything faster (0.6 s), but only after a full, eager 0.4 s decode, and then holds 4.6× the memory.
3. **Both meet the update-size and apply-latency budgets by a wide margin.**
   - A 10-shape drag commit is 944 B (Loro) or 991 B (yrs).
   - Applying a remote update takes ≤ 0.1 ms against a 2 ms budget.
   - yrs single-op updates are 30–75 % smaller.
   - Loro's move is 194 B because the whole Position group (a map with its key strings) is re-encoded. A compact `CellEntry` encoding (positional array instead of `{f, v, u}` maps) would shrink every update and snapshot. Decide this before the M1 schema freeze.
4. **The schema fits Loro and doesn't fit yrs.** yrs has no tree and no movable list, so on yrs Flowster would own:
   - cycle prevention for concurrent reparenting (the §20.5 row is "prevented by the tree CRDT");
   - fractional-index generation and tie-breaking;
   - orphan repair;
   - a row reorder that loses concurrent edits to the moved row, because it is a delete + insert.

   Loro provides all of these natively, plus mark expand rules for Peritext semantics, a shallow snapshot for server compaction (§20.8), and an ephemeral store for presence.
5. **Import from scratch is the slow path on Loro.**
   - Building 10k shapes takes 1.1 s in WASM against 0.26 s for yrs.
   - The fresh document holds 310 MiB because of history and op metadata.
   - **Import flow for local `.vsdx` files:** build, export a (shallow) snapshot, reload it. That lands at the 54 MiB loaded figure.
   - Snapshot export (≈ 0.5 s) belongs in worker idle time (autosave, compaction), never on the input path.
6. **WASM memory growth was the biggest performance bug found, and it is not library-specific.**
   - The stock wasm32 allocator grows linear memory 64 KiB at a time. Each `memory.grow` cost 0.7 ms on a small heap and 4.6 ms at 1 GiB on this machine's V8, with or without a declared maximum.
   - The first Loro WASM run took **18.3 s** to build the document and grew quadratically.
   - `crates/flowster-wasm/src/wasm_alloc.rs` now wraps dlmalloc with a system layer that grows by at least 16 MiB or 1/8 of the heap. The build dropped to **1.1 s**, and every figure above uses it.
   - WebView2 runs the same V8, so this ships in the engine, not just the spike.
7. **Build friction.** `pest` ≥ 2.9 (a Loro dependency) pulls in `psm`, which needs clang to compile for wasm32. `flowster-crdt` pins `pest = "=2.8.8"`. Re-check on every Loro upgrade.
8. **Correctness.** Both libraries pass the §20.5 scenarios scripted so far:
   - move vs resize both survive;
   - concurrent moves never mix PinX/PinY;
   - concurrent typing merges character-wise;
   - z-order converges;
   - delete beats a concurrent edit.

## Decision

Use **Loro** for the persistent document state. `flowster-crdt` is the only crate that touches the library: `document.rs` holds the engine-facing `Document`, and the spike stores hold the rest. The yrs store stays behind the `yrs` feature as the fallback measurement until the M1 schema freeze, then moves out of the shipped WASM build.

## Consequences and follow-ups

- **Ship WASM without yrs.** The `spike` feature of `flowster-wasm` is on by default in M0 so the UI build and the spike share one artefact. Turn it off for release builds, which saves 0.77 MiB raw.
- **Compact `CellEntry` encoding**, decided in M1 (finding 3).
- **Page-at-a-time materialisation on open**, measured with the real loader in M1 (finding 2).
- **Import = build, snapshot, reload**, plus snapshot export in idle time (finding 5).
- **Measure on JavaScriptCore and WebKitGTK.** WKWebView (macOS) and WebKitGTK (Linux) do not use V8, so these WASM numbers describe Windows/WebView2 only. Bun runs JSC and is a cheap first data point; the real one comes from the Tauri app on each OS.
- **Shape data placement.** Shapes live in a nested `shape` map keyed by uuid, with the tree holding only hierarchy. Storing shape data in the tree node's meta map would delete it with the node, with no second index to keep consistent. Evaluate in M1.
- **No wasm threads.** The custom allocator assumes a single-threaded worker (no `atomics` target feature). Revisit if the engine adopts threads.
