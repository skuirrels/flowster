// The Shapes panel: the built-in stencils (Basic, Flowchart, Swimlanes, Text).
// Drag a shape onto the page, or click it to drop it in the middle of the view.

import { useMemo, useState } from "react";

import type { LibraryEntry } from "../engine/protocol";
import { Icon } from "./icons";

/** The drag payload type for shapes dropped on the canvas. */
export const SHAPE_MIME = "application/x-flowster-shape";

/** A library shape's outline, drawn to fit its box. */
export function ShapePreview({ e }: { e: LibraryEntry }) {
  if (e.id === "text") {
    return (
      <svg viewBox="0 0 40 24" className="shape-preview" aria-hidden="true">
        <text x="20" y="17" textAnchor="middle" fontSize="15" fontWeight="600" fill="currentColor">
          Abc
        </text>
      </svg>
    );
  }
  const pad = Math.max(e.width, e.height) * 0.08;
  return (
    <svg
      viewBox={`${-pad} ${-pad} ${e.width + 2 * pad} ${e.height + 2 * pad}`}
      className="shape-preview"
      preserveAspectRatio="xMidYMid meet"
      aria-hidden="true"
    >
      <path d={e.preview} vectorEffect="non-scaling-stroke" fillRule="evenodd" />
    </svg>
  );
}

export function ShapesPanel({ library, onAdd }: { library: LibraryEntry[]; onAdd: (e: LibraryEntry) => void }) {
  const [query, setQuery] = useState("");
  const [closed, setClosed] = useState<Set<string>>(new Set());
  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    const out = new Map<string, LibraryEntry[]>();
    for (const e of library) {
      if (q && !e.name.toLowerCase().includes(q) && !e.category.toLowerCase().includes(q)) continue;
      out.set(e.category, [...(out.get(e.category) ?? []), e]);
    }
    return [...out];
  }, [library, query]);

  return (
    <aside className="shapes-panel" aria-label="Shapes">
      <div className="panel-head">
        <Icon name="shapes" size={16} />
        <span>Shapes</span>
      </div>
      <label className="search">
        <Icon name="search" size={15} />
        <input placeholder="Search shapes" value={query} onChange={(e) => setQuery(e.target.value)} />
      </label>
      <div className="stencils">
        {groups.map(([category, entries]) => {
          const open = query.trim() !== "" || !closed.has(category);
          return (
            <section className="stencil" key={category}>
              <button
                type="button"
                className="stencil-head"
                aria-expanded={open}
                onClick={() =>
                  setClosed((c) => {
                    const n = new Set(c);
                    if (n.has(category)) n.delete(category);
                    else n.add(category);
                    return n;
                  })
                }
              >
                <Icon name="chevron" size={14} className={open ? "" : "rot"} />
                {category}
                <span className="count">{entries.length}</span>
              </button>
              {open && (
                <div className="tiles">
                  {entries.map((e) => (
                    <button
                      type="button"
                      key={e.id}
                      className="shape-tile"
                      title={`${e.name} — drag onto the page, or click to add`}
                      draggable
                      onDragStart={(ev) => {
                        ev.dataTransfer.setData(SHAPE_MIME, e.id);
                        ev.dataTransfer.effectAllowed = "copy";
                      }}
                      onClick={() => onAdd(e)}
                    >
                      <ShapePreview e={e} />
                      <span>{e.name}</span>
                    </button>
                  ))}
                </div>
              )}
            </section>
          );
        })}
        {groups.length === 0 && <p className="insp-hint">No shapes match “{query}”.</p>}
      </div>
    </aside>
  );
}
