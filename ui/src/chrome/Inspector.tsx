// The Inspector: properties of the selection, or of the page and document
// when nothing is selected. Numeric fields commit on Enter or when they lose focus.

import { useEffect, useState } from "react";

import type { CellEdit, Command, DisplayList, DocInfo, ShapeBox } from "../engine/protocol";
import { ColorPicker, fmt as cells, hex } from "./FormatBar";
import { Icon } from "./icons";

/** Arrowheads offered for lines and connectors (Visio arrow kinds). */
const ARROWS: [number, string][] = [
  [0, "None"],
  [4, "Filled arrow"],
  [2, "Small filled arrow"],
  [13, "Long filled arrow"],
  [1, "Open arrow"],
  [3, "Open arrow (wide)"],
  [16, "Hollow arrow"],
  [10, "Dot"],
  [20, "Hollow dot"],
  [22, "Diamond"],
  [11, "Square"],
  [24, "Tick (one)"],
  [27, "Crow's foot (many)"],
  [29, "Zero or many"],
  [31, "Zero or one"],
];

function ArrowSelect({ label, value, onPick }: { label: string; value: number; onPick: (v: number) => void }) {
  return (
    <label className="field wide">
      <span className="field-label">{label}</span>
      <span className="field-box">
        <select value={value} onChange={(e) => onPick(Number(e.target.value))}>
          {!ARROWS.some(([k]) => k === value) && <option value={value}>Type {value}</option>}
          {ARROWS.map(([k, name]) => (
            <option key={k} value={k}>
              {name}
            </option>
          ))}
        </select>
      </span>
    </label>
  );
}

const fmt = (v: number) => (Math.abs(v) < 5e-5 ? "0" : v.toFixed(3).replace(/\.?0+$/, ""));

/** A number field in inches. */
function Num({
  label,
  value,
  onCommit,
  min,
}: {
  label: string;
  value: number;
  onCommit?: (v: number) => void;
  min?: number;
}) {
  const [draft, setDraft] = useState(fmt(value));
  useEffect(() => setDraft(fmt(value)), [value]);
  const commit = () => {
    const v = Number.parseFloat(draft);
    if (!Number.isFinite(v) || (min !== undefined && v < min)) {
      setDraft(fmt(value));
    } else if (Math.abs(v - value) > 1e-6) {
      onCommit?.(v);
    }
  };
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      <span className="field-box">
        <input
          inputMode="decimal"
          value={draft}
          readOnly={!onCommit}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") (e.currentTarget as HTMLInputElement).blur();
            if (e.key === "Escape") {
              setDraft(fmt(value));
              (e.currentTarget as HTMLInputElement).blur();
            }
          }}
        />
        <span className="field-unit">in</span>
      </span>
    </label>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="insp-section">
      <h3>{title}</h3>
      {children}
    </section>
  );
}

function shapeFrame(b: ShapeBox) {
  const [a, bb, c, d, e, f] = b.transform;
  const pin = [a * (b.width / 2) + c * (b.height / 2) + e, bb * (b.width / 2) + d * (b.height / 2) + f];
  const angle = (Math.atan2(bb, a) * 180) / Math.PI;
  return { pinX: pin[0], pinY: pin[1], angle };
}

