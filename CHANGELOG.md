# Changelog

Notable changes to Flowster, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

Add each change under **Unreleased** as it lands. When you `make bump`, rename that heading to the new version and date. The user-facing notes for a release go in `release-notes/<version>.md`, which the download page and the update dialog show.

## Unreleased

### Added

- Help ▸ About ▸ **Third-party licences** lists every open-source component Flowster ships, with its licence text. `scripts/third-party-notices.mjs` generates the list during each installer build.

### Changed

- The spec (`docs/SPEC.md`) now draws on public Microsoft documentation and the Visio oracle only. It no longer describes analysing Visio's program files.
- The Visio oracle runs only on a licensed developer machine, never in CI. CI uses the committed fixtures.

## 0.1.0 — 2026-10-21

The first preview. See [release-notes/0.1.0.md](release-notes/0.1.0.md) for the user-facing notes.

### Added

- **Visio files:** opens `.vsdx`, `.vssx` and `.vstx`, including macro-enabled ones, and saves `.vsdx` that Visio opens.
- **Rendering:** a ShapeSheet formula engine; gradient fills, fill patterns and shadows; arrowheads 1–45; EMF pictures drawn as vectors; text layout.
- **Editing:** drawings live in a collaborative (CRDT) document model, so every change can be undone. Shapes from the Basic, Flowchart and Swimlane stencils, text, and connectors that stay glued.
- **Arranging:** pages, grid and rulers, smart guides, align and distribute, group, z-order, clipboard, and a command palette (Ctrl+K).
- **Saving and output:** safe saving with AutoRecover, PDF (fonts embedded), PNG and SVG export, and printing.
- **Releases:** signed installers for Windows, macOS and Linux, a download site, and Help ▸ Check for updates.
