// The command palette (Ctrl+K): every menu command, every shape and every
// page, found by typing a few letters. The same list, given only shapes and a
// point, is the quick-add popup that opens on a double-click on empty canvas.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import type { LibraryEntry } from "../engine/protocol";
import { Icon, shortcut, type IconName } from "./icons";
import { ShapePreview } from "./ShapesPanel";

export interface PaletteItem {
  id: string;
  label: string;
  /** Shown on the right, and matched too ("Shape", "Page", "View"…). */
  group: string;
  icon?: IconName;
  /** A shape preview instead of an icon. */
  shape?: LibraryEntry;
  keys?: string;
  /** Extra words to match (a shape's category). */
  keywords?: string;
  run: () => void;
}

/** Higher is better; null when the query's letters do not all appear in order. */
function score(item: PaletteItem, q: string): number | null {
  if (!q) return 0;
  const label = item.label.toLowerCase();
  const at = label.indexOf(q);
  if (at >= 0) return 1000 - at + (at === 0 || label[at - 1] === " " ? 200 : 0) - label.length * 0.1;
  const extra = `${item.group} ${item.keywords ?? ""}`.toLowerCase();
  if (extra.includes(q)) return 400;
  // Letters in order, preferring runs and word starts ("bf" → "Bring to front").
  let s = 0;
  let j = 0;
  let prev = -2;
  for (let i = 0; i < label.length && j < q.length; i++) {
    if (label[i] !== q[j]) continue;
    s += (i === prev + 1 ? 5 : 1) + (i === 0 || label[i - 1] === " " ? 8 : 0);
    prev = i;
    j++;
  }
  return j === q.length ? s : null;
}

export function CommandPalette({
  items,
  placeholder,
  onClose,
  at,
  label,
}: {
  items: PaletteItem[];
  placeholder: string;
  onClose: () => void;
  /** Open as a small popup at this viewport point instead of centred at the top. */
  at?: [number, number];
  label: string;
}) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const box = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState(at);

  const results = useMemo(() => {
    const q = query.trim().toLowerCase();
    return items
      .map((it, i) => ({ it, i, s: score(it, q) }))
      .filter((r) => r.s !== null)
      .sort((a, b) => (q ? b.s! - a.s! : 0) || a.i - b.i)
      .slice(0, 80)
      .map((r) => r.it);
  }, [items, query]);

  useEffect(() => setActive(0), [query]);

  // Keep a popup on screen.
  useLayoutEffect(() => {
    if (!at || !box.current) return;
    const r = box.current.getBoundingClientRect();
    setPos([Math.max(8, Math.min(at[0], window.innerWidth - r.width - 8)), Math.max(8, Math.min(at[1], window.innerHeight - r.height - 8))]);
  }, [at]);

  useEffect(() => {
    const away = (e: PointerEvent) => !box.current?.contains(e.target as Node) && onClose();
    window.addEventListener("pointerdown", away, true);
    return () => window.removeEventListener("pointerdown", away, true);
  }, [onClose]);

  useEffect(() => {
    list.current?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const run = (it: PaletteItem | undefined) => {
    if (!it) return;
    onClose();
    it.run();
  };

  return (
    <div className={at ? "palette-pop" : "palette-backdrop"}>
      <div
        className={`command-palette${at ? " at" : ""}`}
        role="dialog"
        aria-label={label}
        ref={box}
        style={at && pos ? { left: pos[0], top: pos[1] } : undefined}
      >
        <label className="palette-search">
          <Icon name="search" size={16} />
          <input
            autoFocus
            placeholder={placeholder}
            value={query}
            role="combobox"
            aria-expanded="true"
            aria-controls="palette-results"
            aria-activedescendant={results[active] ? `palette-${results[active].id}` : undefined}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") onClose();
              else if (e.key === "ArrowDown") setActive((a) => Math.min(a + 1, results.length - 1));
              else if (e.key === "ArrowUp") setActive((a) => Math.max(a - 1, 0));
              else if (e.key === "Enter") run(results[active]);
              else return;
              e.preventDefault();
              e.stopPropagation();
            }}
          />
          {!at && <kbd>Esc</kbd>}
        </label>
        <div className="palette-results" id="palette-results" role="listbox" ref={list}>
          {results.length === 0 && <div className="palette-empty">No matches</div>}
          {results.map((it, i) => (
            <div
              key={it.id}
              id={`palette-${it.id}`}
              data-index={i}
              role="option"
              aria-selected={i === active}
              className={`palette-item${i === active ? " active" : ""}`}
              onPointerMove={() => setActive(i)}
              onClick={() => run(it)}
            >
              <span className="palette-icon">
                {it.shape ? <ShapePreview e={it.shape} /> : it.icon ? <Icon name={it.icon} size={16} /> : null}
              </span>
              <span className="palette-label">{it.label}</span>
              {it.keys ? <kbd className="menu-keys">{shortcut(it.keys)}</kbd> : <span className="palette-group">{it.group}</span>}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
