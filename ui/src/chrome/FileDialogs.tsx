// File dialogs: the unsaved-changes question, Export, and the recovered-drawing notice.

import { useState } from "react";

import { Icon } from "./icons";
import { Dialog } from "./Welcome";

/** "Save changes to X?" before the drawing is closed, replaced or reopened. */
export function UnsavedDialog({
  title,
  onSave,
  onDiscard,
  onCancel,
}: {
  title: string;
  onSave: () => void;
  onDiscard: () => void;
  onCancel: () => void;
}) {
  return (
    <Dialog title="Unsaved changes" onClose={onCancel}>
      <p className="dialog-text">
        Do you want to save the changes to <strong>{title}</strong>?
      </p>
      <p className="dialog-text muted">If you don't, they are lost.</p>
      <div className="dialog-actions">
        <button type="button" className="btn" onClick={onDiscard}>
          Don't save
        </button>
        <span className="spacer" />
        <button type="button" className="btn" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="btn primary" autoFocus onClick={onSave}>
          Save
        </button>
      </div>
    </Dialog>
  );
}

export type ExportFormat = "pdf" | "png" | "svg";

export interface ExportChoice {
  format: ExportFormat;
  /** PDF: every visible page, or just the current one. */
  allPages: boolean;
  /** PNG: dots per inch. */
  dpi: number;
}

const RESOLUTIONS: [number, string][] = [
  [96, "Screen (96 dpi)"],
  [150, "Document (150 dpi)"],
  [300, "Print (300 dpi)"],
];

export function ExportDialog({
  pdf,
  pageName,
  pageCount,
  onExport,
  onClose,
}: {
  /** PDF can be made here (the desktop app); a browser prints to PDF instead. */
  pdf: boolean;
  pageName: string;
  /** Visible foreground pages. */
  pageCount: number;
  onExport: (choice: ExportChoice) => void;
  onClose: () => void;
}) {
  const [format, setFormat] = useState<ExportFormat>(pdf ? "pdf" : "png");
  const [allPages, setAllPages] = useState(pageCount > 1);
  const [dpi, setDpi] = useState(150);
  const formats: [ExportFormat, string, string][] = [
    ["pdf", "PDF", pdf ? "Vector, with the fonts embedded; one page or all of them" : "Use Print and choose Save as PDF"],
    ["png", "PNG", "A picture of this page"],
    ["svg", "SVG", "Vector picture of this page, for the web and other editors"],
  ];
  return (
    <Dialog title="Export" onClose={onClose}>
      <fieldset className="choice-list">
        <legend>Format</legend>
        {formats.map(([f, label, hint]) => (
          <label key={f} className={`choice${f === "pdf" && !pdf ? " disabled" : ""}`}>
            <input
              type="radio"
              name="format"
              checked={format === f}
              disabled={f === "pdf" && !pdf}
              onChange={() => setFormat(f)}
            />
            <span>
              <strong>{label}</strong>
              <span className="muted">{hint}</span>
            </span>
          </label>
        ))}
      </fieldset>
      {format === "pdf" && pageCount > 1 && (
        <fieldset className="choice-list">
          <legend>Pages</legend>
          <label className="choice">
            <input type="radio" name="pages" checked={allPages} onChange={() => setAllPages(true)} />
            <span>All {pageCount} pages</span>
          </label>
          <label className="choice">
            <input type="radio" name="pages" checked={!allPages} onChange={() => setAllPages(false)} />
            <span>This page ({pageName})</span>
          </label>
        </fieldset>
      )}
      {format === "png" && (
        <fieldset className="choice-list">
          <legend>Resolution</legend>
          {RESOLUTIONS.map(([d, label]) => (
            <label key={d} className="choice">
              <input type="radio" name="dpi" checked={dpi === d} onChange={() => setDpi(d)} />
              <span>{label}</span>
            </label>
          ))}
        </fieldset>
      )}
      <div className="dialog-actions">
        <span className="spacer" />
        <button type="button" className="btn" onClick={onClose}>
          Cancel
        </button>
        <button type="button" className="btn primary" autoFocus onClick={() => onExport({ format, allPages, dpi })}>
          Export…
        </button>
      </div>
    </Dialog>
  );
}

/** On the start screen: a drawing with unsaved changes that was lost to a crash or a forced close. */
export function RecoveryNotice({
  title,
  savedAt,
  onRecover,
  onDiscard,
}: {
  title: string;
  savedAt: number;
  onRecover: () => void;
  onDiscard: () => void;
}) {
  const when = new Date(savedAt).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
  return (
    <div className="recovery" role="status">
      <Icon name="info" size={18} />
      <div className="recovery-text">
        <strong>Unsaved changes to “{title}” can be recovered</strong>
        <span>Flowster closed before they were saved. Last kept {when}.</span>
      </div>
      <button type="button" className="btn" onClick={onDiscard}>
        Discard
      </button>
      <button type="button" className="btn primary" onClick={onRecover}>
        Recover
      </button>
    </div>
  );
}
