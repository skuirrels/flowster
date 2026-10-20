// Formatting for the selection: fill, line, font and paragraph controls that
// write ShapeSheet cells through one `setCells` command per change.

import { useEffect, useRef, useState } from "react";

import type { CellEdit, Rgba, ShapeFormat } from "../engine/protocol";
import { Icon, type IconName } from "./icons";

export const FONTS = [
  "Calibri",
  "Arial",
  "Segoe UI",
  "Verdana",
  "Tahoma",
  "Trebuchet MS",
  "Georgia",
  "Times New Roman",
  "Cambria",
  "Garamond",
  "Consolas",
  "Courier New",
];
export const SIZES = [6, 8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 32, 36, 48, 60, 72];
export const WEIGHTS = [0.25, 0.5, 0.75, 1, 1.5, 2.25, 3, 4.5, 6];
/** Visio line patterns offered: solid, dash, dot, dash-dot, long dash. */
export const PATTERNS: [number, string][] = [
  [1, "Solid"],
  [2, "Dash"],
  [3, "Dot"],
  [4, "Dash dot"],
  [9, "Short dash"],
  [16, "Long dash"],
];

/** Theme columns (Office-like) with tints and shades, then standard colours. */
const THEME = ["#ffffff", "#000000", "#e7e6e6", "#44546a", "#4472c4", "#ed7d31", "#a5a5a5", "#ffc000", "#5b9bd5", "#70ad47"];
const STANDARD = ["#c00000", "#ff0000", "#ffc000", "#ffff00", "#92d050", "#00b050", "#00b0f0", "#0070c0", "#002060", "#7030a0"];

function mix(hex: string, toward: number, amount: number): string {
  const n = Number.parseInt(hex.slice(1), 16);
  const c = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => Math.round(v + (toward - v) * amount));
  return `#${c.map((v) => v.toString(16).padStart(2, "0")).join("")}`;
}

export const hex = (c: Rgba | null | undefined) =>
  c ? `#${c.slice(0, 3).map((v) => v.toString(16).padStart(2, "0")).join("")}` : null;

/** A colour button with a swatch strip and a palette popover. */
export function ColorPicker({
  icon,
  label,
  value,
  onPick,
  allowNone,
  noneLabel = "No colour",
}: {
  icon: IconName;
  label: string;
  value: string | null;
  onPick: (color: string | null) => void;
  allowNone?: boolean;
  noneLabel?: string;
}) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => !box.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("pointerdown", away);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("pointerdown", away);
      window.removeEventListener("keydown", esc);
    };
  }, [open]);
  const pick = (c: string | null) => {
    setOpen(false);
    onPick(c);
  };
  return (
    <div className="picker" ref={box}>
      <button
        type="button"
        className="tb color-btn"
        aria-label={label}
        data-tip={label}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <Icon name={icon} size={17} />
        <span className="swatch-strip" style={{ background: value ?? "transparent" }} data-none={value === null} />
      </button>
      {open && (
        <div className="palette" role="dialog" aria-label={label}>
          <div className="palette-title">Theme colours</div>
          <div className="swatches">
            {THEME.map((c) => (
              <button type="button" key={c} className="sw" style={{ background: c }} title={c} onClick={() => pick(c)} />
            ))}
            {[0.8, 0.6, 0.4].flatMap((t) =>
              THEME.map((c) => {
                const m = mix(c, 255, t);
                return <button type="button" key={c + t} className="sw" style={{ background: m }} title={m} onClick={() => pick(m)} />;
              }),
            )}
            {[0.25, 0.5].flatMap((t) =>
              THEME.map((c) => {
                const m = mix(c, 0, t);
                return <button type="button" key={c + "d" + t} className="sw" style={{ background: m }} title={m} onClick={() => pick(m)} />;
              }),
            )}
          </div>
          <div className="palette-title">Standard colours</div>
          <div className="swatches">
            {STANDARD.map((c) => (
              <button type="button" key={c} className="sw" style={{ background: c }} title={c} onClick={() => pick(c)} />
            ))}
          </div>
          <div className="palette-actions">
            {allowNone && (
              <button type="button" className="btn small" onClick={() => pick(null)}>
                <Icon name="noColor" size={15} /> {noneLabel}
              </button>
            )}
            <label className="btn small custom">
              More colours…
              <input type="color" value={value ?? "#000000"} onChange={(e) => pick(e.target.value)} />
            </label>
          </div>
        </div>
      )}
    </div>
  );
}

const style = (f: ShapeFormat, patch: Partial<Pick<ShapeFormat, "bold" | "italic" | "underline">>) => {
  const g = { ...f, ...patch };
  return String((g.bold ? 1 : 0) | (g.italic ? 2 : 0) | (g.underline ? 4 : 0));
};

