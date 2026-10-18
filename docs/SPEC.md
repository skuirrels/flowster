# Flowster — Product & Engineering Specification

A cross-platform (Windows · macOS · Linux) diagramming application built with **Tauri 2**, functionally equivalent to Microsoft Visio and able to read and write native Visio files.

| | |
|---|---|
| Status | Draft 2: adds real-time co-editing (§20) and committed `.vsd` import (§19.5) |
| Date | 2026-10-17 |
| Reference build analysed | Microsoft Visio (Microsoft 365 Click-to-Run) `VISIO.EXE` 16.0.20430.20182, x64 |
| Raw reverse-engineering data | [`docs/re/`](re/) |

---

## 0. Ground rules (read first)

1. **Clean-room boundary.** This spec describes *behaviour, data formats and public interfaces*. We used reverse-engineering analysis (REA) only to inventory features, identifiers and file structures. Nobody may copy or translate disassembled code, and the spec contains none. The Visio file formats are publicly documented by Microsoft ([MS-VSDX], [MS-VSDX] ShapeSheet reference, the VDX XML schema). Where this spec and the public documentation disagree, the public documentation wins.
2. **No Microsoft assets.** Do not ship Microsoft stencils, templates, icons, help, themes, cursors, ribbon artwork or strings. The catalogues in `docs/re/` record *what exists* so we can size our own content. They are not a source to copy from. Users may open their own Visio stencils and templates, which is the interoperability goal.
3. **No trade dress.** The UI has the same *capabilities* and keyboard ergonomics as Visio (users' muscle memory matters), but it uses its own visual design, iconography and naming. The product is "Flowster", not "Visio".
4. **Identifiers are not expression.** We reuse names that the file format requires: ShapeSheet cell names, section names, function names and XML element names. Without them we couldn't interoperate.

---

## 1. Executive summary

Visio is a **formula-driven vector diagramming engine**. Every shape, page, style and document is a *ShapeSheet*: a spreadsheet of typed cells (`PinX`, `Width`, `LineColor`, `Geometry1.X3`, `Prop.Cost`, …) whose values are spreadsheet-like formulas that reference each other across shapes. Geometry, text layout, connector routing, theme colours, smart behaviour (control handles, right-click actions, resize logic) and data binding all live in these formulas. The UI, stencils and templates sit on top of that engine.

Flowster must therefore be built **engine-first**:

* a Rust **core** (`flowster-core`) that implements the document model, the ShapeSheet formula engine, geometry, text layout, routing, layout and VSDX I/O. It compiles to **WebAssembly** for the interactive app and to **native** for the CLI, exports and tests;
* a **Tauri 2 host** for OS integration (files, fonts, printing, clipboard, updates, plugins and data connectors);
* a **TypeScript/React front end** for the chrome (ribbon, panes, dialogs) and a custom **canvas renderer** that consumes a display list produced by the core.

Fidelity target: open any `.vsdx` produced by Visio 2013+ and render it visually equivalent, edit it, and save it back so that Visio opens it with no loss. That includes data Flowster doesn't understand. Legacy binary `.vsd` files from Visio 2003–2010 (and later 2000/2002) are **imported** into the same model and saved as `.vsdx`.

Flowster is **collaborative by design**. The persistent document state is a CRDT (§20), so several people can edit the same diagram in real time, see each other's cursors and selections, work offline and merge cleanly. Every client recomputes formulas, geometry and routing locally and deterministically from the shared inputs. Local-only `.vsdx` files remain a first-class way to work.

---

## 2. Reverse-engineering findings

### 2.1 Method

All analysis was static and ran on the installed binaries and content under `C:\Program Files\Microsoft Office\root\Office16\`:

* PE header, import, export and resource parsing (`pefile`) of `VISIO.EXE`, `VISLIB.DLL` and the `VIS*.DLL` satellites;
* string extraction (ASCII + UTF-16) and C++ RTTI type-descriptor harvesting from `VISLIB.DLL`;
* resource extraction from `1033\VISINTL.DLL` (7,160 localized UI strings, 9 accelerator tables);
* COM type-library dump of the embedded `TYPELIB` (the automation object model);
* unpacking and statistical analysis of all shipped templates (`*.vstx`) and stencils (`*.vssx`).

### 2.2 Binary layout

| Module | Size | Role (inferred) |
|---|---|---|
| `VISIO.EXE` | 1.3 MB | **Stub launcher.** 3.5 KB of code; imports a single ordinal (`#1`) from `VISLIB.dll`; the rest is icons, version and manifest. |
| `VISLIB.DLL` | 32.7 MB | **The application.** 17.6 MB code, MFC (`mfc140u`, 903 imports), ConcRT task scheduler, 488 C++ exports, 5,085 RTTI classes, embedded COM type library, 56 NetUI `UIFILE` layouts, 162 cursors. |
| `VISBRGR.DLL` / `VISBRGRTG.DLL` | 13 / 17 MB | Office **PTLS6 line-services** text layout engine (exports `Ls*` functions): line breaking, justification, complex scripts. |
| `VISUTILS.DLL`, `VISWEB.DLL`, `VISCOLOR.DLL` | small | Add-on libraries ("VisioLibMain" entry point): utility add-ons, Save-as-Web/HTML, colour tools. |
| `VISOCX.DLL` | 0.3 MB | ActiveX drawing control (embeddable viewer). |
| `VISSHE.DLL` | 0.3 MB | Shell extension (thumbnails/preview). |
| `VISXDATA.DLL` | 0.1 MB | External-data COM server. |
| `VISDLGU.DLL` | 0.3 MB | "DlgMaker" declarative dialog runtime. |
| `1033\VISINTL.DLL` | 0.9 MB | Localized strings, accelerators, dialog templates. |
| `MSO*.DLL` (shared) | — | Office shared runtime: ribbon (`mso40ui`), backstage, themes (DrawingML), SVG (`msosvg`), spelling, ink, telemetry, WebView2 hosting for add-ins and the Power Automate export. |

**Takeaway:** Visio is a mature MFC/COM desktop app with its own text layout, task scheduler and declarative UI runtime. Flowster replaces MFC with Tauri/WebView, PTLS with a Rust text stack, and COM automation with a TypeScript plugin API.

### 2.3 Subsystem inventory (from RTTI namespaces and class names)

| Subsystem | Evidence (class / namespace names) | Flowster module |
|---|---|---|
| Document model | `CVDocument`, `CDoc`, `CPage`, `CMaster`, `CStyle`, `CLayer`, `CSection`, `CRow`, `CCell`, `CConnect`, `CDataRecordset` | `core::model` |
| Formula engine | `CFormula`, `CFormulaCopier`, `CFormulaTracingHistory`, error literals `#REF! #NAME? #VALUE! #DIV/0! #N/A #NUM! #NULL! #DIM!`, volatile-function tracking ("Document contains volatile functions") | `core::sheet` |
| Geometry | `CM::` namespace: `GeometryPath`, `MoveToSegment`, `LineToSegment`, `ArcToSegment`, `EllipticalArcToSegment`, `EllipseSegment`, `InfiniteLineSegment`, `PolylineToSegment`, `NURBSToSegment`, `SplineStart/KnotSegment`, `RelMoveTo/RelLineTo/RelCubBezTo/RelQuadBezTo/RelEllipticalArcToSegment`, `Transform` | `core::geom` |
| Snap & glue | `CSnapMgr`, `CSnapPt/Line/Arc/Curve/Spline/Intersect/Glue/Gesture/Target`, `CSnapAdornment` | `core::snap` |
| Routing | `VRoutingAlgorithm`, `VMinOverlappingSegmentStrategy`, line-jump cells | `core::route` |
| Auto layout | `LayoutEngine::Algorithm`: `Sugiyama` (greedy layering, fork placement), `ForceBasedAlgorithm`, `RadialLayoutAlgorithm`, `CircularTransform`, `TreeLayoutAlgorithm`, `ListLayoutAlgorithm`, `GridCompositionAlgorithm`, `IncrementalLayoutAlgorithm`; legacy `Visgrf::` placement engine | `core::layout` |
| Text | `TextModel::` (runs, character/paragraph properties, fields, bullets, tab stops), `CTextComposition`, `CTextEditor`, `CTextEditRuler`, PTLS | `core::text` |
| Structured diagrams | containers, lists, callouts (`ContainerProperties`, `CContainerBounding`, `CalloutTarget`) | `core::structure` |
| Replace Shape | `ReplaceShape::` fix-ups: Accessibility, Callouts, Comments, Container, DataGraphic, DataLink, Foreign, Formulas, Glue, Layers, List, Name, Text, ZOrder | `core::ops::replace` |
| Validation | `StructValidation::CVSXRule/RuleSet/RuleFilter/RuleTest/Issue` | `core::validate` |
| Data | `CDataRecordset`, `CDataColumn`, `CDataConnection`, data-selector wizards (Excel, Access, SharePoint list, ODC/ODBC), `DataRefresh::` conflict resolution, `DataGraphic` | `core::data` + host connectors |
| Data Visualizer / "smart diagrams" | `D3::Visio::` (`SmartDiagram*`, `ShapeBinding`, `ConnectorBinding`, `CrossFunctionalFlowchartBinding`, `MasterSelector`) | `core::dataviz` (P2) |
| BPMN / Power Automate | `Flow::Visio::` (`FlowTrigger`, `FlowCondition`, `FlowSwitch`, `FlowForeach`, `FlowDountil`, `BPMNPropertyFormulaGetter`) | P2 / out of scope |
| Ink | `VisioInk::` (`CInkPen`, `CInkReco`, `CWetInkRenderer`, `CInkVSnap`) | `ui::ink` (P2) |
| Review | `CComment(s)`, `CReviewer`, coauthoring (`CoauthMergeEvent`) | `core::review` |
| Explorers | `CDocExplorer*` (Drawing Explorer tree), `CMasterExplorer*`, `CShapeSheetView` | UI panes |
| Export | `CExportOptionsBMP/GIF/JPG/PNG/TIF`, SVG, EMF, HTML (`VISWEB`), DWG/DXF, PDF/XPS fixed format | `core::export` |
| Undo | `CUndoMgr`, `CM::UndoUnit`, undo scopes in the object model | `core::undo` |

### 2.4 Object model (automation API)

The embedded type library exposes **97 co-classes, 118 dispatch interfaces and 167 enums**. Full dump: [`re/object-model.txt`](re/object-model.txt), [`re/enums.txt`](re/enums.txt). The core hierarchy, which Flowster's plugin API mirrors (§17):

```
Application ─┬─ Documents ── Document ─┬─ Pages ── Page ─┬─ Shapes ── Shape ─┬─ Cells/Sections/Rows
             │                         │                 ├─ Layers           ├─ Characters (text runs)
             ├─ Windows ── Window ── Selection           ├─ Connects         ├─ Hyperlinks, Comments
             └─ Settings, Addons, EventList              └─ Comments         └─ Shapes (group children)
                                       ├─ Masters ── Master (same drawing API as Page)
                                       ├─ Styles ── Style
                                       ├─ DataRecordsets ── DataRecordset ── DataColumns
                                       ├─ Validation ── RuleSets ── Rules / Issues
                                       └─ DocumentSheet, Comments, Colors, Fonts, MasterShortcuts
```

Scale indicators: `IVShape` 186 members, `IVDocument` 167, `IVApplication` 126, `IVPage` 99, `IVSelection` 95, `IVWindow` 87, `IVMaster` 85, `IVCell` 42. **714 built-in UI commands** (`VisUICmds`), **112 event codes**, **495 cell indices**, **28 section indices**, **28 layout algorithm presets**.

### 2.5 ShapeSheet usage in shipped content

Statistics over the **363 US stencils / 9,267 masters** shipped with Visio ([`re/shapesheet-usage.txt`](re/shapesheet-usage.txt)). These counts set the implementation priority of the formula engine and geometry:

* **Sections** (row count): Geometry 100,200 · User 31,673 · Character 17,874 · Connection 6,586 · Paragraph 6,439 · FillGradient 6,294 · Control 5,586 · Actions 4,277 · Property 3,512 · Scratch 3,230 · Field 1,606 · LineGradient 878 · ActionTag 139 · Hyperlink 78 · Tabs 11.
* **Geometry rows:** LineTo 187,575 · RelLineTo 171,655 · RelCubBezTo 166,554 · MoveTo 49,944 · RelMoveTo 47,955 · EllipticalArcTo 32,709 · RelEllipticalArcTo 28,376 · NURBSTo 16,012 · Ellipse 5,271 · ArcTo 5,190 · PolylineTo 3,456 · SplineKnot 1,417 · SplineStart 300 · RelQuadBezTo 167 · InfiniteLine 45. **Every row type is used, so all are required for P0 rendering fidelity.**
* **Functions:** 150 distinct functions in use. The top 20 are `GUARD` (193,568), `THEMEVAL` (136,701), `IF` (111,212), `THEMEGUARD`, `NOT`, `RGB`, `SETF`, `OR`, `LUM`, `TEXTHEIGHT`, `TEXTWIDTH`, `NURBS`, `OPENTEXTWIN`, `INDEX`, `SAT`, `ABS`, `AND`, `MAX`, `MIN` and `ISTHEMED`. Theme functions and text-metric functions are on the critical path.
* The localized string table holds descriptions for **~235 ShapeSheet functions** and **~417 cells**, which sizes the full surface.

### 2.6 File formats observed

Open/save filters found in UI strings: `.vsdx .vsdm .vssx .vssm .vstx .vstm` (Open Packaging Conventions XML, 2013+); `.vsd .vss .vst .vsw` (legacy binary, 2003–2010); `.vdx .vsx .vtx` (2003–2010 XML); `.vdw` (2010 web drawing); `.vtpx` (template package); export to `.svg .png .jpg .gif .tif .bmp .emf .pdf .xps .htm .dwg .dxf`; import `.dwg .dxf`, images, Office documents via OLE; data from Excel, Access, SQL/ODBC (`.odc .dsn`), SharePoint lists and CSV.

Template package layout, verified by unpacking `BASFLO_U.VSTX`:

```
[Content_Types].xml
_rels/.rels
docProps/{app,core,custom}.xml, docProps/thumbnail.emf
visio/document.xml                (DocumentSettings, Colors, FaceNames, StyleSheets, DocumentSheet)
visio/_rels/document.xml.rels     → pages, masters, windows, validation, theme
visio/pages/pages.xml (+ page1.xml, _rels/pages.xml.rels)
visio/masters/masters.xml (+ masterN.xml, _rels)   ← stencils / documents with masters
visio/windows.xml                 (workspace: open windows, docked stencils, zoom)
visio/validation.xml              (rule sets, rules, issues)
visio/theme/theme1.xml            (DrawingML theme + Visio `vt:` extensions)
```

Content types: `application/vnd.ms-visio.drawing.main+xml` (template: `…template.main+xml`, stencil: `…stencil.main+xml`, macro variants: `….macroEnabled.main+xml`), `…pages+xml`, `…page+xml`, `…masters+xml`, `…master+xml`, `…windows+xml`, `…validation+xml`, plus the standard OOXML theme, core, extended and custom properties. Relationship types are under `http://schemas.microsoft.com/visio/2010/relationships/{pages,page,masters,master,windows,validation,image,oleObject,comments,recordsets,recordset,connections,solutions,solution}`.

### 2.7 Built-in content (sizing only — not to be shipped)

* 228 distinct templates ([`re/template-catalog.txt`](re/template-catalog.txt)), each in US and metric variants, with right-to-left variants for wireframes and UML.
* 363 stencils / 9,267 masters per unit system ([`re/stencil-catalog.tsv`](re/stencil-catalog.tsv)). Families: basic shapes, flowchart, cross-functional flowchart, BPMN 2.0, EPC, Six Sigma/TQM, org charts, network (basic/detailed/rack), AWS and Azure architecture, UML (class/sequence/use case/activity/state/component/deployment), database (Chen/Crow's Foot/IDEF1X/UML), floor plans (office, home, site, HVAC, plumbing, electrical), engineering (electrical, circuits, P&ID, fluid power), software wireframes (web/mobile), timelines, Gantt, calendars, brainstorming, maps, value-stream maps, business frameworks (SWOT, PESTLE, value chain), infographics.

---

## 3. Goals and non-goals

### 3.1 Goals

1. **G1 Fidelity:** open, render, edit and save `.vsdx/.vssx/.vstx` with lossless round-trip of everything we don't edit. Import legacy `.vsd/.vss/.vst/.vsw` with shapes, text, masters, data and (P1) formulas intact.
2. **G2 Engine parity:** implement the ShapeSheet model and formula language so that third-party Visio stencils behave correctly (smart shapes, control handles, actions, resize behaviour).
3. **G3 Productive diagramming:** flowcharts, org charts, network and architecture diagrams, UML/ERD, BPMN, floor plans and wireframes, with connectors, auto-layout, containers and themes.
4. **G4 Cross-platform native feel** on Windows 10/11, macOS 12+ and mainstream Linux (Ubuntu 22.04+, Fedora 39+), with a ≤ 25 MB installer, fast start and native menus, dialogs and file associations.
5. **G5 Extensibility:** a TypeScript plugin API modelled on the Visio object model, plus a CLI for headless conversion and export.
6. **G6 Real-time co-editing:** many people edit one diagram at the same time, with live cursors and selections, offline editing that merges on reconnect, per-user undo, comments and version history. The server can be Flowster-hosted or self-hosted.

### 3.2 Non-goals (v1)

* Running VBA macros or COM add-ins. `.vsdm` macro projects are preserved byte-for-byte but never executed.
* Co-editing *with Microsoft Visio users*. Visio coauthoring runs through OneDrive/SharePoint and Microsoft's own service protocols. Flowster co-edits among Flowster clients, and Visio interop goes through `.vsdx` import and export.
* Writing legacy binary `.vsd`. It is import-only; saving produces `.vsdx`.
* DWG write and DWG import (needs a licensed ODA SDK). **DXF** import/export is in scope.
* Visio Services/SharePoint publishing, Power Automate export, PivotDiagrams, 3-D map content and ink handwriting recognition.
* Pixel-identical rendering. The target is perceptual equivalence (§22).

### 3.3 Personas

| Persona | Needs |
|---|---|
| Business analyst | Flowcharts, swimlanes, BPMN, validation, data-linked shapes, export to PDF/PNG for decks. |
| IT/cloud architect | Network and cloud diagrams, large stencil libraries, layers, auto-layout, SVG export. |
| Engineer/facilities | Scaled drawings (drawing scale, units), floor plans, DXF interop, precise sizing. |
| Visio power user | ShapeSheet editing, custom stencils, formulas, keyboard shortcuts identical to Visio. |
| Linux/Mac user stuck with `.vsdx` | Opens and edits files from colleagues without Windows. |

---

## 4. Feature scope and priority

`P0` = MVP (first public beta) · `P1` = v1.0 · `P2` = post-1.0 · `—` = out of scope.

| Area | Feature | Pri |
|---|---|---|
| Files | Open/save `.vsdx`, `.vssx`, `.vstx` (lossless round-trip) | P0 |
| | Open `.vsdm/.vssm/.vstm` (macros preserved, disabled) | P0 |
| | Import `.vdx/.vsx/.vtx` (2003–2010 XML) | P1 |
| | Import legacy binary `.vsd/.vss/.vst/.vsw`, Visio 2003–2010: shapes, values, text, masters, data (§19.5) | P0 |
| | `.vsd` formula decoding (keeps smart-shape behaviour), Visio 2000/2002 files | P1 |
| | Visio 5 and older `.vsd` | P2 |
| | Autosave, crash recovery, recent files, file locking | P0 |
| Export | SVG, PNG, JPEG, PDF | P0 |
| | DXF, EMF (Windows), HTML (static, multi-page with pan/zoom) | P1 |
| | GIF/TIFF/BMP, XPS | P2 |
| Print | Print with page tiling, fit-to, headers/footers, print preview | P1 |
| Drawing | Pointer, text, connector, connection-point, text-block, pencil, freeform, line, arc, rectangle, ellipse, crop and stamp tools | P0 (crop/stamp P1) |
| | Vertex editing, add/delete vertex, arc bow editing | P1 |
| | Shape operations: Union, Combine, Fragment, Intersect, Subtract, Join, Trim, Offset, Fit Curve | P1 |
| Shapes | Drag from stencils, masters & master inheritance, groups, 1-D/2-D shapes, control handles, actions (right-click) | P0 |
| | Replace Shape (with fix-ups, §2.3) | P1 |
| | Smart shape behaviours via ShapeSheet (resize logic, text auto-height, etc.) | P0 |
| ShapeSheet | Full model, formula engine, ~150 in-use functions | P0 |
| | Remaining documented functions, ShapeSheet editor window, formula tracing | P1 |
| Text | Rich text in shapes, character/paragraph/tab formatting, bullets, fields, text block rotation | P0 |
| | Spell check, find/replace, AutoCorrect, vertical text, RTL & complex scripts | P1 |
| Connectors | Dynamic connector (right-angle/straight/curved), glue (static/dynamic), line jumps, reroute | P0 |
| | AutoConnect arrows + Quick Shapes mini-toolbar, split connector on drop | P1 |
| Layout | Align/distribute, auto align & space, re-layout page (flowchart/hierarchy/tree/radial/circular/compact) | P1 |
| Structure | Containers, lists, swimlanes (cross-functional flowchart), callouts | P1 |
| | Sub-process creation, legends | P2 |
| Pages | Multiple pages, background pages, page setup, drawing scale, units, rulers, grid, guides | P0 |
| Layers | Layers (visible/print/lock/snap/glue/colour) | P1 |
| Themes | Themes, variants, quick styles, styles | P0 (render) / P1 (editing UI) |
| Data | Shape Data (properties), define/edit shape data, reports | P0 / P1 |
| | Link to external data (CSV, Excel, SQLite, JSON), refresh, data graphics | P1 |
| | Data Visualizer (generate diagram from table) | P2 |
| Validation | Rule sets in document, validate, issues pane; built-in Flowchart/BPMN/Swimlane rules | P1 |
| Review | Comments (threaded), alt-text, accessibility checker | P1 |
| | Ink/pen drawing | P2 |
| Views | Zoom/pan, Pan & Zoom pane, full-screen presentation, multiple windows per doc | P0 (multi-window P1) |
| Panes | Shapes (stencils, search, My Shapes), Size & Position, Shape Data, Format Shape, Layers, Drawing Explorer, Comments, Validation Issues, External Data | P0–P1 |
| Extensibility | Plugin API (TypeScript), CLI, custom ribbon via manifest | P1 |
| Collaboration | CRDT document model, deterministic engine (foundation; required from M0) | P0 |
| | Share a document, real-time co-editing, presence (cursors, selections, follow), per-user undo, offline merge | P0 |
| | Roles (owner/editor/commenter/viewer), link sharing, version history with named versions and restore | P0 / P1 |
| | Self-hosted sync server (Docker, single binary) and Flowster-hosted service | P0 |
| | LAN sessions without a server, end-to-end encryption | P2 |
| Content | Flowster's own stencils/templates (§16) | P0 core set, P1 extended |

---

## 5. Architecture

### 5.1 Process and runtime model

```
┌─────────────────────────────── Tauri 2 application ───────────────────────────────┐
│  Rust host (native)                                                                │
│  ├─ app shell: windows, menus (macOS app menu), tray-less, single-instance        │
│  ├─ fs service: atomic save, file locks, autosave dir, recent files               │
│  ├─ font service: enumerate system fonts, serve font bytes on demand              │
│  ├─ print service, clipboard service (multi-format), drag-drop                    │
│  ├─ data connectors: CSV, XLSX (calamine), SQLite, ODBC (opt), JSON/HTTP          │
│  ├─ plugin host: manifest, permissions, sandbox bootstrap                         │
│  ├─ updater, crash reporter (opt-in), logging                                      │
│  └─ flowster-core (native build) for CLI-equivalent jobs: batch export, thumbnails │
│                     ▲  Tauri IPC (commands + events, binary payloads)              │
│  WebView (WebView2 / WKWebView / WebKitGTK)                                        │
│  ├─ UI thread: React chrome (ribbon, panes, dialogs) + Canvas renderer             │
│  │        ▲ display-list diffs, hit-test results        │ commands, pointer events │
│  │        │                                              ▼                          │
│  └─ Engine Worker (Web Worker): flowster-core (wasm32) — owns live documents       │
│        CRDT document (Loro) = source of truth for persistent state                 │
│        derived caches: ShapeSheet recalc · geometry · text layout · routing        │
│        snap/glue · per-user undo · VSDX/VSD parse · VSDX serialize                 │
│        sync client (WebSocket) + presence ─────────────────────────┐               │
└──────────────────────────────────────────────────────────────────── │ ──────────────┘
                                                                      │ wss (binary CRDT updates,
                                                                      ▼  ephemeral presence)
┌──────────────── Flowster Sync Server (Flowster-hosted or self-hosted) ─────────────┐
│ axum/tokio rooms · update validation · Postgres|SQLite log + snapshots ·           │
│ blob store (images, fonts, OLE) · OIDC auth · ACLs · version history ·            │
│ flowster-core (native) for thumbnails, exports, search indexing                    │
└────────────────────────────────────────────────────────────────────────────────────┘
```

**Key decision: the live document lives in WASM inside the WebView, not in the Rust host.**
Rationale: interactive operations (drag, resize, rotate, connector rerouting) recalculate formulas on every pointer move. A Tauri IPC round trip (serialize → host → serialize back) per pointer event adds latency and jank, especially on WebKitGTK. A worker-hosted WASM core costs about one `postMessage` per frame. Keeping it off the UI thread keeps typing and scrolling smooth during heavy recalcs or layouts.

The **same `flowster-core` crate** compiles natively for the CLI (`flowster convert in.vsdx out.pdf`), headless thumbnail generation, golden-image tests and server use.

### 5.2 Repository layout (Cargo + pnpm workspace)

```
flowster/
├─ crates/
│  ├─ flowster-core/        # model, sheet, geom, text, route, layout, structure, validate, undo
│  ├─ flowster-vsdx/        # OPC/VSDX read/write, VDX import, unknown-part preservation
│  ├─ flowster-render/      # display-list builder (shared) + tiny-skia raster backend
│  ├─ flowster-export/      # SVG, PDF (krilla), PNG/JPEG, DXF, EMF (win), HTML
│  ├─ flowster-wasm/        # wasm-bindgen facade, worker protocol
│  ├─ flowster-cli/         # headless convert/export/validate/thumbnail
│  ├─ flowster-vsd/         # legacy binary .vsd/.vss/.vst/.vsw importer (MPL-2.0 licensed, §19.5)
│  ├─ flowster-crdt/        # model ↔ Loro schema, canonical formula refs, conflict rules (§20)
│  └─ flowster-sync-proto/  # wire protocol shared by client and server
├─ server/                  # flowster-server: sync rooms, auth, storage, history (Rust, axum)
├─ src-tauri/               # host app: commands, services, capabilities, bundling config
├─ ui/                      # React + TypeScript + Vite
│  ├─ src/engine/           # worker client, typed protocol, command bus
│  ├─ src/canvas/           # renderer, interaction controllers (tools), overlays
│  ├─ src/chrome/           # ribbon, backstage, panes, dialogs, status bar
│  ├─ src/plugins/          # plugin runtime (sandbox), public API typings
│  └─ src/i18n/
├─ content/                 # Flowster stencils & templates (source form → built .vssx/.vstx)
├─ tools/visio-oracle/      # Windows-only conformance harness driving Visio via COM (§22.3)
└─ docs/                    # this spec, ADRs, docs/re/
```

### 5.3 Technology choices

| Concern | Choice | Notes |
|---|---|---|
| Shell | Tauri **2.x** | Plugins: dialog, fs, opener, clipboard-manager, single-instance, window-state, updater, deep-link (file associations), log, process. |
| Core language | Rust (2024 edition), `#![forbid(unsafe_code)]` outside FFI crates | |
| WASM | `wasm-bindgen`, `wasm-opt -O3`, SIMD128 enabled; size budget ≤ 6 MB compressed | |
| XML | `quick-xml` streaming reader/writer | Must preserve unknown elements/attributes and namespaces. |
| ZIP/OPC | `zip` crate (deflate via `miniz_oxide`, works in WASM) | |
| Text shaping/layout | `parley` + `fontique` (host font enumeration) + `skrifa` (outlines/metrics) + `harfrust`/`swash` shaping | Same layout in WASM and native → identical `TEXTWIDTH/TEXTHEIGHT`. |
| Geometry | `kurbo` (Bézier/arc math), `lyon` (tessellation, for the WebGL backend), `i_overlay` (boolean ops) | |
| Raster export / tests | `tiny-skia` | |
| PDF | `krilla` (font subsetting, tagged PDF) | |
| UI framework | React 19 + TypeScript 5 + Vite; Zustand for chrome state; Radix UI primitives; own design system | Canvas is **not** React-managed. |
| Renderer | Canvas 2D (P0), WebGL2 tiled backend (P1) | §8.6 |
| Spreadsheet import | `calamine` (xlsx/xls/ods) | |
| DB | `rusqlite` (bundled); ODBC via `odbc-api` behind feature flag | |
| Plugin sandbox | Sandboxed `<iframe>` (opaque origin) + MessageChannel RPC | §17 |
| CRDT | **Loro** (Rust, WASM + native): map, movable list, movable tree with fractional-index ordering, rich text (Peritext-style marks), local-only `UndoManager`, shallow snapshots, ephemeral store for presence | Fallback if Loro proves unsuitable: `yrs` (Y-CRDT). Record the choice in an ADR after the M0 spike. |
| Deterministic math | `libm` crate (pure Rust) for all transcendental functions in core, in both WASM and native builds | Platform libm results differ in the last bits, and peers must compute identical derived state (§20.3). |
| Legacy `.vsd` container | `cfb` crate (OLE2 compound files) | §19.5 |
| Sync server | `axum`, `tokio`, `tokio-tungstenite`; `sqlx` (Postgres, or SQLite for self-host); S3-compatible or filesystem blob store; `openidconnect` | §20.8 |
| Tests | `cargo nextest`, `insta` snapshots, Playwright (via `tauri-driver`/WebDriver) for UI | |

### 5.4 Engine ↔ UI protocol

All edits flow through **commands**, so undo, macro recording, plugins and collaboration share one path.

```ts
// ui/src/engine/protocol.ts (abridged)
type DocId = string; type ShapeRef = { page: number; id: number };

type EngineRequest =
  | { kind: "open"; bytes: ArrayBuffer; fileName: string }
  | { kind: "exec"; doc: DocId; cmd: Command }            // undoable
  | { kind: "gesture"; doc: DocId; g: GestureEvent }      // pointer stream for tools
  | { kind: "query"; doc: DocId; q: Query }               // read-only
  | { kind: "save"; doc: DocId; format: "vsdx" | "vssx" | "vstx" }
  | { kind: "export"; doc: DocId; format: ExportFormat; opts: ExportOptions };

type EngineEvent =
  | { kind: "displayListPatch"; doc: DocId; page: number; patch: DisplayListPatch }
  | { kind: "selectionChanged"; doc: DocId; sel: ShapeRef[] }
  | { kind: "modelChanged"; doc: DocId; changes: ChangeSummary }  // drives panes
  | { kind: "overlay"; doc: DocId; items: OverlayItem[] }          // snap guides, handles, AutoConnect arrows
  | { kind: "undoState"; doc: DocId; canUndo: boolean; canRedo: boolean; labels: string[] }
  | { kind: "presence"; doc: DocId; peers: PeerPresence[] }                      // §20.7
  | { kind: "syncState"; doc: DocId; state: "local" | "connecting" | "live" | "offline"; pendingOps: number };
```

* `Command` is a tagged enum with ~300 variants in v1, modelled on `VisUICmds` (`re/enums.txt`), e.g. `ObjectGroup`, `AlignObjectLeft`, `SetLineWeight{…}`, `SetCellFormula{shape, cell, formula}`.
* Display-list patches are binary (`ArrayBuffer`, transferable) with a versioned schema; see §8.5.
* Host IPC (`invoke`) is used only for OS services: `fs_read`, `fs_write_atomic`, `fonts_list`, `font_bytes`, `print_pdf`, `clipboard_write_multi`, `data_query`, `plugin_*`.

### 5.5 Undo/redo

* Every command runs as one **CRDT transaction** tagged with the local origin. Undo/redo uses the CRDT's local `UndoManager`, so it reverses **only this user's own changes** and leaves collaborators' concurrent edits in place. That is the expected behaviour in shared documents, and it is identical for local-only files.
* Undo scopes nest (`BeginUndoScope/EndUndoScope` in the API). A whole gesture (a drag) is one scope, because previews during the gesture are not written to the CRDT (§20.6).
* If an undo targets something a collaborator has since deleted, that step is skipped and the status bar says so.
* Default depth is 100 steps (configurable).
* `Repeat` (F4) replays the last repeatable command on the current selection.

---

## 6. Document model

### 6.1 Entities

```
Document
 ├─ DocumentSheet (ShapeSheet for the doc: DocProps, User, Prop, Scratch, Hyperlinks…)
 ├─ DocumentSettings (snap/glue settings, default styles, protection flags)
 ├─ Colors (legacy palette), FaceNames (font table)
 ├─ StyleSheets[]          (ShapeSheet-backed styles; inherit via LineStyle/FillStyle/TextStyle)
 ├─ Themes[]               (DrawingML theme + Visio extensions; per page)
 ├─ Masters[]              (each: PageSheet + Shapes tree + icon + metadata; may be MasterShortcut)
 ├─ Pages[]                (foreground/background; BackPage link; PageSheet + Shapes + Connects)
 ├─ DataRecordsets[]       (columns, rows, primary key, connection, refresh settings)
 ├─ Validation             (RuleSets → Rules; Issues; ShowIgnored)
 ├─ Comments               (author, dates, page/shape anchors, threads)
 ├─ SolutionXML[]          (opaque named XML blobs owned by add-ins — preserved)
 ├─ Windows (workspace)    (open windows, zoom, docked stencils — read/write)
 ├─ Properties             (core/app/custom docProps)
 └─ Unknown parts          (preserved verbatim with their relationships)

Shape (Type = Shape | Group | Guide | Foreign)
 ├─ ID (unique per page/master), NameU/Name, UniqueID (GUID, lazily created)
 ├─ Master / MasterShape refs (inheritance source)
 ├─ LineStyle/FillStyle/TextStyle refs
 ├─ Sections → Rows → Cells        (the ShapeSheet)
 ├─ Text (runs referencing Character/Paragraph/Tabs rows by IX; fields)
 ├─ Data1/2/3, ForeignData (embedded image/OLE/metafile/SVG via rel)
 └─ Shapes[] (if Group)
```

### 6.2 Identity

* Shape IDs are integers, unique within a page or master, and stable across save/load. New IDs come from `max+1`.
* Names follow Visio's convention, `NameU` like `Process.12`; universal names are stable, local names are localizable.
* `UniqueID` GUIDs are created on demand (for data links, API `UniqueID(visGetOrMakeGUID)`).
* **The internal identity of every entity is a UUIDv7**, and it is the key in the CRDT. Formulas reference other sheets by UUID internally (§20.4), so the integer ID is a serialization attribute, not an identity. That lets two collaborators create shapes concurrently without ID clashes.
* The integer ID is kept as `legacyId` for round-trip. New shapes get one lazily. If two peers allocate the same integer, collisions are resolved deterministically at export (§20.4).
* The UUID is never written to VSDX except in a Flowster `SolutionXML` blob, which lets a re-imported file map back to its shared document.

### 6.3 Inheritance (critical)

Cell value resolution order for a shape cell:

1. **Local** cell on the shape (has `F` and/or `V` in XML).
2. **Master shape** (instance shapes inherit from the corresponding shape in the master; group children map by ID).
3. **Style** for that cell's category (line, fill or text cells → `LineStyle/FillStyle/TextStyle`, recursively through `BasedOn` style chains).
4. **Theme** via `THEMEVAL()`-family formulas (themes are reached through formulas, not as a separate inheritance tier).
5. **Application default** (No Style).

Rules:

* A cell absent in XML is **inherited**. `F='No Formula'` means the formula was explicitly deleted (value local, no formula). `F='Inh'` marks a cell that explicitly inherits.
* Sections and rows also inherit: an instance has its master's Geometry/Connection/User rows unless it overrides or deletes them (`Del='1'` on rows and sections).
* Editing a master propagates to all instances on recalc, except cells that are overridden locally.
* Inherited formulas are evaluated **in the context of the inheriting shape**. `Width*0.5` in a master evaluates against the instance's `Width`.

### 6.4 Coordinate systems & units

* Internal units: **inches** for lengths, **radians** for angles, **days** for durations/dates (OLE date), and unitless numbers. XML `V` is always internal units; `U` is the display unit (`IN`, `MM`, `CM`, `PT`, `FT`, `DEG`, `DA`, …).
* Page coordinates: origin bottom-left, +Y up. The canvas flips Y at render time only.
* Local shape coordinates: origin at the shape's bottom-left of its Width×Height box, +Y up.
* Transform from local to parent: `parent = Pin + R(Angle) · F(FlipX,FlipY) · (local − LocPin)`, where F mirrors about the local pin axis. *Verify flip semantics against the oracle (§22.3) before freezing.*
* 1-D shapes: `BeginX/BeginY/EndX/EndY` drive `PinX/PinY/Width/Angle` (Width = length, Height = thickness) through standard 1-D formulas.
* Drawing scale: `PageScale` / `DrawingScale` (e.g. 1 in = 10 ft). Shape cells are in drawing units; rendering uses page units. `DrawingScaleType`/`DrawingSizeType` drive page-setup UI.
* Unit literals in formulas, as found in the unit-name table: `IN, IN., ", INCH, INCHES, FT, ', FOOT, FEET, YD, MI, NM, MM, CM, M, KM, PT, P (picas), C/CI (ciceros), D/DI (didots), DEG, °, RAD, ', " (min/sec of arc), DMS, ED/EH/EM/ES/EW (elapsed), DATE, TIME, %`, area units `SQ IN`, `FT^2` and so on. Parsing is locale-aware for decimal and list separators.

### 6.5 Pages

* Foreground pages may reference one background page (`BackPage`); backgrounds can chain.
* PageSheet sections: Page Properties (PageWidth/Height, scale, shadow offsets, `UIVisibility`, `PageLockReplace/Duplicate`), Page Layout (placement and routing style, block/avenue sizes, line-jump settings), Print Properties (margins, orientation, `PagesX/Y`, `PaperKind`, `CenterX/Y`, `ScaleX/Y`, `PrintGrid`), Ruler & Grid, Layers, User, Prop, Scratch, Hyperlinks, Actions, Theme Properties.
* Auto-size page (`DrawingResizeType`): the page grows and shrinks in page-size tiles as shapes move near the edges.

### 6.6 Layers

Per page and per master. Row cells: `Name, NameUniv, Visible, Print, Active, Lock, Snap, Glue, Color, ColorTrans, Status`. Shapes belong via `LayerMember` (`"0;2"`). Layer colour overrides shape line/fill colours when set. Masters carry layers that merge into the page on drop, by name.

### 6.7 Connects

`Connect{FromSheet, FromCell, FromPart, ToSheet, ToCell, ToPart}` records glue. It is derived state: the source of truth is the formulas in the connector's `BeginX/EndX` (e.g. `PAR(PNT(Sheet.3!Connections.X2,Sheet.3!Connections.Y2))`) and the 1-D glue type. Flowster recomputes `Connects` on load and save for consistency, and writes them for Visio compatibility.

### 6.8 Data records

`DataRecordset{ID, Name, Columns[{Name, DisplayName, Type, Unit, Currency…}], PrimaryKey, Rows[]}` stored in `visio/data/recordsetN.xml` with a connection part. Shape links are `Prop.*` rows with `DataLinked='1'` plus shape-to-row mapping. Refresh conflicts are kept per recordset.

### 6.9 Persistent state vs derived state

The model splits into two layers. This split is what makes co-editing (§20) possible:

* **Persistent inputs** live in the CRDT document: entities, hierarchy, z-order, local cell formulas and constants, text, data and comments. Everything that is saved or shared lives here.
* **Derived state** is recomputed by each client and never shared: cell results, parsed formula ASTs, the dependency graph, geometry paths, dynamic-connector routes, `Connects`, text layout, display lists and thumbnails.

The same split applies to local-only `.vsdx` files. The file is imported into a CRDT document and exported back on save, so there is a single code path.

---

## 7. ShapeSheet engine (`core::sheet`)

### 7.1 Sections and cells

The engine supports every section in the ShapeSheet identifier table ([`re/shapesheet-identifiers.txt`](re/shapesheet-identifiers.txt)). Cell names below are the VSDX `N` attributes.

| Section (UI name) | Kind | Key cells |
|---|---|---|
| Shape Transform | single | `Width Height Angle PinX PinY LocPinX LocPinY FlipX FlipY ResizeMode` |
| 1-D Endpoints | single | `BeginX BeginY EndX EndY` |
| Alignment | single | `AlignLeft AlignCenter AlignRight AlignTop AlignMiddle AlignBottom` |
| Text Transform | single | `TxtPinX TxtPinY TxtWidth TxtHeight TxtLocPinX TxtLocPinY TxtAngle` |
| Text Block Format | single | `LeftMargin RightMargin TopMargin BottomMargin VerticalAlign TextBkgnd TextBkgndTrans DefaultTabStop TextDirection` |
| Line Format | single | `LineWeight LineColor LinePattern LineCap Rounding BeginArrow EndArrow BeginArrowSize EndArrowSize LineColorTrans CompoundType` |
| Fill Format | single | `FillForegnd FillBkgnd FillPattern FillForegndTrans FillBkgndTrans ShdwForegnd ShdwPattern ShdwForegndTrans ShapeShdwType ShapeShdwOffsetX/Y ShapeShdwObliqueAngle ShapeShdwScaleFactor ShapeShdwBlur ShapeShdwShow` |
| Gradient Properties | single | `FillGradientEnabled FillGradientDir FillGradientAngle LineGradientEnabled LineGradientDir LineGradientAngle RotateGradientWithShape UseGroupGradient` |
| Fill/Line Gradient Stops | rows | `GradientStopColor GradientStopColorTrans GradientStopPosition` |
| Additional Effects | single | `ReflectionTrans ReflectionSize ReflectionDist ReflectionBlur GlowColor GlowColorTrans GlowSize SoftEdgesSize SketchSeed SketchEnabled SketchAmount SketchLineWeight SketchLineChange SketchFillChange` |
| Bevel / 3-D Rotation | single | `BevelTopType/Width/Height BevelBottom* BevelDepth* BevelContour* BevelMaterialType BevelLightingType BevelLightingAngle RotationXAngle RotationYAngle RotationZAngle RotationType Perspective DistanceFromGround KeepTextFlat` |
| Quick Style / Theme | single | `QuickStyleLineColor QuickStyleFillColor QuickStyleShadowColor QuickStyleFontColor QuickStyleLineMatrix QuickStyleFillMatrix QuickStyleEffectsMatrix QuickStyleFontMatrix QuickStyleType QuickStyleVariation ColorSchemeIndex EffectSchemeIndex ConnectorSchemeIndex FontSchemeIndex ThemeIndex VariationColorIndex VariationStyleIndex EmbellishmentIndex` |
| Character | rows (IX) | `Font Color Style Case Pos FontScale Size DblUnderline Overline Strikethru DoubleStrikethrough Letterspace ColorTrans AsianFont ComplexScriptFont ComplexScriptSize LangID Highlight UseVertical Perpendicular RTLText` |
| Paragraph | rows (IX) | `IndFirst IndLeft IndRight SpLine SpBefore SpAfter HorzAlign Bullet BulletStr BulletFont BulletFontSize TextPosAfterBullet Flags` |
| Tabs | rows | `Position Alignment Leader` per stop |
| Text Fields | rows | `Value Format Type UICat UICod UIFmt Calendar ObjectKind` |
| Geometry*n* | sections, rows typed | section cells `NoFill NoLine NoShow NoSnap NoQuickDrag`; row types in §8.1 |
| Connection Points | rows | `X Y DirX DirY Type AutoGen Prompt` (and ABCD variant) |
| Controls | rows | `X Y XDyn YDyn XCon YCon CanGlue Tip Prompt` |
| Protection | single | `LockWidth LockHeight LockMoveX LockMoveY LockAspect LockDelete LockBegin LockEnd LockRotate LockCrop LockVtxEdit LockTextEdit LockFormat LockGroup LockCalcWH LockSelect LockCustProp LockFromGroupFormat LockThemeColors LockThemeEffects LockThemeConnectors LockThemeFonts LockThemeIndex LockReplace LockVariation` |
| Miscellaneous | single | `NoObjHandles NonPrinting NoCtlHandles NoAlignBox UpdateAlignBox HideText DynFeedback GlueType WalkPreference BegTrigger EndTrigger ObjType Comment IsDropSource NoLiveDynamics LocalizeMerge NoProofing Calendar LangID ShapeKeywords DropOnPageScale` |
| Events | single | `TheData TheText EventDblClick EventXFMod EventDrop EventMultiDrop` |
| Group Properties | single | `SelectMode DisplayMode IsDropTarget IsSnapTarget IsTextEditTarget DontMoveChildren` |
| Shape Layout | single | `ShapePermeableX/Y ShapePermeablePlace ShapeFixedCode ShapePlowCode ShapeRouteStyle ShapePlaceStyle ShapePlaceFlip ShapeSplit ShapeSplittable ConFixedCode ConLineJumpCode ConLineJumpStyle ConLineJumpDirX/Y ConLineRouteExt DisplayLevel Relationships` |
| Page Layout | single (page) | `PlaceStyle RouteStyle PlaceDepth PlowCode ResizePage EnableGrid DynamicsOff CtrlAsInput LineAdjustFrom/To BlockSizeX/Y AvenueSizeX/Y LineToNodeX/Y LineToLineX/Y LineJumpFactorX/Y LineJumpCode LineJumpStyle PageLineJumpDirX/Y PageShapeSplit PlaceFlip LineRouteExt AvoidPageBreaks` |
| Glue Info | single | `GlueType WalkPreference BegTrigger EndTrigger` |
| Image / Foreign | single | `ImgOffsetX ImgOffsetY ImgWidth ImgHeight ClippingPath Gamma Contrast Brightness Sharpen Blur Denoise Transparency` |
| Actions | rows | `Action Menu TagName ButtonFace SortKey Checked Disabled ReadOnly Invisible BeginGroup FlyoutChild` |
| Action Tags | rows | `X Y TagName XJustify YJustify DisplayMode ButtonFace Disabled Description` |
| Layer Membership | single | `LayerMember` |
| User-defined Cells | named rows | `Value Prompt` |
| Shape Data | named rows | `Value Prompt Label Format SortKey Type Invisible Verify DataLinked LangID Calendar` |
| Scratch | rows | `X Y A B C D` |
| Hyperlinks | named rows | `Description Address SubAddress ExtraInfo Frame NewWindow Default Invisible SortKey` |
| Reviewer / Annotation | rows | legacy markup (preserve only) |
| Document Properties | doc | `OutputFormat LockPreview AddMarkup ViewMarkup PreviewQuality PreviewScope DocLangID DocLockReplace DocLockDuplicatePage NoCoauth` |
| Style Properties | style | `EnableLineProps EnableFillProps EnableTextProps HideForApply` |
| Change Shape Behavior | single | `ReplaceLockShapeData ReplaceLockText ReplaceLockFormat ReplaceCopyCells` |
| Master Selectors / Diagram Bindings / Diagram Settings | rows | Data Visualizer (preserve in P0/P1; implement P2) |

Shape Data types (`Prop.*.Type`): 0 String · 1 Fixed list · 2 Number · 3 Boolean · 4 Variable list · 5 Date/time · 6 Duration · 7 Currency.

### 7.2 Value types

`Number(f64, unit)`, `String`, `Bool`, `Color` (index or RGB, plus transparency), `Date`, `Duration`, `Currency`, `Point` (`PNT`), `Polyline`/`NURBS` blobs, `SheetRef` tokens (for `GetRef`, `SETATREF`), `SetResult` (aggregate sets for validation: `GLUEDSHAPES`, `CONNECTEDSHAPES`, `SHAPESONPAGE`, `FILTERSET`), `Error(kind)`, where kind is one of `#REF! #NAME? #VALUE! #DIV/0! #N/A #NUM! #NULL! #DIM!`, and `Empty`.

### 7.3 Formula grammar (EBNF)

```
formula     := [ "=" ] expr
expr        := comparison
comparison  := concat { ("=" | "<>" | "<" | ">" | "<=" | ">=") concat }
concat      := additive { "&" additive }
additive    := term { ("+" | "-") term }
term        := power { ("*" | "/") power }
power       := unary { "^" unary }
unary       := ("-" | "+") unary | postfix
postfix     := primary [ "%" ]
primary     := number [ unit ] | string | "TRUE" | "FALSE"
             | function "(" [ arglist ] ")"
             | reference | "(" expr ")"
arglist     := expr { listsep expr }              (listsep = "," or locale ";")
reference   := [ sheetref "!" ] cellref
sheetref    := "Sheet." int | name-with-id | "ThePage" | "TheDoc" | "Pages[" name "]" [ "!" sheetref ]
             | "Masters[" name "]" | "Styles[" name "]" | "Doc" | "Page"
cellref     := cellname                            e.g. Width, PinX, LocPinX
             | section "." rowname [ "." cellsuffix ]   e.g. Prop.Cost, User.X.Prompt, Controls.Row_1.Y
             | section int "." cellname int        e.g. Geometry1.X3, Scratch.Y1, Connections.X2, Char.Size
string      := '"' { char | '""' } '"'
```

* Case-insensitive identifiers. Universal names (`NameU`) are used in files; the UI may show local names.
* Formulas are stored and displayed with the user's locale separators, and serialized to VSDX in universal syntax (`.` decimals, `,` separators).
* Implicit coercions follow Visio: booleans ↔ 0/1, strings to numbers on demand, units carried through arithmetic (length×scalar=length, length/length=scalar, and so on).

### 7.4 Evaluation & recalculation

* **Dependency graph** at cell granularity across all sheets of a document. Edges come from parsed references (precedents) and are indexed in reverse (dependents) for propagation.
* **Incremental recalculation:** a cell write marks dependents dirty; recalculation evaluates in topological order at transaction end and again at each gesture frame for live dynamics.
* **Cycles:** circular references are allowed in Visio for some patterns (e.g. Width ↔ text height via `GUARD`/`SETF`). Detect strongly connected components and evaluate each once per pass in document order, with a pass limit of 3. If values haven't stabilized after that, keep the last values and flag the formula-tracing window.
* **Volatile functions** (`NOW`, `RAND`, `RANDBETWEEN`, `PAGECOUNT`, `PAGENUMBER`, `DOCLASTSAVE`, …) recalc on events (save, page change), not on a timer.
* **User edits vs formulas:** when the UI changes a cell (e.g. dragging changes `PinX`), the engine overwrites the formula unless it is wrapped in `GUARD(...)`. In that case the UI action is blocked or redirected, matching Visio. `SETF(GetRef(cell), formula)` writes a formula into another cell, `SETATREF(…)` redirects UI edits of one cell to another, and `SETATREFEXPR/SETATREFEVAL` access the proposed value.
* **Triggers:** `DEPENDSON(...)` forces re-evaluation. Event cells (`EventDblClick`, `EventDrop`, `EventXFMod`, `EventMultiDrop`, `TheText`, `TheData`) evaluate their formulas *as actions* when the event fires (e.g. `OPENTEXTWIN()`, `GOTOPAGE("…")`, `DOCMD(1312)`, `RUNADDON("…")`).
* **Performance budgets:** parse ≥ 1 M formulas/s native; recalculating a single dragged shape with 300 dependent cells < 1 ms in WASM; full recalc of a 5,000-shape page < 250 ms.

### 7.5 Function library

Implement in three tiers. Counts are uses across shipped masters (§2.5).

**Tier A — P0 (≥ 100 uses, or required by core UX):**
`GUARD THEMEVAL IF THEMEGUARD NOT RGB SETF OR LUM TEXTHEIGHT TEXTWIDTH NURBS OPENTEXTWIN INDEX SAT ABS AND MAX MIN ISTHEMED HSL COS SIN HUE STRSAME SETATREFEXPR SQRT MODULUS SHAPETEXT BITXOR POLYLINE ATAN2 MASTERNAME ISERROR DOCMD RUNADDONWARGS BITAND LEN LOOKUP TINT SHADE SETATREF LUMDIFF FIELDPICTURE FONT GRAVITY MSOTINT FORMAT DEPENDSON PNT THEMEPROP TAN BOUND ARG CHAR ISERRVALUE ANG360 PAR INT PNTX PNTY FORMATEX ISERR LOC CEILING SETATREFEVAL RUNADDON ROUND THEMECBV SUM NA SIGN FIND IFERROR THEME BITOR LISTSHEETREF DAY REF PI CONTAINERSHEETREF NOW FORMULAEXISTS POINTALONGPATH TONE USE MAGNITUDE ATAN GETREF`

**Tier B — P0/P1 (observed in content):**
`RED GREEN BLUE NEARESTPOINTONPATH ID EVALTEXT LEFT DATETIME POW RIGHT MSOSHADE DATEVALUE MID DATE ISERRNA LOCALFORMULAEXISTS OPENGROUPWIN DEFAULTEVENT LOCTOPAR LISTMEMBERCOUNT USERUI ACOS FLOOR PAGENUMBER LISTORDER SATDIFF HOUR DEG RAD RECTSECT ASIN WEEKDAY MINUTE SECOND DOCLASTSAVE YEAR DOCLASTEDIT MONTH MASTERMAP PAGECOUNT EVALCELL DOCCREATION SHAPEBINDING CONNECTORBINDING LAYOUT FONTTOID THEMERESTORE RAND BLEND CREATOR ANGLETOLOC PATHLENGTH CELLISTHEMED LISTSEP FILENAME ANGLETOPAR MEMBEROFGRID CFFBINDING TITLE DIRECTORY DESCRIPTION`

**Tier C — P1 (documented, needed by validation, structure and fields):**
`AGGCOUNT GLUEDSHAPES CONNECTEDSHAPES SHAPESONPAGE FILTERSET HASCATEGORY ONLAYER ROLE IS1D CALLOUTCOUNT CALLOUTTARGETREF CONTAINERCOUNT CONTAINERMEMBERCOUNT STRSAMEEX UPPER LOWER TRIM SUBSTITUTE REPLACE REPT INTUP TRUNC INTERSECTX INTERSECTY ANGLEALONGPATH SEGMENTCOUNT HELP HYPERLINK GOTOPAGE PLAYSOUND CALLTHIS (stub → plugin registry) PAGENAME BKGPAGENAME HYPERLINKBASE KEYWORDS MANAGER COMPANY CATEGORY SUBJECT DOCLASTPRINT TIME TIMEVALUE LN LOG10 EXP COSH SINH TANH RANDBETWEEN`, then every remaining function in the localized description table (~235 total). Generate the authoritative list from the public ShapeSheet function reference, and diff it against the oracle (§22.3) to catch functions that the docs omit.

Rules:

* `CALLTHIS`, `RUNADDON` and `RUNADDONWARGS` map to the **plugin command registry** (§17). Unknown add-on names are a no-op plus a status-bar notice. Built-in Visio add-on names that matter for stock content (e.g. "Network Shape Properties", org-chart helpers) map to Flowster equivalents.
* `DOCMD(n)` maps Visio command IDs to Flowster `Command`s via a table generated from `VisUICmds`. Unsupported IDs are a no-op.
* Text metrics (`TEXTWIDTH`, `TEXTHEIGHT`) use the core text layout (§9) and are therefore deterministic across platforms.
* Theme functions (`THEMEVAL`, `THEMEGUARD`, `THEMEPROP`, `THEMECBV`, `ISTHEMED`, `THEME`) resolve through the page theme and variant (§11).

### 7.6 ShapeSheet window (P1)

A grid editor with collapsible sections, formula/value toggle, a formula bar with syntax highlighting and autocomplete (cells, functions, units), insert/delete row and section, change row type, "Show formula tracing" (precedents and dependents graph), editing for document, page and style sheets, and error display per cell. Opened with the Developer tab, the shape context menu, or Alt+double-click (Visio behaviour).

---

## 8. Geometry and rendering

### 8.1 Geometry rows

| Row | Cells | Semantics |
|---|---|---|
| `MoveTo` / `RelMoveTo` | X, Y | Start subpath (Rel: fractions of Width/Height). |
| `LineTo` / `RelLineTo` | X, Y | Straight segment. |
| `ArcTo` | X, Y, A | Circular arc to (X,Y); A = bow (signed sagitta). |
| `EllipticalArcTo` / `RelEllipticalArcTo` | X, Y, A, B, C, D | Arc to (X,Y) through control point (A,B); C = major-axis angle; D = axis ratio. |
| `Ellipse` | X, Y, A, B, C, D | Closed ellipse: centre (X,Y), point on major axis (A,B), point on minor axis (C,D). |
| `InfiniteLine` | X, Y, A, B | Line through two points, used by guides. |
| `PolylineTo` | X, Y, A | A = `POLYLINE(xType, yType, x1,y1, …)` intermediate points. |
| `NURBSTo` | X, Y, A, B, C, D, E | A last knot, B last weight, C first knot, D first weight, E = `NURBS(knotLast, degree, xType, yType, x1,y1,k1,w1, …)`. |
| `SplineStart` / `SplineKnot` | X, Y, A–D / X, Y, A | Legacy splines (convert to B-spline). |
| `RelCubBezTo` | X, Y, A, B, C, D | Cubic Bézier with relative control points (A,B),(C,D). |
| `RelQuadBezTo` | X, Y, A, B | Quadratic Bézier. |

* Each Geometry section is a path. `NoFill/NoLine/NoShow/NoSnap/NoQuickDrag` apply per section. A shape is closed (fillable) per subpath if it ends where it started.
* Path building yields `kurbo::BezPath` per section in local coordinates. Conversion is exact for line, cubic and quadratic segments, arcs use Bézier approximation with tolerance ≤ 0.001 in at 100 % zoom, and NURBS are converted to Béziers via knot insertion.
* Visual bounding box = path bounds + stroke + effects. The alignment box is Width × Height unless `NoAlignBox`/`UpdateAlignBox`.

### 8.2 Line formatting

* Weight (in, or pt in UI), colour and transparency, cap (round/square/flat), rounding (corner radius applied to path joints), compound type (single/double/thick-thin/thin-thick/triple).
* **Line patterns:** indices 0–23 standard patterns (solid, dashes, dots, dash-dot…) plus custom patterns via `USE("name")`. The dash array scales with line weight.
* **Line ends:** 45 standard arrowheads (index 0–45) with sizes 0–6 (very small … colossal). Arrowheads are geometry generated in core and trimmed from the line end to avoid overlap.
* Line gradients (stops, direction, angle).

### 8.3 Fill formatting

* Solid, 40 legacy patterns (`FillPattern` 2–40 hatch/shade patterns using fore/background colours), gradients (`FillPattern` 25–40 legacy linear/radial plus modern gradient stops), picture fill (via foreign data), and transparency per colour.
* Shadow: legacy page-offset shadow (`ShdwOffsetX/Y`) and modern shape shadow (`ShapeShdwType` outer/inner, offset, blur, scale, oblique angle).
* Effects (P1): glow, soft edges, reflection, bevel (approximated with gradients) and 3-D rotation (approximated; preserved exactly in file). Sketch effect (P2) uses a seeded jitter of the path (`SketchSeed`).

### 8.4 Groups, foreign objects, guides

* Groups have their own transform. Child coordinates are in group-local space. `DisplayMode` controls whether the group's own geometry draws behind, in front of, or not at all. `SelectMode` sets group-first versus member-first selection.
* Foreign shapes: images (PNG/JPEG/GIF/BMP/TIFF/EMF/WMF/SVG) with crop via `ImgOffset/ImgWidth/ImgHeight` and adjustments (brightness, contrast, gamma, transparency). EMF/WMF render through a Rust metafile interpreter on all platforms (P1). OLE objects show their cached preview image and can't be activated off Windows.
* Guides (`Type='Guide'`) are infinite lines that shapes glue to, and moving a guide moves the glued shapes.

### 8.5 Display list

The core emits a **retained display list per page**, so the renderer stays thin and every backend (Canvas, WebGL, tiny-skia, PDF, SVG) draws identical content:

```
DisplayItem {
  id: ShapeKey, z: u32, layerMask: u64, bounds: Rect,
  ops: [ PushTransform | PushClip | FillPath{path, paint} | StrokePath{path, stroke, paint}
       | GlyphRun{fontId, size, glyphs[{id,x,y}], paint} | Image{imageId, rect, filters}
       | Shadow{…} | Effect{…} | Pop ]
}
Paint = Solid(rgba) | Linear{stops,…} | Radial{…} | Pattern{patternId, fg, bg}
```

* Paths are shared by hash. Glyph outlines are shared per `(fontId, glyphId)`.
* Patches are `Upsert(items)`, `Remove(ids)` and `Reorder(ranges)`, sent per animation frame.
* Overlays (selection handles, snap indicators, connection points, AutoConnect arrows, drag previews, guides) are a separate ephemeral list.

### 8.6 Renderers

* **Canvas 2D (P0):** `Path2D` caches, glyph outline caching, devicePixelRatio-aware, culling by viewport, and level of detail (below 4 px on screen, draw the bounding box; skip text below 3 px).
* **WebGL2 tiled backend (P1):** for pages over 5,000 items or slow WebKitGTK. Tessellation in WASM via `lyon`, 512×512 tiles cached per zoom bucket, MSAA, and text from glyph outlines tessellated or via SDF atlas.
* **tiny-skia (native/WASM):** PNG/JPEG export, thumbnails (`docProps/thumbnail` written as PNG; we don't emit EMF thumbnails), stencil icons and golden tests.
* Background: page drawn with drop shadow on a neutral desk. Grid (fixed or variable density from Ruler & Grid cells), page breaks and rulers are drawn by UI overlays.
* Hit testing in core: precise path and stroke hit with a tolerance of 3 screen px, group semantics, and filtering by layer lock/visibility and `LockSelect`.

**Performance targets:** 60 fps pan/zoom with 10,000 shapes on mid-range 2022 hardware (WebView2 and WKWebView); ≥ 30 fps on WebKitGTK with the WebGL backend; first paint of a 2 MB VSDX < 1.0 s after file read.

---

## 9. Text (`core::text`)

* Text is stored as a run sequence with markers that reference **Character (cp)**, **Paragraph (pp)** and **Tabs (tp)** rows by index, plus **fields** (`<fld IX>`) that reference Text Fields rows. This mirrors VSDX `<Text>` with `<cp IX='n'/>`, `<pp/>`, `<tp/>` and `<fld/>`.
* **Layout:** a text block of `TxtWidth × TxtHeight` minus margins, vertical alignment top/middle/bottom, and paragraph alignment left/centre/right/justify/distributed. Line spacing (`SpLine` < 0 = multiple, > 0 = exact), space before/after, indents, bullets (`Bullet`, `BulletStr`, `BulletFont`), tab stops with leaders, and font scale, letter spacing, case, position (super/sub), underline, strike and small caps.
* **Overflow:** text is not clipped; it can extend beyond the block (Visio behaviour). Shapes use `TEXTHEIGHT` to grow.
* **Rotation:** `TxtAngle`, the text pin, and "keep text upright" behaviour via formulas (`BOUND`, `ANG360` patterns in content).
* **Fonts:** font-family fallback per script (Latin, Asian, Complex). Missing fonts are substituted with a metric-compatible fallback (Carlito for Calibri, Liberation for Arial/Times/Courier, Caladea for Cambria). The substitution is recorded and reported in the "Fonts" info panel.
* **Fields:** date/time (create, edit, print, current), document properties, page name/number/count, geometry (width, height, angle), shape data (`Prop.x`), custom formula fields, and number/date formats via `FORMAT` pictures (`0.00 u`, `#,##0`, `ddd, mmm d`, …).
* **Editing:** in-place editor on the canvas (IME, dead keys and emoji supported through a hidden `contenteditable` proxy that feeds composition events to core; the caret and selection are rendered by core), text ruler for tabs and indents, Format Text dialog, AutoCorrect, and spell-check (Hunspell dictionaries via host; P1).
* **Complex scripts & RTL (P1):** HarfBuzz-compatible shaping, bidi (UAX #9) and vertical text (`TextDirection`).

---

## 10. Interaction model

### 10.1 Tools (with Visio-compatible shortcuts)

| Tool | Shortcut | Behaviour |
|---|---|---|
| Pointer | Ctrl+1 | Select, move, resize, rotate, drag from stencils. |
| Connection Point | Ctrl+Shift+1 | Add/move/delete connection points (Ctrl+click adds). |
| Text | Ctrl+2 | Click for a text-only shape; select a shape to edit its text. |
| Crop | Ctrl+Shift+2 | Crop images and foreign objects. |
| Connector | Ctrl+3 | Drag shape-to-shape or point-to-point; creates a Dynamic connector. |
| Stamp | Ctrl+Shift+3 | Click repeatedly to stamp the selected master. |
| Pencil | Ctrl+4 | Lines and arcs by gesture; vertex editing. |
| Text Block | Ctrl+Shift+4 | Move, resize and rotate the shape's text block. |
| Freeform | Ctrl+5 | Smoothed spline (NURBS) drawing. |
| Line | Ctrl+6 | Straight lines; Shift constrains to 45°. |
| Arc | Ctrl+7 | Quarter arcs. |
| Rectangle | Ctrl+8 | Shift for square. |
| Ellipse | Ctrl+9 | Shift for circle. |

Also: lasso select, area select, multiple select (Shift/Ctrl+click), select by type (shapes, groups, guides, OLE, ink, layer, master), and Esc to deselect.

### 10.2 Handles and gestures

* Selection box with 8 resize handles. Corner handles keep aspect ratio when `LockAspect` is set or Shift is held. Ctrl resizes from the centre.
* Rotation handle with snapping to 15° (Shift for free rotation); the pin is draggable to change the rotation centre.
* 1-D shapes show begin and end handles and glue indicators.
* Yellow **control handles** (Controls section) drive `XDyn/YDyn` behaviour (`XCon/YCon` codes); hover shows the `Tip`.
* Text block handles in Text Block tool.
* Arrow keys nudge by one grid unit, Shift+arrow by one pixel. Alt-drag temporarily disables snap.
* Ctrl-drag duplicates. Right-drag offers a menu (move, copy, link, cancel).
* Double-click runs `EventDblClick` (default: edit text; groups open per `DblClick` behaviour; hyperlinks follow).
* Right-click shows Actions-section rows first (checked, disabled, flyout and grouping supported), then the standard context menu.

### 10.3 Snap & glue

* **Snap targets** (`VisSnapSettings`): ruler subdivisions, grid, guides, selection handles, vertices, connection points, geometry, alignment box, shape extensions, intersections. **Snap extensions** (`VisSnapExtensions`): alignment box extension, centre-axis, curve tangent, endpoint, midpoint, linear and curve extension, isometric angles, and so on. Snap strength is configurable in px.
* **Glue targets** (`VisGlueSettings`): guides, handles, vertices, connection points, geometry.
* **Dynamic grid:** shows alignment and spacing guides against nearby shapes while dragging; drop at the same spacing as neighbours.
* Glue types: **static/point** (to a specific connection point; the formula references `Connections.Xn`) and **dynamic/shape-to-shape** (`PAR(PNT(...))` to the shape and walking connection points, honouring `WalkPreference`). Visual feedback: a green or highlighted outline for shape glue and a highlighted point for point glue.
* Settings are stored in `DocumentSettings` (`GlueSettings`, `SnapSettings`, `SnapExtensions`, `SnapAngles`, `DynamicGridEnabled`) for round-trip.

### 10.4 Drag from stencil & AutoConnect (P1)

* Dragging a master onto the page creates an instance: it copies the master's styles, layers and theme behaviour, applies `DropOnPageScale`, fires `EventDrop`, and auto-names the shape (`Process.7`).
* Dropping on a connector splits it (`ShapeSplit/ShapeSplittable`). Dropping into a container or list adds membership.
* Hovering a shape shows **AutoConnect arrows** in 4 directions. Clicking one duplicates the shape (or the Quick Shape chosen from a mini-toolbar showing the first 4 masters of the active stencil) at the page's spacing and connects them with the default connector.
* "Quick Shapes" is each stencil's designated subset of masters, kept in Flowster stencil metadata or derived from master order.

### 10.5 Arrange & shape operations

* Align (left, centre, right, top, middle, bottom), distribute (horizontal and vertical, by centres or gaps), **Auto Align & Space**, rotate/flip, z-order (forward, backward, front, back), group/ungroup (Ctrl+G / Ctrl+Shift+U), position presets, and **Spacing Options**.
* Operations (P1): Union, Combine, Fragment, Intersect, Subtract, Join, Trim, Offset, Fit Curve. Implemented with exact boolean ops on flattened curves plus curve refitting; results are new geometry rows.
* Format Painter (Ctrl+Shift+P), copy/paste formatting, and Repeat (F4).

### 10.6 Find, replace, spell

* Find and replace across shape text, shape data, shape names and master names, with an optional scope (selection, page, all pages).
* Spelling (F7) over shape text, honouring `NoProofing` and `LangID`.

---

## 11. Themes, variants, styles

* Themes are DrawingML (`a:theme`) parts plus the Visio **`vt:` extensions** observed in `theme1.xml`: `vt:schemeID` (theme enum, e.g. 59 = Office), `vt:variationClrSchemeLst` (4 colour variants), `vt:variationStyleSchemeLst` (4 style variants), `vt:fmtConnectorScheme`, `vt:fmtSchemeEx`, `vt:fmtConnectorSchemeEx`, `vt:fillStyles`, `vt:lineStyles` (with `vt:lineEx` arrow/rounding), `vt:fontStylesGroup`, `vt:themeScheme`, and `vt:bkgnd` (background styles).
* Each page applies one theme and variant (`ThemeIndex`, `VariationColorIndex`, `VariationStyleIndex` in the PageSheet). Shapes participate through `THEMEVAL("FillColor")`-style formulas, the `QuickStyle*` cells (which select from the variant's colour and style matrices) and `ColorSchemeIndex/EffectSchemeIndex/ConnectorSchemeIndex/FontSchemeIndex`.
* Shape-level locks (`LockThemeColors`, `LockThemeEffects`, `LockThemeConnectors`, `LockThemeFonts`, `LockThemeIndex`, `LockVariation`) prevent theme changes.
* **Quick Styles gallery:** 7 style rows × variant colours. **Embellishment** levels. "Apply theme to new shapes" toggle.
* **Styles** (legacy ShapeSheet styles) are fully supported: define, apply (line, fill, text separately), `BasedOn` chains, and the Drawing Explorer listing. Default document styles: No Style, Normal, Text Only, None, Guide, Theme, Connector.
* **Flowster content:** we design our own themes (≥ 12 at P1) and export them as VSDX-compatible theme parts so that Visio can display them.

---

## 12. Connectors and routing (`core::route`)

* **Dynamic connector:** a 1-D shape whose geometry is computed by the router (`ObjType=2`, `ConFixedCode`, `ShapeRouteStyle`). It stores its path as geometry rows plus `Connects`, so any VSDX consumer sees the path.
* **Appearance:** right-angle, straight, curved (`ShapeRouteStyle`/`LineRouteExt`). Page-level `RouteStyle` (flowchart top-to-bottom, left-to-right, tree, organization chart, network, simple, centre-to-centre, …) sets the default.
* **Reroute modes:** freely, as needed, never, on crossover (`ConFixedCode`).
* **Algorithm:** orthogonal routing on a sparse visibility graph built from padded obstacle rectangles (padding = `AvenueSize`/`LineToNode` cells), A* with costs for length, bends (penalty), crossings and proximity to other connectors. Ports are connection-point directions (`DirX/DirY`) or the side facing the target. Incremental: only connectors whose endpoints or obstacles changed are rerouted. Curved style fits cubic Béziers through orthogonal waypoints.
* **Line jumps:** page `LineJumpCode` (none, horizontal, vertical, last routed, last displayed) and styles (arc, gap, square, triangle, 2–6 sided) with size factors. Computed after routing by segment-intersection sweep.
* **Connector text:** text positioned at the path midpoint, draggable via the text-block control handle.
* **Manual control:** midpoint and bend handles on right-angle connectors. Dragging one switches that connector to "never reroute" for the moved segments.
* **Budgets:** reroute 200 connectors < 50 ms in WASM; full-page reroute of 2,000 connectors < 1 s.

---

## 13. Structured diagrams (P1)

### 13.1 Containers

* A container is a shape with `User.msvStructureType="Container"` (and related `msvSDContainer*` user cells), which ensures interoperability with Visio containers.
* Membership is stored via relationships (`Relationships` cell formula with `DEPENDSON` lists) and the `msvSDMembers` conventions.
* Behaviour: auto-resize (off, expand only, expand and contract), margins, heading styles, lock membership, move members together, disband, select members, fit to contents, "add to new container" (from selection).

### 13.2 Lists

Ordered containers (e.g. UML class members, swimlanes in a CFF) with direction (top-to-bottom, left-to-right, …), alignment, spacing and drag reorder. Inserting a shape pushes the others along.

### 13.3 Cross-functional flowchart

Swimlanes (horizontal or vertical) as a list of lane containers inside a CFF container, plus phase separators. Adding or removing lanes and phases, and dragging shapes between lanes, updates membership and the `Prop.Function` formulas (e.g. `CONTAINERSHEETREF(1,"Swimlane")!User.VISHEADINGTEXT`).

### 13.4 Callouts

Callout shapes associate with a target via `CALLOUTTARGETREF`, move with the target and keep the leader line glued.

### 13.5 Sub-processes (P2)

Create a new page from a selection, replace it with a sub-process shape hyperlinked to that page, and keep off-page references.

---

## 14. Automatic layout (`core::layout`, P1)

| Preset family (from `VisLayoutAlgorithm`) | Algorithm |
|---|---|
| Flowchart TB / LR / BT / RL | Sugiyama layered: cycle removal (DFS), greedy layering, crossing minimization (barycentre + transposition), Brandes–Köpf coordinate assignment, fork placement (centre/left aligned) |
| Hierarchy (12 direction/alignment variants) | Layered tree with alignment option |
| Compact/Wide tree (8 directions) | Tidy-tree (Reingold–Tilford/Walker) |
| Radial | BFS rings, angular sector allocation by subtree size |
| Circular | Cycle-aware circular ordering, with components on separate circles |
| Force-based ("organic", internal) | Fruchterman–Reingold with overlap removal |
| Grid / List composition | For containers and lists |
| Incremental | Places newly added nodes and keeps existing positions |

Parameters come from the page's Page Layout cells: spacing (block and avenue size), connector style after layout, "avoid page breaks", "enlarge page to fit" and connector rerouting. The Re-Layout Page dialog previews the result live (runs in the worker; cancellable). Disconnected components are laid out separately and packed.

---

## 15. Data features

### 15.1 Shape Data (P0)

* The Shape Data pane edits `Prop.*` rows with type-specific editors: text, number with format, Boolean, date picker, duration, currency, and fixed and variable lists (from `Format` with `;`-separated options). `Invisible`, `Verify` (prompt on drop) and `SortKey` ordering are honoured.
* Define Shape Data dialog and Shape Data Sets (apply a set of properties to many shapes).
* Reports (P1): tabulate shape data across pages into a table shape or CSV/XLSX export, with filters and grouping.

### 15.2 External data (P1)

* Connectors in the host: **CSV/TSV, Excel (.xlsx/.xls/.ods via calamine), SQLite, JSON (file/URL with JSONPath row selector)**, and ODBC (Windows/Linux, behind a feature flag). SharePoint lists and Access are out of scope (Access is readable via ODBC on Windows).
* External Data window: a grid of recordset rows with link-status icons.
* Linking: drag a row onto a shape (link) or onto blank page (drop the default master linked); automatic linking by matching a column to shape text or shape data.
* Refresh: manual or on-open, with conflict resolution (deleted rows, duplicate keys) via a dialog. The recordset is stored in VSDX so that Visio can read it.
* **Data Graphics:** text callouts, data bars, icon sets and colour-by-value, positioned relative to the shape (horizontal and vertical position grids) and implemented as sub-shapes in a group (Visio's model: data graphic masters with `GraphicItems`). Legends (P2).

### 15.3 Data Visualizer (P2)

Generate flowcharts, cross-functional flowcharts and org charts from a table. Columns map to ID, description, next step, connector label, function/swimlane, phase, shape type and alt text. Two-way refresh. Uses the `Diagram Bindings / Diagram Settings / Master Selectors` sections for interoperability.

---

## 16. Content: stencils & templates

### 16.1 Strategy

* Flowster ships **its own** stencils and templates, authored in a source format (`content/**/*.toml` + SVG or ShapeSheet XML fragments) and compiled to `.vssx/.vstx` at build time. Our content opens in Visio, and the same loader also handles user-supplied Visio stencils.
* Masters use ShapeSheet formulas (theme-aware via `THEMEVAL`, resize-aware, text auto-height, connection points, control handles where useful). We design them from scratch against the public cell reference.
* Search: master keywords (`ShapeKeywords`), names and stencil names, with a local full-text index. Online shape search is out of scope.
* **My Shapes:** a user stencil folder (`~/Documents/Flowster/My Shapes`), add-to-My-Shapes, and editing masters and icons in a stencil editing mode.
* Third-party cloud icons (AWS, Azure, GCP): **licensing review required** before bundling. Fallback: generic cloud icon set plus an importer for the vendors' official SVG icon packs into stencils.

### 16.2 Content roadmap

| Pri | Templates | Stencils |
|---|---|---|
| P0 | Blank (US/metric), Basic Diagram, Basic Flowchart, Cross-Functional Flowchart, Block Diagram, Org Chart (manual), Basic Network | Basic shapes, arrows, callouts, containers, flowchart, CFF, org chart, basic network, connectors, borders & titles |
| P1 | BPMN 2.0, UML (class, sequence, use case, activity, state, component, deployment), Database (Crow's Foot, Chen, IDEF1X), Timeline, Calendar, Brainstorming/mind map, Office layout, Floor plan, Wireframes (web, mobile), Generic cloud architecture, Six Sigma, Value Stream Map, Business frameworks (SWOT, PESTLE, value chain), EPC, Gantt | ~60 stencils |
| P2 | Rack diagrams, detailed network, electrical/circuit, P&ID, HVAC, plumbing, site plan, maps, infographics | ~120 stencils |

Each template defines its page setup, theme, docked stencils (`windows.xml`), validation rule set (if any), page layout/routing defaults and sample content.

---

## 17. Extensibility (P1)

### 17.1 Plugin model

* A plugin is a folder or `.flowster-plugin` zip with `manifest.json` (id, version, permissions, contributions) and an ES module entry.
* It runs in a **sandboxed iframe** (opaque origin, strict CSP) and talks to the engine via an async RPC proxy over `MessageChannel`. No Node or Tauri APIs are exposed directly.
* Permissions: `document.read`, `document.write`, `fs.dialog` (user-picked files only), `network:<origins>`, `data.connectors`, `ui.ribbon`, `ui.pane`. They are granted at install with a per-plugin toggle.
* Contributions: ribbon tabs, groups and buttons (with a declarative JSON resembling RibbonX concepts), task panes (HTML in their own sandboxed iframe), context-menu items, commands (callable from `RUNADDON`/`CALLTHIS` via a `formulaName` alias), file importers/exporters, and validation rule sets.

### 17.2 API surface (mirrors the Visio object model)

```ts
interface Application { activeDocument: Document | null; documents: Documents; windows: Windows;
  undoScope<T>(label: string, fn: () => Promise<T>): Promise<T>; on(evt: AppEvent, cb): Disposable; }
interface Document { name; path; pages: Pages; masters: Masters; styles: Styles;
  dataRecordsets: DataRecordsets; validation: Validation; save(); saveAs(path, format); exportAs(fmt, opts); }
interface Page { name; id; isBackground; shapes: Shapes; layers: Layers; pageSheet: Sheet;
  drop(master, x, y): Promise<Shape>; dropConnected(from, to, connectorMaster?): Promise<Shape>;
  drawRectangle(x1,y1,x2,y2); drawLine(...); drawOval(...); drawNURBS(...); layout(opts?); autoSize(); }
interface Shape extends Sheet { id; nameU; text: string; master?: Master; parent: Shape | Page;
  shapes: Shapes; connects: Connect[]; gluedShapes(flags); connectedShapes(flags);
  containerProperties?: ContainerProperties; data: Record<string, CellValue>;
  linkToData(recordset, rowId); replaceShape(master, flags); delete(); duplicate(); group(); ungroup(); }
interface Sheet { cell(name: string): Cell; cellSRC(s, r, c): Cell; addSection(); addRow(); addNamedRow();
  sectionExists(); rowExists(); getFormulas(cells[]); setFormulas(map); getResults(cells[], unit?); }
interface Cell { name; formula: string; result(unit?): number; resultStr(unit?): string;
  setFormula(f: string, opts?: { force?: boolean }); setResult(v, unit?); isInherited; dependents(); precedents(); }
```

* Events mirror `VisEventCodes` (shape added/deleted/changed, cell changed, text changed, selection changed, before/after save, page added, connects changed, marker events, etc.), with filtering by object and cell (`SetFilterSRC` equivalent) to keep the event volume manageable.
* A **macro recorder** (P2) records commands as TypeScript.

### 17.3 CLI

```
flowster convert <in> <out>            # vsdx/vssx/vstx/vdx/vsd(*) → vsdx/svg/pdf/png/jpg/dxf
flowster export  <in> --page "Page-1" --format svg --dpi 192
flowster validate <in> [--ruleset Flowchart] --json
flowster thumbnail <in> <out.png> --size 256
flowster info <in> --json               # pages, masters, fonts, data recordsets, issues
```

---

## 18. User interface specification

### 18.1 Main window

```
┌───────────────────────────────────────────────────────────────────────────────────┐
│ ▣ Flowster │ QAT: Save Undo Redo ▾ │  Document title — Flowster  │ (A)(B)+2 [Share] │ — □ × │  (custom title bar on Win/Linux; native traffic lights on macOS)
├───────────────────────────────────────────────────────────────────────────────────┤
│ File  Home  Insert  Design  Data  Process  Review  View  Developer  [contextual]   │  Ribbon (collapsible: Ctrl+F1)
│ ┌Clipboard┐┌Font────────┐┌Paragraph┐┌Tools──────┐┌Shape Styles┐┌Arrange┐┌Editing┐  │
├───────────┬───────────────────────────────────────────────────────┬───────────────┤
│ Shapes    │  ┌ruler────────────────────────────────────────────┐ │ Task pane     │
│ [search ] │  │                                                  │ │ (Format Shape │
│ ▸ More    │  │                 canvas (page on desk)            │ │  / Shape Data │
│ ▾ Basic   │  │                                                  │ │  / Size&Pos / │
│   ▢ ○ △   │  │                                                  │ │  Comments /   │
│ ▸ Arrows  │  │                                                  │ │  Layers …)    │
│           │  └──────────────────────────────────────────────────┘ │               │
├───────────┴──┬────────────────────────────────────────────────────┴───────────────┤
│ ◂ ▸ Page-1 Page-2 Background-1 [+]   │ Page 1 of 3 │ Width: 1 in │ English │ ▭ 100% ─○─ ⤢│  Status bar
└──────────────────────────────────────────────────────────────────────────────────────┘
```

* Panes dock left or right, float, auto-hide and remember their layout per user (window-state plugin plus our own layout store).
* Multiple document windows: each document opens in its own OS window (macOS and Linux convention). On Windows, optional tabbed documents within one window.
* Dark, light and high-contrast chrome. The canvas page stays white unless the theme background says otherwise.

### 18.2 Ribbon

| Tab | Groups |
|---|---|
| **Home** | Clipboard (paste ▾, cut, copy, format painter) · Font (family, size, grow/shrink, B/I/U/S, colour, case, clear) · Paragraph (align H/V, indent, bullets, spacing, text direction) · Tools (Pointer, Connector, Text, Connection Point, Text Block, drawing tools ▾, Crop, Stamp) · Shape Styles (Quick Styles gallery, fill, line, effects) · Arrange (align, position, bring/send, group, Auto Align & Space, layers ▾) · Editing (find, replace, select ▾, change shape ▾) |
| **Insert** | Pages (new page, background page, duplicate) · Illustrations (picture, SVG, icons) · Diagram Parts (container, callout, connector) · Links (hyperlink) · Text (text box, field, symbol, equation P2) · Data Visualizer (P2) |
| **Design** | Page Setup (orientation, size, auto-size, page setup dialog) · Themes gallery · Variants gallery · Backgrounds (background, borders & titles) · Layout (re-layout page, connectors, spacing options) |
| **Data** | External Data (link data, refresh, data window) · Display Data (data graphics, legend P2) · Show/Hide (shape data window) · Shape Data (define, data sets, report) |
| **Process** | Subprocess (P2) · Diagram Validation (check diagram, rules to check, issues window, import rules) |
| **Review** | Proofing (spelling, thesaurus P2) · Language · Comments (new, delete, previous/next, pane) · Accessibility (checker, alt text, navigation order) |
| **View** | Views (full screen F5) · Show (rulers, page breaks, grid, guides, connection points, task panes ▾) · Zoom (zoom, fit to window, page width) · Visual Aids (dynamic grid, AutoConnect, connection points, snap & glue dialog) · Window (new window, arrange, switch) · Plugins |
| **Developer** (hidden by default) | Plugins (manage, reload, dev console) · Shape Design (operations ▾, group, ShapeSheet ▾, protection, behaviour) · Show/Hide (Drawing Explorer, Master Explorer) · Stencil (new stencil, edit master) |
| Contextual | Text Editing · Container Format · Picture Format · Connector |

Customize the ribbon and the Quick Access Toolbar (P1). Key tips appear with Alt (Windows/Linux), and the full command search ("Tell me") is on Ctrl+Q / Alt+Q.

### 18.3 Backstage (File)

New (template gallery by category, US/metric toggle, recent templates, blank; "New shared document") · Open (recent, browse, pinned, **Shared with me**, servers; `.vsdx` and `.vsd`) · Share (people and roles, link settings, server, convert local ↔ shared) · Version History (§20.1) · Info (properties, check for issues: accessibility, compatibility, **remove hidden information**: personal info, preview, unused masters, unused styles, data recordsets, validation rules) · Save / Save As (format selector) · Export (PDF, SVG, PNG, JPEG, DXF, HTML, with options) · Print (preview, printer, page range, fit, tiling, headers/footers) · Options.

### 18.4 Panes

Mapped from `VisWinTypes`:

* **Shapes** (stencils, search, My Shapes, More Shapes menu, Quick Shapes, view: icons and names / icons only / names / icon and detail)
* **Pan & Zoom**
* **Size & Position** (X, Y, W, H, angle, pin position)
* **Shape Data**
* **Drawing Explorer** (tree of the document: backgrounds, foreground pages → shapes, masters, styles, layers, fill/line patterns, line ends)
* **Master Explorer**
* **Formula Tracing** (P1)
* **External Data**
* **Validation Issues** (arrange by rule, category, page, ignored)
* **Format Shape** (fill, line, shadow, glow, soft edges, 3-D, text box)
* **Layers** (non-modal replacement for the Layer Properties dialog)
* **Comments**
* **Navigation/Accessibility order**

### 18.5 Dialogs (non-exhaustive)

Page Setup (Print Setup, Page Size, Drawing Scale, Page Properties, Layout and Routing, Shadows) · Snap & Glue · Ruler & Grid · Text (Font, Font Properties, Paragraph, Text Block, Tabs, Bullets) · Line · Fill · Protection · Behaviour (interaction style, selection highlighting, resize behaviour, group behaviour, double-click, placement, connector) · Shape Name · Define Shape Data · Hyperlinks · Layer Properties · Align / Distribute / Spacing Options · Re-Layout · Configure Layout · Container Properties · Data Selector wizard · Configure Refresh · Data Graphic editor · Rules to Check · Options (General, Proofing, Save, Language, Advanced: editing, display, units, precision, "always offer Metric and US units", "live preview in Shapes window"; Trust Center: external links prompt, plugin permissions; Customize Ribbon; QAT).

### 18.6 Keyboard shortcuts (drawing window)

Verified from Visio's default accelerator table ([`re/keyboard-shortcuts.txt`](re/keyboard-shortcuts.txt)). On macOS, **Ctrl ↦ ⌘** for all application shortcuts. Function keys keep their meaning, with alternatives where macOS reserves them.

| Keys | Command | Keys | Command |
|---|---|---|---|
| Ctrl+N / Ctrl+O / Ctrl+S | New / Open / Save | F12, Alt+F2 | Save As |
| Ctrl+P, Ctrl+Shift+F12 | Print | Ctrl+F2 | Print Preview |
| Ctrl+W, Ctrl+F4 | Close window/document | F6 | Next window |
| Ctrl+Z / Ctrl+Y | Undo / Redo | F4 | Repeat |
| Ctrl+X / C / V | Cut / Copy / Paste | Ctrl+Alt+V | Paste Special |
| Ctrl+D | Duplicate | Del | Delete |
| Ctrl+A | Select All | Esc | Deselect |
| Ctrl+F | Find | Ctrl+K | Insert Hyperlink |
| Ctrl+G, Ctrl+Shift+G / Ctrl+Shift+U | Group / Ungroup | Ctrl+Shift+F / Ctrl+Shift+B | Bring to Front / Send to Back |
| Ctrl+L / Ctrl+R | Rotate left / right 90° | Ctrl+H / Ctrl+J | Flip horizontal / vertical |
| F8 | Align Shapes | Ctrl+Shift+P | Format Painter |
| Ctrl+1…9, Ctrl+Shift+1…4 | Tools (§10.1) | F2 | Edit text |
| Ctrl+B / I / U | Bold / Italic / Underline | Ctrl+Shift+D | Double underline |
| Ctrl+Shift+A / K | All caps / Small caps | Ctrl+= / Ctrl+Shift+= | Subscript / Superscript |
| Ctrl+Shift+, / . | Font size down / up | F11 | Text dialog |
| Ctrl+Shift+L / C / R / J | Align left / centre / right / justify | Ctrl+Shift+T / M / V | Align top / middle / bottom |
| Ctrl+Shift+W | Fit page in window | Ctrl+Shift+I | Zoom 100 % |
| Alt+F6 / Shift+Alt+F6 | Zoom in / out | F5 | Full screen |
| Ctrl+PgUp / Ctrl+PgDn | Previous / next page | Ctrl+Alt+P | Reorder pages |
| Shift+F4 | Go To Page dialog | Shift+F5 | Page Setup |
| Shift+F11 | New page | Ctrl+F9 | Insert field |
| F9 / Shift+F9 | Toggle glue / snap | Alt+F9 | Snap & Glue dialog |
| F7 | Spelling | Ctrl+F1 | Collapse ribbon |
| Alt+F1 | Collapse Shapes window | Alt+F8 / Alt+F11 | Plugins / Developer console (replaces Macros/VBE) |
| Ctrl+Backspace / Ctrl+Del | Delete previous/next word (text) | Alt+← / Alt+→ | Navigate back / forward (hyperlinks) |

### 18.7 Accessibility

* Full keyboard operation, including moving and resizing selected shapes with arrow keys and keyboard-only connector creation (select two shapes, then *Connect*).
* A screen-reader model: a hidden DOM tree mirrors pages and shapes (role, name from `Title`/text, alt text, data summary), and follows the navigation order (`NavigationIndex`).
* Alt-text editing for shapes and pages (`AlternativeText`, `Title`), plus an accessibility checker (missing alt text, low contrast, reading order).
* High-contrast mode, reduced motion, and UI scaling up to 200 %.

### 18.8 Localization

* ICU MessageFormat strings and locale-aware number, date and unit formatting.
* Full RTL UI mirroring.
* Formula locale (separators, local function and cell names: universal names stored, local names displayed).
* P0 languages: en-US, en-GB. P1 languages: de, fr, es, it, pt-BR, ja, zh-CN, ko.

---

## 19. File I/O (`flowster-vsdx`)

### 19.1 VSDX/VSSX/VSTX read

* OPC: parse `[Content_Types].xml` and the `.rels` graph. Locate the main part by relationship type, not by fixed path.
* Streaming parse (`quick-xml`) into the model. Every element and attribute not modelled is retained in an `UnknownXml` side store, keyed by parent identity and position, and re-emitted on save. Unknown parts and relationships are kept verbatim.
* Supported parts: document, pages, page N, masters, master N, windows, validation, theme(s), comments (`visio/comments.xml`), data recordsets and connections, solutions (`SolutionXML`), media (images), embeddings (OLE, preserved), docProps, thumbnail, VBA project (`vbaProject.bin`, preserved and never executed).
* Robustness: tolerate missing optional parts, bad formulas (keep the `F` text, and use `V` as the result with an error flag), and duplicate IDs (remap with a warning). Corrupt ZIPs trigger recovery mode and salvage readable parts.
* Lazy loading: for large stencils, masters load on first use. Icons decode on display.

### 19.2 Write

* Deterministic output: stable part ordering and relationship IDs reused when present.
* Cells: write `V` (computed value in internal units), `U` (display unit) and `F` (formula) only when local (not inherited), mirroring Visio. Inherited cells are omitted. Write `F='No Formula'` where the formula was explicitly removed.
* Write `Connects`, `Text` with cp/pp/tp/fld markers, and the master `Icon` (generated PNG-in-ICO base64 for new masters).
* Atomic save: write to a temp file in the same directory, `fsync`, rename, and keep a `~$name.vsdx` lock file while open (Visio-compatible semantics; respect existing locks with a read-only fallback).
* **Compatibility check:** verify output with the oracle (§22.3) in CI on Windows. Every saved file must open in Visio without a repair prompt.

### 19.3 Other formats

| Format | Direction | Notes |
|---|---|---|
| `.vsdm/.vssm/.vstm` | R/W | Macro part preserved. Banner: "Macros are disabled in Flowster." |
| `.vdx/.vsx/.vtx` | R (P1) | 2003–2010 XML schema, which is close to VSDX semantics. Write is P2. |
| `.vsd/.vss/.vst/.vsw` | R (P0 for Visio 2003–2010) | Native Rust importer `flowster-vsd`, see §19.5. Never written; saving produces `.vsdx`. |
| SVG | W (P0), R as image/shape import (P1) | Export per page or selection with embedded fonts optional, text as `<text>` (searchable) or outlines, and `data-shape-id` attributes. Import converts SVG paths to geometry. |
| PDF | W (P0) | Vector output with embedded subset fonts, one PDF page per diagram page or tiled per print setup, internal hyperlinks between pages, external links, tagged PDF with alt text (P1). |
| PNG/JPEG | W (P0) | Resolution and DPI, background transparent or white, selection/page/all pages. |
| DXF | R/W (P1) | Layers ↔ layers, scale, polylines, splines, text, blocks for masters (via a Rust DXF crate). |
| EMF | W (P1, Windows only) | Via GDI+ on Windows. Elsewhere, offer SVG/PDF. |
| HTML | W (P1) | Self-contained viewer: pages as SVG, a JS pan/zoom/navigation shell, shape data on hover and hyperlinks. |
| Clipboard | R/W | Internal `application/x-flowster-shapes` (JSON + embedded VSDX fragment), SVG, PNG and plain text. Paste images and SVG from other apps. On Windows, also read and write `Visio 15.0 Shapes`/EMF where feasible (P2). |

### 19.4 Autosave & recovery

* Autosave interval (default 5 min) writes an incremental snapshot to `app_data/recovery/<doc-uuid>/`.
* On crash, the next launch offers recovery.
* Explicit save never touches the recovery files beyond cleaning them up.
* Shared documents (§20) don't need autosave: every change is persisted to the local update queue at once and synced to the server.

### 19.5 Legacy binary import: `.vsd/.vss/.vst/.vsw` (`flowster-vsd`)

Many organisations still hold years of Visio 2003–2010 files, so this importer is P0.

**Public references.** As far as we could establish, Microsoft publishes no complete open specification of the `.vsd` binary format.

* **[MS-VGSFF]** (the Visio Graphics Service `.vdw` format, on Microsoft Open Specifications) is the closest public Microsoft document. It must be evaluated in M0 to confirm how much of the shared binary structure it covers. We could not fetch it during this analysis.
* **libvisio** (Document Liberation Project, MPL-2.0) and **Apache POI HDGF** (Apache-2.0) document the format through working code.
* **Black-box pairs from the Visio oracle** (§22.3): Visio opens each `.vsd` and saves it as `.vsdx`, which gives us ground truth for every cell.

**Licensing approach.** `flowster-vsd` is licensed **MPL-2.0** as a separate crate. That lets engineers port logic from libvisio legally (MPL is file-level copyleft: we publish this crate's source, and the rest of Flowster is unaffected). The importer also runs in WASM, so there is no C++ dependency chain (librevenge, boost, ICU) to ship on three OSes.

**Format outline** (as described by libvisio and POI; confirm against the references in M0):

* The container is an OLE2 compound file with the main stream `VisioDocument`, plus OLE/VBA storages and summary-information streams.
* A trailer gives a tree of **pointers** to **streams**. Streams are optionally compressed with an LZ77-style scheme and hold typed **chunks**: document, styles, pages, masters, shapes, XForm, line, fill, text block, character/paragraph/tab rows, geometry rows, connection points, controls, user and Prop rows, layers, text, fields, foreign data, colour and font tables, and names.
* Cells carry their **value** and, where present, a **tokenized binary formula**.
* File versions: 11 = Visio 2003/2007/2010, 6 = Visio 2000/2002, ≤ 5 = older.

**Fidelity tiers**

| Tier | Pri | Imported |
|---|---|---|
| T1 Structure & appearance | P0 (v11) | Pages and background pages; shapes, groups, guides and foreign images; master hierarchy and inheritance; styles; all cell **values** for transform, line, fill, text block, character, paragraph, tabs and geometry; text with formatting and fields (as values); connection points, controls, connects and glue; layers; Shape Data, User and Scratch rows; hyperlinks; document properties; colour and font tables. Cells whose formula is not decoded become constants, so shapes look right but lose "smart" behaviour. |
| T2 Formulas | P1 | Decode tokenized formulas back to universal formula text (references, functions, operators, units), so control handles, actions and resize logic keep working. Unknown tokens keep the value and are flagged in an import report. |
| T3 Older & extras | P1 / P2 | Visio 2000/2002 (v6) at P1. Visio 5 and older at P2, via a libvisio helper process or not at all. Workspaces (`.vsw`) reopen the listed drawing and stencils. The VBA storage is carried into `.vsdm` output (P2). OLE objects show their preview image only. |

**User experience**

* `.vsd` files open like any other document, with a banner: "Imported from Visio 2003–2010 format. Save as .vsdx to keep your changes."
* An **import report** (Info pane) lists the cells kept as constants, substituted fonts and unsupported objects.
* `flowster convert old.vsd new.vsdx` handles batch migration, and folder-wide batch import (with a CSV summary) is in the CLI and the backstage.
* A `.vsd` can be **imported straight into a shared document** (§20.9).

**Acceptance criteria**

* Robustness: the parser is fuzzed (`cargo-fuzz`) with size and ratio limits on decompression.
* The corpus has ≥ 1,000 `.vsd` files: our VSDX corpus saved down to `.vsd` by the oracle, plus public samples.
* Per-file cell agreement against the oracle's `.vsdx` ground truth: ≥ 98 % of cell values at T1, and ≥ 95 % of formulas textually equivalent after normalization at T2.
* Rendering at T1 reaches SSIM ≥ 0.97 against Visio's PNG export.
* No crash on any corpus file. Unreadable streams degrade to partial import with a report.

### 19.6 VSDX import into an existing document

"Insert from file" brings pages, masters and styles from another `.vsdx` or `.vsd` into the current document (local or shared). It reconciles master and style names (match by `BaseID`/`UniqueID`, otherwise rename) and remaps formula references.

---

## 20. Real-time co-editing

### 20.1 Experience and scope

* **Two kinds of document.** A *local document* is a `.vsdx` file on disk. A *shared document* lives on a sync server, and each client keeps a full offline replica. **Share…** turns a local document into a shared one; **Download as .vsdx/.pdf/…** works from any shared document.
* **Live editing:** remote changes appear within 200 ms (p95) on the same region. Two people can type in the **same shape's text** at the same time and the edits merge character by character.
* **Presence:**
  * avatars in the title bar;
  * coloured, name-labelled cursors on the canvas;
  * collaborators' selections outlined in their colour;
  * page-tab badges showing who is on which page;
  * a "ghost" of shapes another user is dragging;
  * **Follow** mode (your viewport tracks theirs);
  * a "Jump to user" command.
* **Offline:** edits continue without a connection, are queued locally (persisted through the host to `app_data/sync/`) and merge automatically on reconnect. The status bar shows `Live`, `Offline – n changes pending` or `Syncing`.
* **Per-user undo** (§5.5).
* **Comments** are threaded, real-time and anchored to shapes or page points. They support @mentions (with notifications when the server has email configured) and can be resolved or reopened.
* **Version history:**
  * automatic checkpoints during activity, plus **named versions**;
  * view any version read-only, with a page-by-page visual diff (P1);
  * **restore** as new changes (history is never rewritten);
  * export any version as `.vsdx`.
* **Roles:** owner, editor, commenter (comments only) and viewer. Link sharing can be restricted to people, the organisation or anyone with the link. Roles are enforced on the server (§20.8).
* **Out of scope:** co-editing with Microsoft Visio clients (§3.2).

### 20.2 Replication approach (decision)

| Option | Verdict |
|---|---|
| Server-authoritative ops with locking | Rejected. No offline editing, poor latency, lock contention on shared shapes. |
| Operational Transformation | Rejected. A central transform server is required, and transforms for tree, structural and ShapeSheet operations are complex and error-prone. |
| **CRDT (Loro)** | **Chosen.** Offline-first; the server is a relay plus persistence; the same Rust library runs in WASM and native; it has a movable tree (grouping and z-order), rich text, local undo, shallow snapshots and presence built in. |

Rule: **replicate inputs, never outputs.** Only what a user or a command authored is replicated. Everything computable is recomputed locally (§6.9). That keeps updates small and avoids "computed value" conflicts.

### 20.3 CRDT schema (`flowster-crdt`)

```
root (Map)
├─ meta            Map    document settings, docProps, DocumentSheet rows, legacy palette & fonts
├─ pages           MovableList<PageId>              page order
├─ page/<id>       Map    { name, isBackground, backPage, pageSheet: Sheet, theme ref }
├─ shapes/<page>   Tree   node = shape (UUID); parent = group/page root; child order = z-order (fractional index)
├─ shape/<uuid>    Map    { type, nameU, legacyId, master, masterShape, lineStyle, fillStyle, textStyle,
│                            sheet: Sheet, text: RichText, foreign: BlobRef? }
├─ masters         Map<uuid, Master{ meta, pageSheet: Sheet, shapes Tree, icon: BlobRef }>
├─ styles          Map<uuid, Style{ name, basedOn…, sheet: Sheet }>
├─ themes          Map<uuid, BlobRef>               theme XML parts, replaced as a whole (LWW)
├─ data            Map<recordsetId, { columns: Map, rows: Map<rowId, Map>, connection, settings }>
├─ validation      Map    rule sets, rules, ignored issues
├─ comments        Map<threadId, { anchor, resolved, messages: MovableList<Map> }>
├─ unknown         Map<partPath | elementPath, BlobRef>   preserved VSDX content we don't model
└─ assets          Map<hash, { mime, size }>          content-addressed blobs (images, fonts, OLE)

Sheet = Map<sectionKey, Section>
  single-row sections → Map<cellName, CellEntry>
  named-row sections (User, Prop, Actions, Hyperlink, …) → Map<rowName, Row>
  indexed sections (Geometry n, Character, Paragraph, Tabs, Connection, Controls, Scratch) → MovableList<Row>
Row = Map<cellName, CellEntry> + { rowType, deleted }
CellEntry = { f?: CanonicalFormula, v?: constant, u?: unit, noFormula?: bool }   — written as ONE value (LWW)
```

**Atomic cell groups.** Some cells must change together or the merged result is nonsense. Each group below is stored as a single LWW value, so a concurrent move and resize can't produce a mix:

| Group | Cells |
|---|---|
| Position | `PinX, PinY` + `parentTag` (§20.5) |
| Size | `Width, Height, LocPinX, LocPinY` |
| Rotation | `Angle` |
| Flip | `FlipX, FlipY` |
| Endpoints | `BeginX, BeginY, EndX, EndY` + last resolved world positions (fallback for deleted glue targets) |
| Text transform | `TxtPinX … TxtAngle` |

Because these are separate groups, a move by one user and a resize by another both survive.

**Shape text** is a Loro rich-text container:

* character properties (font, size, colour, style, case, position, language, …) are marks;
* paragraph properties are marks on the paragraph-terminating character;
* fields are U+FFFC placeholders carrying a field-row reference.

At VSDX export this becomes `Character/Paragraph/Tabs` rows and `<cp/><pp/><tp/><fld/>` markers.

**Determinism requirement.** All peers compute derived state independently, so `flowster-core` must produce bit-identical results from identical inputs on every platform and in both WASM and native builds:

* no dependence on hash-map iteration order (use ordered maps or sort by UUID);
* `libm` for transcendental functions;
* no FMA contraction;
* stable sort with explicit tie-breaks in routing and layout;
* text layout from the same font bytes.

**Fonts in shared documents.** Text layout feeds `TEXTWIDTH/TEXTHEIGHT`, so every peer must lay text out with the same fonts. Shared documents resolve fonts deterministically from the bundled Flowster font pack plus fonts embedded in the document as assets (uploaded only when the font's embedding licence allows it). Locally installed fonts outside that set are mapped to the same metric-compatible substitute on every peer, and a notice shows which fonts were substituted.

### 20.4 References and identity

* Formulas are stored in **canonical form**: sheet references are UUID tokens, e.g. `PAR(PNT({s:0190…}!Connections.X2,{s:0190…}!Connections.Y2))`, and cell names are universal.
  * The UI shows and accepts the familiar `Sheet.12!`/`Process.3!` form. The editor converts both ways using the current names and `legacyId`s.
  * Renaming or renumbering a shape never rewrites anyone's formulas.
* `legacyId` collisions (two peers creating "Sheet.12" offline) are resolved deterministically at VSDX export: the lower `(creation Lamport timestamp, peer id)` keeps the number and the other gets `max+1`.
  * The new number is written back to the CRDT by the exporting client only if it has edit rights, so later exports are stable.
* `NameU` suffixes (`Process.12`) follow the same rule.

### 20.5 Conflict semantics

| Concurrent situation | Result |
|---|---|
| Two users edit the same cell or atomic group | Last writer wins (Lamport order, then peer id). The loser sees the value change with a brief highlight in the winner's colour. |
| Move vs resize of the same shape | Both apply (different atomic groups). |
| Edit vs delete of a shape or page | Delete wins. Edits to deleted entities are dropped, and the shape can be restored from version history or by the deleter's undo. |
| Grouping a shape while another user moves it | The Position group carries `parentTag`, the parent its coordinates were written in. If it differs from the current parent, the engine converts it through the tagged parent's world transform at read time. The result is deterministic and needs no write. |
| Concurrent reparenting that would create a cycle | Prevented by the tree CRDT's move semantics. |
| Z-order changes on both sides | Fractional indices converge. Ties break by peer id. |
| Connector glued to a shape another user deleted | The endpoint formula's reference resolves to "missing", and the engine uses the stored last world position, so the connector shows as unglued. Nothing is written until someone edits that connector. |
| Shape added to two different containers concurrently | Membership is derived from relationship cells. A deterministic rule keeps the innermost container, then the earliest-created. |
| Two users type in the same text | Character-wise merge. Formatting merges with Peritext semantics. |
| Master edited while instances are overridden | Independent cells, so normal inheritance applies. |
| Data refresh by two users | The server grants a short **refresh lease** per recordset, so only one client refreshes at a time and the others receive the result. |
| Theme changed by two users | Page theme reference is LWW. |

**Invariant repair.** Some states left by a merge need fixing up, such as an empty group after all its children were deleted remotely. Peers must **not** each write their own repair, because that duplicates ops. Repairs are either:

* *read-time* derivations (preferred), or
* written only by the client whose own transaction caused the break, inside that same transaction.

### 20.6 Engine integration

* The CRDT document is the source of truth. A command opens a transaction, writes CRDT ops and commits, and the engine subscribes to CRDT diff events. **Local and remote changes take the same path:** diff → mark dirty cells and structures → incremental recalc → display-list patch.
* **Live dynamics during gestures:** while dragging, resizing or rotating, the engine computes a **local preview** (full recalc of affected shapes) without writing to the CRDT. The preview is broadcast through presence as a ghost at 20 Hz. On release, one transaction commits the final cells. This keeps updates and history small and makes one gesture one undo step.
* Text editing writes to the rich-text container per keystroke (that's how co-typing works). Pauses of 500 ms or more split undo steps.
* Remote updates arriving mid-gesture are applied to the model and the preview is recomputed on top of them.

### 20.7 Presence (ephemeral, not persisted)

Per peer: `{ userId, name, colour, page, viewport, cursor (page coords), selection: [uuid], textCursor (CRDT cursor), drag ghost, editing: uuid? }`.

* Sent through the CRDT's ephemeral store over the same socket, throttled to 30 Hz for cursors and 20 Hz for ghosts. Peers are removed 15 s after their last heartbeat.
* "Editing" shows a soft badge on the shape ("Ana is editing text"). It is advisory and never a lock.

### 20.8 Sync protocol and server

**Wire protocol** (`flowster-sync-proto`): binary frames over `wss`.

* `Hello{docId, token, clientVersionVector}` → `Welcome{role, serverVersionVector}`.
* Both sides then exchange the updates the other lacks: `SyncUpdates`.
* After that: incremental `Update{bytes}`, `Ack{serverSeq}`, `Ephemeral{bytes}`, `RequestSnapshot` and `Error{code}`.
* Reconnect uses exponential backoff with jitter. The protocol is versioned, and the server rejects clients that are too old with an upgrade prompt.

**Server** (`server/`, Rust):

* One **room** per open document, holding an in-memory Loro doc. Rooms are placed on nodes by consistent hashing on `docId`, which scales horizontally without a shared cache.
* **Validation:** every incoming update is decoded, applied to the room's doc and checked against the schema (types, required fields, size limits, permitted subtrees for the sender's role). Invalid updates are rejected and the client resyncs. Commenters may only touch `comments/*`, and viewers may send presence only.
* **Storage:** an append-only update log (Postgres for the hosted service, SQLite for self-host) plus periodic snapshots. Compaction to shallow snapshots keeps a configurable history window (default 90 days, with named versions kept forever). Assets go into a content-addressed blob store (S3-compatible or filesystem), and clients fetch them lazily.
* **Auth:**
  * OIDC: Microsoft Entra ID, Google, GitHub and generic providers;
  * personal access tokens for the CLI;
  * per-document ACLs and link-sharing policies;
  * org/workspace grouping.
* **Server-side engine:** native `flowster-core` generates thumbnails, version diffs, full-text search indexes and on-demand exports (`GET /docs/{id}/export?format=vsdx|pdf|svg|png`).
* **Deployment:**
  * the Flowster-hosted service;
  * a **self-hosted** single binary or Docker image (SQLite + filesystem by default, Postgres/S3 optional);
  * Helm chart (P1).
* Admin API covers users, documents, retention and audit log (P1).
* Security: TLS only, rate limits, per-document size quotas, and audit events for sharing changes. Optional end-to-end encryption is P2. With E2EE the server cannot validate or index content, so it is opt-in per document.
* **LAN sessions (P2):** a desktop app hosts a temporary room for peers on the same network, discovered via mDNS, with no server.

### 20.9 Shared documents and Visio files

* **Share** a local `.vsdx`, or import a `.vsd` (§19.5) into a new shared document. The import runs on the client, and unknown VSDX content goes into `unknown/*` so that later exports keep it.
* **Download as .vsdx** exports the current state, including preserved unknown parts, so a Visio user can open it.
* Saving a shared document to disk creates a **detached copy**, which is a local document.
* **Update from file** (P1) re-imports a `.vsdx` that someone edited in Visio. Shapes are matched by the UUIDs stored in Flowster's `SolutionXML`, falling back to `UniqueID` and then `legacyId`, and the differences are applied as ordinary CRDT changes. Collaborators see them as one user's edits, which can be undone.

### 20.10 Budgets

| Metric | Target |
|---|---|
| Remote edit visible to peers (same region) | p95 < 200 ms, p99 < 500 ms |
| Apply a typical remote update | < 2 ms in WASM |
| Open a 10,000-shape shared document from snapshot | < 3 s to interactive |
| Update size for a typical drag commit | < 1 KB |
| CRDT memory overhead vs plain model | ≤ 1.5× |
| Concurrency per document | 50 editors + 200 viewers |
| Server room capacity | ≥ 500 active rooms per 4-vCPU node |

### 20.11 Testing

* **Deterministic multi-peer simulation:** N simulated peers apply random command sequences under partitions, reordering, duplication and offline periods. After healing, the CRDT state **and** a hash of the derived state (cell results, geometry, routes, display list) must match on all peers, across WASM and native builds and across Windows, macOS and Linux runners.
* Conflict-table tests: one scripted scenario per row of §20.5.
* Server: fuzz the update validator; permission tests per role; load tests (k6 or a custom Rust harness) at the budgets above.
* End-to-end: two app instances in CI drive the same shared document via WebDriver and assert convergence and presence.

---

## 21. Platform integration (Tauri host)

| Concern | Windows | macOS | Linux |
|---|---|---|---|
| WebView | WebView2 (Evergreen; bootstrapper in installer) | WKWebView (macOS 12+) | WebKitGTK 4.1 (Ubuntu 22.04+, Fedora 39+) |
| Menus | In-window ribbon + window menu | Native app menu (File/Edit/View/Window/Help) mirrored from commands + ribbon | In-window |
| File associations | `.vsdx .vssx .vstx .vsdm .vdx .vsd` (+ "Open with" only by default; opt-in to be default) | UTIs `com.microsoft.visio.*` + document types in Info.plist | `.desktop` MIME types `application/vnd.ms-visio.drawing.main+xml`, etc. |
| Open-file events | single-instance plugin forwards argv | `RunEvent::Opened` URLs | single-instance (D-Bus) |
| Fonts | DirectWrite enumeration via `fontique` | CoreText | fontconfig |
| Printing | Generate PDF in core → system print dialog (Windows: `ShellExecute "print"` or a WebView2 print of a PDF viewer); P2: native print API | `NSPrintOperation` on the PDF via a small Objective-C shim | GTK print dialog on the PDF (`GtkPrintUnixDialog`) |
| Clipboard | multi-format via `clipboard-win` | `NSPasteboard` | X11/Wayland via `arboard` + MIME types |
| Drag & drop | files in/out (out: export SVG/PNG temp file) | same | same |
| Packaging | MSI + NSIS (per-user and per-machine), Authenticode-signed | DMG + notarized `.app`, universal binary | AppImage, `.deb`, `.rpm`, Flatpak (Flathub) |
| Updates | Tauri updater (signed) | Tauri updater (signed) | Tauri updater for AppImage; distro channels otherwise |
| HiDPI | per-monitor v2 | Retina | fractional scaling (GTK) |

WebView caveats to design for:

* WebKitGTK canvas performance and WebGL availability vary by driver. The WebGL backend has to be selectable and auto-detected via a micro-benchmark on first run.
* WKWebView: no `showSaveFilePicker`. All file pickers go through Tauri dialogs. Keyboard shortcuts that conflict with system ones are remapped (see §18.6).
* Use the `asset:` protocol, or binary IPC, for large binary transfers (font bytes, file bytes). Avoid base64 JSON.
* Tauri capabilities: least-privilege per window. The main window may call only Flowster commands. Plugin iframes get no Tauri access.

---

## 22. Quality, testing & conformance

### 22.1 Non-functional requirements

| Metric | Target |
|---|---|
| Cold start to usable window | < 1.5 s (Win/mac), < 2.5 s (Linux) |
| Open 5 MB / 1,000-shape VSDX | < 2 s to first paint, < 4 s fully interactive |
| Memory for 10,000-shape doc | < 800 MB total process set |
| Interaction latency (drag) | < 16 ms frame time p95 on reference hardware |
| Crash-free sessions | ≥ 99.8 % |
| Installer size | ≤ 25 MB (Win), ≤ 30 MB (mac universal), ≤ 35 MB AppImage |
| Round-trip | 100 % of corpus files reopen in Visio without repair prompt |

### 22.2 Test strategy

* **Unit tests** in core: formula parser (property-based with `proptest`), each function, unit conversions, geometry rows, transforms, routing, layout and text layout.
* **Corpus tests:** a curated corpus of ≥ 500 VSDX files (our own plus permissively licensed public samples), checking parse → serialize → parse equality (model-level) and XML-level diff, which tolerates only attribute reordering.
* **Golden rendering:** tiny-skia renders compared to references with perceptual diff (SSIM ≥ 0.98 per page) across the corpus.
* **UI e2e:** Playwright scenarios via WebDriver on all three OSes in CI (GitHub Actions: windows-latest, macos-latest, ubuntu-22.04).
* **Fuzzing:** `cargo-fuzz` on the VSDX reader, the formula parser and DXF import.

### 22.3 The Visio oracle (Windows-only conformance harness)

Microsoft Visio is installed on the development machine where this analysis ran, so we use it as a **black-box oracle** through its public COM automation interface (`Visio.InvisibleApp`, documented API). No reverse engineering is involved.

* `tools/visio-oracle/` (Python + `pywin32`, or PowerShell) runs on a licensed Windows runner. These conformance tests are run only by people who hold a Visio licence.
* **Formula conformance:** for generated test cases (expression, context shape, units), set `Cell.FormulaU` in Visio and record `ResultIU`/`ResultStr`. Store `(formula, context, expected)` fixtures in the repo. The core must match within 1e-9 (numbers) or exactly (strings).
* **Transform and geometry conformance:** export Visio's computed `Paths`/`BoundingBox`/`VisualBoundingBox` for corpus shapes. Compare to core.
* **Rendering references:** export PNG at 192 DPI from Visio for corpus pages as golden images. Store only for our own corpus files.
* **Round-trip acceptance:** open each Flowster-saved file in Visio via COM, check that no repair dialog appears (`AlertResponse`), then re-save and diff.
* Fixtures are committed. CI on other OSes consumes fixtures only.

---

## 23. Security & privacy

* **No code execution from documents:** VBA is never run. `RUNADDON`/`CALLTHIS` resolve only to installed plugins. `DOCMD` resolves only to safe built-in commands.
* **External links:** hyperlinks to non-http(s) or file targets need confirmation, with an option to allow them for the session (mirrors the "External App links" prompt found in Visio strings). `file:` links open only after confirmation.
* XML parsing has no DTD or external entities. ZIP has bomb limits (part-size and ratio caps) and path-traversal checks on part names.
* Images are decoded with memory-safe Rust decoders, with size limits.
* Plugins run sandboxed and are permission-gated (§17). Plugin network access is limited to declared origins.
* Telemetry is **off by default**, opt-in with anonymous usage counts only and no document content. Crash reports are opt-in and scrubbed of file paths.
* "Remove hidden information" (§18.3) strips author, comments' authors, preview, unused masters and styles, data recordsets and validation rules.
* **Shared documents:**
  * the server enforces roles on every update (§20.8);
  * documents are encrypted at rest on the hosted service;
  * sharing changes are audited;
  * links can expire and can be revoked;
  * exporting a shared document applies "Remove hidden information" by default for viewers.
* The `.vsd` importer (`flowster-vsd`) is fuzzed and bounded: decompression ratio and size caps, a limit on pointer-tree depth, and no recursion on untrusted offsets.

---

## 24. Milestones

Indicative, assuming 4 core engineers, 2 front-end engineers, **2 sync/server engineers, 1 legacy-format engineer**, 1 designer and 1 content designer. The main line (M0–M6) runs alongside two tracks.

| Milestone | Duration | Exit criteria |
|---|---|---|
| **M0 Foundations** | 5 wks | Repo, CI on 3 OSes, Tauri shell, worker + WASM core skeleton, command bus, oracle harness skeleton. **CRDT spike** (Loro vs `yrs` on the §20.3 schema with 10k shapes: memory, load time, update size) → ADR. **Determinism harness** (derived-state hash across WASM/native/OSes). [MS-VGSFF] and libvisio/POI evaluation for `.vsd` → ADR. |
| **M1 Viewer** | 10 wks | VSDX read **into the CRDT model**, full model + inheritance, formula engine Tier A, all geometry rows, line/fill/text rendering, themes (`THEMEVAL`), pages/backgrounds/layers display, zoom/pan, PDF/PNG/SVG export, CLI `convert/export`. ≥ 90 % corpus pages at SSIM ≥ 0.95. |
| **M2 Editor core** | 12 wks | Selection, move/resize/rotate, tools (Ctrl+1–9), text editing (rich-text CRDT), per-user undo/redo, snap/glue, stencils pane with own P0 content, drag-drop masters, control handles, actions, Shape Data, Size & Position, VSDX write with lossless round-trip (oracle-verified), autosave. **Public alpha (local documents).** |
| **M3 Connectors & arrange** | 8 wks | Dynamic connector routing (deterministic), line jumps, AutoConnect, align/distribute/auto-space, groups, z-order, page setup, drawing scale, rulers/grid/guides, print. |
| **M4 Structure, layout, themes UI** | 8 wks | Containers, lists, CFF, callouts, re-layout algorithms, themes/variants/quick styles UI, styles, layers pane, Format Shape pane, Tier B functions. **Public beta: P0 complete, including co-editing and `.vsd` T1 import.** |
| **M5 Data, validation, review** | 8 wks | External data + data graphics (with refresh leases), validation + issues pane, accessibility, find/replace/spell, ShapeSheet window, VDX import, DXF, HTML export. |
| **M6 Extensibility & 1.0** | 8 wks | Plugin API + ribbon/pane contributions, P1 content set, localization (9 languages), performance (WebGL backend), installers, signing, updater. **v1.0.** |

| Parallel track | Starts | Deliverables |
|---|---|---|
| **S — Collaboration** | after M0 | S1 (with M1–M2): sync protocol, self-hosted server (SQLite), rooms, validation, offline queue, presence (cursors, selections). S2 (with M3): sharing UI, roles, OIDC, comments in real time, ghost drags, follow mode. S3 (with M4): version history and named versions, Postgres/S3 hosted deployment, load tests at §20.10 budgets → **co-editing in public beta**. S4 (with M5–M6): visual version diff, "Update from file", admin API, Helm chart. |
| **L — Legacy `.vsd`** | after M0 | L1 (with M1–M2): container, pointers, decompression, chunk parsing; T1 import of shapes, masters, styles, text and values for v11 files; oracle pair corpus. L2 (with M3–M4): T1 completeness (data, layers, connects, foreign images), import report, batch CLI → **`.vsd` import in public beta**. L3 (with M5–M6): T2 formula decoding, v6 (Visio 2000/2002) files. |

---

## 25. Risks & open questions

| # | Risk / question | Mitigation |
|---|---|---|
| R1 | ShapeSheet semantic edge cases (cycles, GUARD/SETATREF interplay, inheritance quirks) are under-documented. | Oracle-driven conformance fixtures (§22.3). Treat the oracle as ground truth. |
| R2 | Text metrics differ from Visio (PTLS vs our stack), so `TEXTHEIGHT`-driven shapes size differently. | Metric-compatible fallback fonts, oracle comparisons, tolerance in layout-driven formulas, and preserving `V` values when recalc isn't triggered. |
| R3 | WebKitGTK performance. | WebGL backend, LOD, worker offload, auto-detect. |
| R4 | Legal: interoperability with proprietary format; trade dress. | Clean-room rules (§0), public specs, own content and design, legal review before beta. |
| R5 | Third-party icon licensing (AWS/Azure/GCP). | Legal review, plus a user-driven importer as fallback. |
| R6 | No complete public spec for binary `.vsd`; formula token decoding (T2) is the hardest part. | MPL-2.0 importer crate that may port from libvisio; [MS-VGSFF] evaluation; oracle `.vsd`→`.vsdx` ground-truth pairs; T1 (values) ships first so files always open. |
| R7 | WASM memory ceiling (4 GB, practical ~2 GB) for huge documents, now including CRDT overhead. | Lazy master loading, page-level unloading of display lists, row-level CRDT granularity, shallow snapshots, memory64 when available across WebViews. |
| R8 | Derived state diverges between peers (non-determinism in floats, fonts, routing, layout). | §20.3 determinism rules, the derived-state hash check in the multi-peer simulation on every CI run, and a deterministic font policy for shared documents. |
| R9 | CRDT library maturity or performance at 10k+ shapes. | M0 spike with the real schema; `yrs` as fallback; the CRDT is isolated behind `flowster-crdt` so the library can be swapped. |
| R10 | Running a hosted sync service (cost, uptime, data protection: GDPR, data residency). | Self-hosted option from day one; regional storage for the hosted service; retention controls; E2EE later. |
| Q1 | ~~Ship real-time collaboration in 1.x or 2.0?~~ **Decided: v1.0, P0** (§20). Hosted service, self-hosted, or both at launch? The spec assumes both. | |
| Q2 | Commercial model (open source core + paid hosting/content/plugins?) affects the content licence and whether the hosted service is the main revenue line. `flowster-vsd` is MPL-2.0 either way. | |
| Q4 | Which identity providers and enterprise features (SSO enforcement, SCIM, audit export) are needed at launch? | |
| Q3 | Do we need EMF export on macOS/Linux for Office interop, or are SVG/PDF enough? | |

---

## Appendix A. Reverse-engineering artefacts (`docs/re/`)

| File | Contents |
|---|---|
| `object-model.txt` | All co-classes and dispatch interfaces with member lists from the Visio type library. |
| `enums.txt` | All 167 enums with members (`VisUICmds` = 714 commands, `VisCellIndices` = 495, `VisEventCodes` = 112, `VisLayoutAlgorithm` = 28, …). |
| `keyboard-shortcuts.txt` | Default drawing-window accelerators mapped to command names. |
| `shapesheet-identifiers.txt` | Cell/section/row identifiers and ShapeSheet UI section names. |
| `shapesheet-usage.txt` | Usage frequency of sections, row types, functions and cells in shipped stencils. |
| `stencil-catalog.tsv` | 363 stencils × master counts and sample master names (sizing only). |
| `template-catalog.txt` | 228 template names (sizing only). |

## Appendix B. References (public)

* [MS-VSDX]: Visio Graphics Service VSDX File Format (Microsoft Open Specifications).
* Visio ShapeSheet reference (cells, sections, functions): Microsoft Learn, "Visio ShapeSheet reference".
* Visio VBA/Object model reference: Microsoft Learn, "Visio object model reference" (for API naming parity).
* ECMA-376 Part 2 (Open Packaging Conventions) and Part 1 DrawingML (themes).
* Visio 2003–2010 XML (VDX) schema reference.
* libvisio (Document Liberation Project, MPL-2.0): legacy `.vsd` parsing.
* Apache POI HDGF (Apache-2.0): Visio binary stream and chunk structure.
* [MS-VGSFF]: Visio Graphics Service File Format (`.vdw`), Microsoft Open Specifications. Evaluate it for coverage of the shared binary structure.
* Loro CRDT documentation (loro.dev): movable tree, rich text, `UndoManager`, shallow snapshots, ephemeral store.
* Kleppmann et al., "A highly-available move operation for replicated trees" (2021), and Litt et al., "Peritext: A CRDT for collaborative rich text editing" (2022).
