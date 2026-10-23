// The start screen and the Help dialogs.

import { useEffect, useState } from "react";

import { Icon, shortcut } from "./icons";

export function Welcome({
  onNew,
  onOpen,
  busy,
  notice,
}: {
  onNew: () => void;
  onOpen: () => void;
  busy: boolean;
  /** Shown above the card (a recoverable drawing). */
  notice?: React.ReactNode;
}) {
  return (
    <div className="welcome">
      {notice}
      <div className="welcome-card">
        <div className="welcome-mark">
          <Icon name="logo" size={34} />
        </div>
        <h1>Start a drawing</h1>
        <p className="welcome-sub">Diagrams that open in Visio, made anywhere.</p>
        <div className="welcome-actions">
          <button type="button" className="tile" onClick={onNew}>
            <span className="tile-icon">
              <Icon name="new" size={22} />
            </span>
            <span className="tile-text">
              <strong>Blank drawing</strong>
              <span>Letter, landscape</span>
            </span>
            <kbd>{shortcut("Ctrl+N")}</kbd>
          </button>
          <button type="button" className="tile" onClick={onOpen} disabled={busy}>
            <span className="tile-icon">
              <Icon name="open" size={22} />
            </span>
            <span className="tile-text">
              <strong>{busy ? "Opening…" : "Open a file"}</strong>
              <span>.vsdx drawings, stencils and templates</span>
            </span>
            <kbd>{shortcut("Ctrl+O")}</kbd>
          </button>
        </div>
        <p className="welcome-hint">You can also drop a file anywhere in this window.</p>
      </div>
    </div>
  );
}

export const SHORTCUTS: [string, string][] = [
  ["New drawing", "Ctrl+N"],
  ["Open", "Ctrl+O"],
  ["Save", "Ctrl+S"],
  ["Undo", "Ctrl+Z"],
  ["Redo", "Ctrl+Y"],
  ["Select all", "Ctrl+A"],
  ["Delete", "Del"],
  ["Edit text", "F2"],
  ["Pointer tool", "Ctrl+1"],
  ["Rectangle tool", "Ctrl+8"],
  ["Ellipse tool", "Ctrl+9"],
  ["Line tool", "Ctrl+6"],
  ["Zoom in / out", "Ctrl++ / Ctrl+-"],
  ["Fit page", "Ctrl+Shift+W"],
  ["Next / previous page", "Ctrl+PgDn / Ctrl+PgUp"],
  ["Command palette", "Ctrl+K"],
  ["Cut / copy / paste", "Ctrl+X / Ctrl+C / Ctrl+V"],
  ["Duplicate (or Alt+drag)", "Ctrl+D"],
  ["Copy / paste style", "Ctrl+Alt+C / Ctrl+Alt+V"],
  ["Nudge (Shift: further)", "Arrow keys"],
  ["Fit page / zoom to selection", "Shift+1 / Shift+2"],
  ["Move without snapping", "Hold Ctrl"],
  ["Keep proportions, lock axis, 15° steps", "Hold Shift"],
  ["Group / ungroup", "Ctrl+G / Ctrl+Shift+U"],
  ["Bring to front / send to back", "Ctrl+Shift+F / Ctrl+Shift+B"],
  ["Rotate left / right 90°", "Ctrl+L / Ctrl+R"],
  ["Flip horizontal / vertical", "Ctrl+H / Ctrl+J"],
  ["Pan", "Space+drag, middle button or wheel"],
  ["Cancel / deselect", "Esc"],
];

export function Dialog({
  title,
  onClose,
  wide,
  children,
}: {
  title: string;
  onClose: () => void;
  /** Landscape, for content laid out in columns. */
  wide?: boolean;
  children: React.ReactNode;
}) {
  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose]);
  return (
    <div className="scrim" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={wide ? "dialog wide" : "dialog"} role="dialog" aria-modal="true" aria-label={title}>
        <header>
          <h2>{title}</h2>
          <button type="button" className="icon-btn" aria-label="Close" onClick={onClose}>
            <Icon name="close" size={16} />
          </button>
        </header>
        <div className="dialog-body">{children}</div>
      </div>
    </div>
  );
}

export function ShortcutsDialog({ onClose }: { onClose: () => void }) {
  return (
    <Dialog title="Keyboard shortcuts" onClose={onClose} wide>
      {/* In columns, so the whole list fits without scrolling. */}
      <dl className="keys-grid">
        {SHORTCUTS.map(([what, keys]) => (
          <div className="keys-row" key={what}>
            <dt>{what}</dt>
            <dd>
              <kbd>{shortcut(keys)}</kbd>
            </dd>
          </div>
        ))}
      </dl>
    </Dialog>
  );
}

export function AboutDialog({ onClose }: { onClose: () => void }) {
  const [notices, setNotices] = useState<string | null>(null);
  // scripts/third-party-notices.mjs writes the file next to the built UI (not in dev builds, where
  // the dev server answers with index.html instead).
  const showNotices = () =>
    void fetch("/third-party-notices.txt")
      .then((r) => (r.ok && r.headers.get("content-type")?.startsWith("text/plain") ? r.text() : Promise.reject()))
      .then(setNotices, () => setNotices("The licence notices are added to release builds."));
  return (
    <Dialog title="About Flowster" onClose={onClose}>
      <div className="about">
        <div className="welcome-mark small">
          <Icon name="logo" size={26} />
        </div>
        <p>
          <strong>Flowster</strong> is an open diagramming editor that reads and writes Visio .vsdx files.
        </p>
        <p className="muted">Version {__APP_VERSION__}</p>
        <p className="muted">Drawings are kept in a collaborative document model, so every change can be undone.</p>
        {notices === null ? (
          <button type="button" className="btn small" onClick={showNotices}>
            Third-party licences
          </button>
        ) : (
          <pre className="notices">{notices}</pre>
        )}
      </div>
    </Dialog>
  );
}