export function Inspector({
  dl,
  info,
  selection,
  onCommand,
  onTitle,
  onDelete,
}: {
  dl: DisplayList;
  info: DocInfo;
  selection: number[];
  onCommand: (cmd: Command) => void;
  onTitle: (title: string) => void;
  onDelete: () => void;
}) {
  const page = dl.page.index;
  const boxes = selection.map((id) => dl.shapes.find((b) => b.id === id)).filter((b) => b !== undefined);

  if (boxes.length === 0) {
    return (
      <aside className="inspector" aria-label="Inspector">
        <Section title="Document">
          <TitleField value={info.meta.title ?? ""} onCommit={onTitle} />
          <dl className="insp-facts">
            <dt>File</dt>
            <dd title={info.fileName}>{info.fileName}</dd>
          </dl>
        </Section>
        <Section title="Page">
          <dl className="insp-facts">
            <dt>Name</dt>
            <dd>{dl.page.name}</dd>
            <dt>Size</dt>
            <dd>
              {fmt(dl.page.width)} × {fmt(dl.page.height)} in
            </dd>
            <dt>Shapes</dt>
            <dd>{dl.shapes.length}</dd>
          </dl>
        </Section>
        <p className="insp-hint">Select a shape to see and change its position, size and text.</p>
      </aside>
    );
  }

  if (boxes.length > 1) {
    return (
      <aside className="inspector" aria-label="Inspector">
        <Section title="Selection">
          <p className="insp-big">{boxes.length} shapes</p>
          <button type="button" className="btn danger-ghost" onClick={onDelete}>
            <Icon name="trash" size={16} /> Delete
          </button>
        </Section>
      </aside>
    );
  }

  const b = boxes[0];
  const oneD = b.begin !== undefined && b.end !== undefined;
  const { pinX, pinY, angle } = shapeFrame(b);
  const bounds = (patch: Partial<{ pinX: number; pinY: number; width: number; height: number }>) =>
    onCommand({ kind: "setBounds", page, shape: b.id, pinX, pinY, width: b.width, height: b.height, ...patch });
  const format = (edits: CellEdit[]) => onCommand({ kind: "setCells", page, shapes: [b.id], cells: edits });
  const f = b.format;
  const ends = (patch: Partial<{ beginX: number; beginY: number; endX: number; endY: number }>) =>
    onCommand({
      kind: "setEndpoints",
      page,
      shape: b.id,
      beginX: b.begin![0],
      beginY: b.begin![1],
      endX: b.end![0],
      endY: b.end![1],
      ...patch,
    });

  return (
    <aside className="inspector" aria-label="Inspector">
      <div className="insp-title">
        <Icon name={oneD ? "line" : "rectangle"} size={16} />
        <span>{b.connector ? "Connector" : oneD ? "Line" : "Shape"}</span>
        <span className="insp-id">#{b.id}</span>
      </div>
      {oneD ? (
        <Section title="Endpoints">
          <div className="field-grid">
            <Num label="Begin X" value={b.begin![0]} onCommit={(v) => ends({ beginX: v })} />
            <Num label="Begin Y" value={b.begin![1]} onCommit={(v) => ends({ beginY: v })} />
            <Num label="End X" value={b.end![0]} onCommit={(v) => ends({ endX: v })} />
            <Num label="End Y" value={b.end![1]} onCommit={(v) => ends({ endY: v })} />
          </div>
          <dl className="insp-facts">
            <dt>Length</dt>
            <dd>{fmt(b.width)} in</dd>
          </dl>
        </Section>
      ) : (
        <>
          <Section title="Position">
            <div className="field-grid">
              <Num label="X" value={pinX} onCommit={(v) => bounds({ pinX: v })} />
              <Num label="Y" value={pinY} onCommit={(v) => bounds({ pinY: v })} />
            </div>
          </Section>
          <Section title="Size">
            <div className="field-grid">
              <Num label="Width" value={b.width} min={0.01} onCommit={(v) => bounds({ width: v })} />
              <Num label="Height" value={b.height} min={0.01} onCommit={(v) => bounds({ height: v })} />
            </div>
            {Math.abs(angle) > 0.01 && (
              <dl className="insp-facts">
                <dt>Rotation</dt>
                <dd>{fmt(angle)}°</dd>
              </dl>
            )}
          </Section>
        </>
      )}
      <Section title="Style">
        <div className="style-row">
          {!oneD && (
            <span className="style-item">
              <ColorPicker icon="fill" label="Fill" value={hex(f.fill)} allowNone noneLabel="No fill" onPick={(c) => format(cells.fill(c))} />
              Fill
            </span>
          )}
          <span className="style-item">
            <ColorPicker icon="lineColor" label="Line colour" value={hex(f.line)} allowNone noneLabel="No line" onPick={(c) => format(cells.line(c))} />
            Line
          </span>
          <span className="style-item">
            <ColorPicker icon="text" label="Text colour" value={hex(f.textColor)} onPick={(c) => c && format(cells.textColor(c))} />
            Text
          </span>
        </div>
        {oneD ? (
          <div className="field-grid one">
            <ArrowSelect label="Begin arrow" value={f.beginArrow} onPick={(v) => format(cells.beginArrow(v))} />
            <ArrowSelect label="End arrow" value={f.endArrow} onPick={(v) => format(cells.endArrow(v))} />
          </div>
        ) : (
          <div className="field-grid">
            <Num label="Rounding" value={f.rounding} min={0} onCommit={(v) => format(cells.rounding(v))} />
          </div>
        )}
      </Section>
      <Section title="Text">
        <TextField
          key={b.id}
          value={b.text ?? ""}
          onCommit={(text) => onCommand({ kind: "setText", page, shape: b.id, text })}
        />
      </Section>
      <div className="insp-actions">
        <button type="button" className="btn danger-ghost" onClick={onDelete}>
          <Icon name="trash" size={16} /> Delete shape
        </button>
      </div>
    </aside>
  );
}

function TextField({ value, onCommit }: { value: string; onCommit: (v: string) => void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  return (
    <textarea
      className="insp-text"
      rows={3}
      placeholder="Add text…"
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => draft !== value && onCommit(draft)}
      onKeyDown={(e) => {
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          (e.currentTarget as HTMLTextAreaElement).blur();
        }
      }}
    />
  );
}

function TitleField({ value, onCommit }: { value: string; onCommit: (v: string) => void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  return (
    <label className="field wide">
      <span className="field-label">Title</span>
      <span className="field-box">
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => draft !== value && onCommit(draft)}
          onKeyDown={(e) => e.key === "Enter" && (e.currentTarget as HTMLInputElement).blur()}
        />
      </span>
    </label>
  );
}
