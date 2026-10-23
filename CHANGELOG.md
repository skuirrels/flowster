# Changelog

Notable changes to Flowster, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

Add each change under **Unreleased** as it lands. When you `make bump`, rename that heading to the new version and date. The user-facing notes for a release go in `release-notes/<version>.md`, which the download page and the update dialog show.

## Unreleased

## 0.1.6 — 2026-10-23

See [release-notes/0.1.6.md](release-notes/0.1.6.md) for the user-facing notes.

### Changed

- Help › Keyboard shortcuts opens a wide dialog with the shortcuts in three columns (fewer on a narrow window), so the list fits without scrolling.

### Fixed

- A new or opened drawing now opens fitted to the window. It used to keep the previous drawing's zoom and scroll, because the canvas only refitted when the page index changed, and every drawing starts on page 1.

## 0.1.5 — 2026-10-23

See [release-notes/0.1.5.md](release-notes/0.1.5.md) for the user-facing notes.

### Added

- Page setup, as on Visio's Design tab, in a new Design menu and in the Inspector when nothing is selected: portrait or landscape, standard paper sizes (Letter, Legal, Tabloid, A5, A4, A3, A2), a custom size, and Fit to Drawing.
- Auto Size: the page stays a whole number of sheets of its paper around the shapes, growing and shrinking after each edit in the same undo step. Shapes past the left or bottom edge add sheets there, and the view pans so nothing jumps.
- Files opened from outside Flowster (Finder, Explorer's Open With, the command line) now open in it, asking about unsaved changes first.
- The installers register Flowster for `.vsdx`, `.vsdm`, `.vssx`, `.vssm`, `.vstx` and `.vstm` files.
- File › Settings… (desktop app) with an Open with Flowster section: on macOS it makes Flowster the default app for the chosen file types; on Windows it opens Default apps.

### Changed

- New drawings start with Auto Size on, on Letter landscape paper, as Visio's do.

## 0.1.4 — 2026-10-23

See [release-notes/0.1.4.md](release-notes/0.1.4.md) for the user-facing notes.

### Added

- While a file opens, the window shows "Opening <file name>…" with a spinner instead of appearing to do nothing.

### Changed

- Large drawings open about twice as fast: a 34-page drawing with 77,000 shapes now opens in under 5 seconds instead of about 9. The undo history now starts after the drawing is loaded, so the load no longer records a change event for every shape.

### Fixed

- Text on the canvas stays lined up with its shapes after the first redraw. The selection overlay left the canvas set to centre text, so from the second frame on every word was drawn centred on its own position: words in table cells overlapped or spread apart, centred labels slid left out of their boxes, and rotated swimlane labels moved along their bands and were cut off at the edge.
- Centred and right-aligned text in SVG exports is anchored by its alignment, so it stays centred or right-aligned in the viewer's font instead of being placed by an estimated width.

## 0.1.3 — 2026-10-23

See [release-notes/0.1.3.md](release-notes/0.1.3.md) for the user-facing notes.

### Added

- Holding Space over the canvas shows a grab hand, and the hand closes while you drag to pan. In a text field or while editing a shape's text, Space types a space as before.

### Fixed

- The page tabs at the bottom of the window no longer show scrollbars that clipped the tab names. The tabs scroll with the mouse wheel or trackpad, the current page's tab stays in view, the insert-page button stays pinned beside them, and the status items on the right are no longer cut off.

## 0.1.2 — 2026-10-23

### Fixed

- The macOS app is ad-hoc signed, so Gatekeeper no longer reports the download as damaged.

## 0.1.1 — 2026-10-22

### Added

- Help ▸ About ▸ **Third-party licences** lists every open-source component Flowster ships, with its licence text. `scripts/third-party-notices.mjs` generates the list during each installer build.

### Changed

- The spec (`docs/SPEC.md`) now draws on public Microsoft documentation and the Visio oracle only. It no longer describes analysing Visio's program files.
- The Visio oracle runs only on a licensed developer machine, never in CI. CI uses the committed fixtures.

### Fixed

- Drawings with several themes take each page's or shape's own theme, so text that came out white on white (missing headings, labels and table rows) now shows in its proper colours.
- Paragraph bullets are drawn on screen and in PDF, PNG and SVG export, and lines may break after a slash or a dash, as in Visio.

## 0.1.0 — 2026-10-21

The first preview. See [release-notes/0.1.0.md](release-notes/0.1.0.md) for the user-facing notes.

### Added

- **Visio files:** opens `.vsdx`, `.vssx` and `.vstx`, including macro-enabled ones, and saves `.vsdx` that Visio opens.
- **Rendering:** a ShapeSheet formula engine; gradient fills, fill patterns and shadows; arrowheads 1–45; EMF pictures drawn as vectors; text layout.
- **Editing:** drawings live in a collaborative (CRDT) document model, so every change can be undone. Shapes from the Basic, Flowchart and Swimlane stencils, text, and connectors that stay glued.
- **Arranging:** pages, grid and rulers, smart guides, align and distribute, group, z-order, clipboard, and a command palette (Ctrl+K).
- **Saving and output:** safe saving with AutoRecover, PDF (fonts embedded), PNG and SVG export, and printing.
- **Releases:** signed installers for Windows, macOS and Linux, a download site, and Help ▸ Check for updates.