/** Cell edits for the common formatting actions. */
export const fmt = {
  fill: (c: string | null): CellEdit[] =>
    c === null ? [{ cell: "FillPattern", value: "0" }] : [{ cell: "FillForegnd", value: c }, { cell: "FillPattern", value: "1" }],
  line: (c: string | null): CellEdit[] =>
    c === null ? [{ cell: "LinePattern", value: "0" }] : [{ cell: "LineColor", value: c }, { cell: "LinePattern", value: "1" }],
  weight: (pt: number): CellEdit[] => [{ cell: "LineWeight", value: String(pt / 72) }],
  pattern: (p: number): CellEdit[] => [{ cell: "LinePattern", value: String(p) }],
  font: (name: string): CellEdit[] => [{ section: "Character", cell: "Font", value: name }],
  size: (pt: number): CellEdit[] => [{ section: "Character", cell: "Size", value: String(pt / 72) }],
  textColor: (c: string): CellEdit[] => [{ section: "Character", cell: "Color", value: c }],
  style: (f: ShapeFormat, patch: Partial<Pick<ShapeFormat, "bold" | "italic" | "underline">>): CellEdit[] => [
    { section: "Character", cell: "Style", value: style(f, patch) },
  ],
  align: (a: number): CellEdit[] => [{ section: "Paragraph", cell: "HorzAlign", value: String(a) }],
  valign: (a: number): CellEdit[] => [{ cell: "VerticalAlign", value: String(a) }],
  rounding: (inches: number): CellEdit[] => [{ cell: "Rounding", value: String(inches) }],
  endArrow: (kind: number): CellEdit[] => [{ cell: "EndArrow", value: String(kind) }],
  beginArrow: (kind: number): CellEdit[] => [{ cell: "BeginArrow", value: String(kind) }],
};

/** The toolbar's format group for the current selection. */
export function FormatBar({ format, apply }: { format: ShapeFormat; apply: (cells: CellEdit[]) => void }) {
  const toggle = (k: "bold" | "italic" | "underline", icon: IconName, label: string, keys: string) => (
    <button
      type="button"
      className="tb"
      aria-label={label}
      aria-pressed={format[k]}
      data-tip={`${label}  ${keys}`}
      onClick={() => apply(fmt.style(format, { [k]: !format[k] }))}
    >
      <Icon name={icon} size={17} />
    </button>
  );
  return (
    <>
      <div className="tb-group">
        <ColorPicker icon="fill" label="Fill" value={hex(format.fill)} allowNone noneLabel="No fill" onPick={(c) => apply(fmt.fill(c))} />
        <ColorPicker icon="lineColor" label="Line colour" value={hex(format.line)} allowNone noneLabel="No line" onPick={(c) => apply(fmt.line(c))} />
        <select
          className="tb-select narrow"
          aria-label="Line weight"
          data-tip="Line weight"
          value={WEIGHTS.find((w) => Math.abs(w - format.lineWeight) < 0.05) ?? ""}
          onChange={(e) => apply(fmt.weight(Number(e.target.value)))}
        >
          {!WEIGHTS.some((w) => Math.abs(w - format.lineWeight) < 0.05) && <option value="">{format.lineWeight.toFixed(2)} pt</option>}
          {WEIGHTS.map((w) => (
            <option key={w} value={w}>
              {w} pt
            </option>
          ))}
        </select>
        <select
          className="tb-select narrow"
          aria-label="Line pattern"
          value={PATTERNS.some(([p]) => p === format.linePattern) ? format.linePattern : ""}
          onChange={(e) => apply(fmt.pattern(Number(e.target.value)))}
        >
          {!PATTERNS.some(([p]) => p === format.linePattern) && <option value="">{format.linePattern === 0 ? "None" : "Custom"}</option>}
          {PATTERNS.map(([p, name]) => (
            <option key={p} value={p}>
              {name}
            </option>
          ))}
        </select>
      </div>
      <div className="tb-group">
        <select className="tb-select font" aria-label="Font" value={format.font} onChange={(e) => apply(fmt.font(e.target.value))}>
          {!FONTS.includes(format.font) && <option value={format.font}>{format.font}</option>}
          {FONTS.map((f) => (
            <option key={f} value={f} style={{ fontFamily: f }}>
              {f}
            </option>
          ))}
        </select>
        <select
          className="tb-select size"
          aria-label="Font size"
          value={Math.round(format.size)}
          onChange={(e) => apply(fmt.size(Number(e.target.value)))}
        >
          {!SIZES.includes(Math.round(format.size)) && <option value={Math.round(format.size)}>{Math.round(format.size)}</option>}
          {SIZES.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
        {toggle("bold", "bold", "Bold", "Ctrl+B")}
        {toggle("italic", "italic", "Italic", "Ctrl+I")}
        {toggle("underline", "underline", "Underline", "Ctrl+U")}
        <ColorPicker icon="text" label="Text colour" value={hex(format.textColor)} onPick={(c) => c && apply(fmt.textColor(c))} />
      </div>
      <div className="tb-group">
        {(
          [
            [0, "alignLeft", "Align left"],
            [1, "alignCenter", "Centre"],
            [2, "alignRight", "Align right"],
          ] as [number, IconName, string][]
        ).map(([a, icon, label]) => (
          <button
            type="button"
            key={a}
            className="tb"
            aria-label={label}
            aria-pressed={format.align === a}
            data-tip={label}
            onClick={() => apply(fmt.align(a))}
          >
            <Icon name={icon} size={17} />
          </button>
        ))}
        {(
          [
            [0, "valignTop", "Top"],
            [1, "valignMiddle", "Middle"],
            [2, "valignBottom", "Bottom"],
          ] as [number, IconName, string][]
        ).map(([a, icon, label]) => (
          <button
            type="button"
            key={`v${a}`}
            className="tb"
            aria-label={`Text ${label.toLowerCase()}`}
            aria-pressed={format.valign === a}
            data-tip={`Text ${label.toLowerCase()}`}
            onClick={() => apply(fmt.valign(a))}
          >
            <Icon name={icon} size={17} />
          </button>
        ))}
      </div>
    </>
  );
}
