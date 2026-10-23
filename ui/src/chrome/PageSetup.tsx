// Page setup, as on Visio's Design tab: Orientation, Size (standard paper,
// Fit to Drawing, a custom size) and Auto Size.

import { useState } from "react";

import type { Command, PageInfo } from "../engine/protocol";
import { Dialog } from "./Welcome";

export interface Paper {
  /** PaperKind, as the engine stores it. */
  kind: number;
  name: string;
  /** Portrait size in inches. */
  w: number;
  h: number;
}

/** The sizes offered. Keep in step with `flowster_core::paper::PAPER`. */
export const PAPER: Paper[] = [
  { kind: 1, name: "Letter", w: 8.5, h: 11 },
  { kind: 5, name: "Legal", w: 8.5, h: 14 },
  { kind: 3, name: "Tabloid", w: 11, h: 17 },
  { kind: 11, name: "A5", w: 5.83, h: 8.27 },
  { kind: 9, name: "A4", w: 8.27, h: 11.69 },
  { kind: 8, name: "A3", w: 11.69, h: 16.54 },
  { kind: 66, name: "A2", w: 16.54, h: 23.39 },
];

export const isLandscape = (p: PageInfo) => p.width > p.height;

const near = (a: number, b: number) => Math.abs(a - b) < 0.01;

/** The standard size the page is, in either orientation, or null for any other size. */
export function paperOf(p: PageInfo): Paper | null {
  return (
    PAPER.find((s) => (near(s.w, p.width) && near(s.h, p.height)) || (near(s.h, p.width) && near(s.w, p.height))) ?? null
  );
}

/** Sets a standard size, keeping the page's orientation. */
export function paperCommand(p: PageInfo, paper: Paper): Command {
  const [width, height] = isLandscape(p) ? [paper.h, paper.w] : [paper.w, paper.h];
  return { kind: "setPageSize", page: p.index, width, height, paper: paper.kind };
}

export const sizeLabel = (w: number, h: number) => `${+w.toFixed(2)} × ${+h.toFixed(2)} in`;

/** Design > Size > Custom size: any width and height (this turns Auto Size off). */
export function PageSizeDialog({
  page,
  onApply,
  onClose,
}: {
  page: PageInfo;
  onApply: (cmd: Command) => void;
  onClose: () => void;
}) {
  const [width, setWidth] = useState(String(+page.width.toFixed(3)));
  const [height, setHeight] = useState(String(+page.height.toFixed(3)));
  const w = Number.parseFloat(width);
  const h = Number.parseFloat(height);
  const valid = [w, h].every((v) => Number.isFinite(v) && v >= 0.25 && v <= 2000);
  const apply = () => {
    if (!valid) return;
    onApply({ kind: "setPageSize", page: page.index, width: w, height: h });
    onClose();
  };
  return (
    <Dialog title="Page size" onClose={onClose}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          apply();
        }}
      >
        <div className="field-grid">
          {(
            [
              ["Width", width, setWidth],
              ["Height", height, setHeight],
            ] as const
          ).map(([label, value, set], i) => (
            <label className="field" key={label}>
              <span className="field-label">{label}</span>
              <span className="field-box">
                <input inputMode="decimal" value={value} autoFocus={i === 0} onChange={(e) => set(e.target.value)} />
                <span className="field-unit">in</span>
              </span>
            </label>
          ))}
        </div>
        {!valid ? (
          <p className="dialog-text error-text">Each side is 0.25 to 2000 inches.</p>
        ) : (
          page.autoSize && <p className="dialog-text muted">A custom size turns Auto Size off.</p>
        )}
        <div className="dialog-actions">
          <span className="spacer" />
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn primary" disabled={!valid}>
            Apply
          </button>
        </div>
      </form>
    </Dialog>
  );
}
