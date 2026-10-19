// The start screen and the Help dialogs.

import { useEffect } from "react";

import { Icon, shortcut } from "./icons";

export function Welcome({ onNew, onOpen, busy }: { onNew: () => void; onOpen: () => void; busy: boolean }) {
  return (
    <div className="welcome">
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
  ["Pan", "Space+drag, middle button or wheel"],
  ["Cancel / deselect", "Esc"],
];

export function Dialog({ title, onClose, children }: { title: string; onClose: () => void; children: React.ReactNode }) {
  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose]);
  return (
    <div className="scrim" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog" role="dialog" aria-modal="true" aria-label={title}>
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
    <Dialog title="Keyboard shortcuts" onClose={onClose}>
      <table className="keys-table">
        <tbody>
          {SHORTCUTS.map(([what, keys]) => (
            <tr key={what}>
              <td>{what}</td>
              <td>
                <kbd>{shortcut(keys)}</kbd>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </Dialog>
  );
}

export function AboutDialog({ onClose }: { onClose: () => void }) {
  return (
    <Dialog title="About Flowster" onClose={onClose}>
      <div className="about">
        <div className="welcome-mark small">
          <Icon name="logo" size={26} />
        </div>
        <p>
          <strong>Flowster</strong> is an open diagramming editor that reads and writes Visio .vsdx files.
        </p>
        <p className="muted">Drawings are kept in a collaborative document model, so every change can be undone.</p>
      </div>
    </Dialog>
  );
}
