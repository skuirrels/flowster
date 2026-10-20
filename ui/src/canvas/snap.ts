// Snapping while moving, resizing and drawing: to other shapes' edges and
// centres (smart guides, as in Figma or Lucidchart), to the page, and to the
// grid. Also the gap measurements shown to the nearest neighbours.
// Page inches, +Y up.

import type { ShapeBox } from "../engine/protocol";
import { pageBounds, type Pt } from "./edit";

export type Rect = [number, number, number, number];

/** A guide line between two page points (drawn dashed). */
export interface Guide {
  a: Pt;
  b: Pt;
}

/** A measured gap: a line between two page points with its length. */
export interface Measure {
  a: Pt;
  b: Pt;
  label: string;
}

export interface Snapper {
  /** Snap targets: x positions and y positions with the extents they come from. */
  xs: { v: number; from: Rect }[];
  ys: { v: number; from: Rect }[];
  /** Grid step in inches, or null when grid snapping is off. */
  grid: number | null;
  /** Snap distance in inches. */
  tol: number;
  /** Bounds of the shapes not being moved, for gap measurements. */
  others: Rect[];
}

/**
 * Snap targets from every shape except `moving`, plus the page's edges and
 * centre. Connectors are left out: they follow the shapes, so lining up with
 * them would mostly mean lining up with what is being moved.
 */
export function snapper(
  shapes: ShapeBox[],
  moving: number[],
  page: { width: number; height: number },
  opts: { guides: boolean; grid: number | null; tol: number },
): Snapper {
  const others = shapes.filter((b) => !moving.includes(b.id) && !b.connector).map(pageBounds);
  const pageRect: Rect = [0, 0, page.width, page.height];
  const xs: Snapper["xs"] = [];
  const ys: Snapper["ys"] = [];
  if (opts.guides) {
    for (const r of [...others, pageRect]) {
      for (const v of [r[0], (r[0] + r[2]) / 2, r[2]]) xs.push({ v, from: r });
      for (const v of [r[1], (r[1] + r[3]) / 2, r[3]]) ys.push({ v, from: r });
    }
  }
  return { xs, ys, grid: opts.grid, tol: opts.tol, others: opts.guides ? others : [] };
}

/** The best snap of any of `candidates` to `targets`: the offset to apply and the target value. */
function best(candidates: number[], targets: { v: number; from: Rect }[], tol: number) {
  let found: { d: number; v: number } | null = null;
  for (const c of candidates) {
    for (const t of targets) {
      const d = t.v - c;
      if (Math.abs(d) <= tol && (!found || Math.abs(d) < Math.abs(found.d) - 1e-9)) found = { d, v: t.v };
    }
  }
  return found;
}

function gridSnap(v: number, grid: number) {
  return Math.round(v / grid) * grid - v;
}

/** Vertical guides at `x` through the moving rect and every target lined up with it. */
function guidesAt(axis: "x" | "y", v: number, r: Rect, targets: { v: number; from: Rect }[]): Guide[] {
  const hits = targets.filter((t) => Math.abs(t.v - v) < 1e-6).map((t) => t.from);
  if (hits.length === 0) return [];
  const span = [r, ...hits];
  if (axis === "x") {
    const lo = Math.min(...span.map((s) => s[1]));
    const hi = Math.max(...span.map((s) => s[3]));
    return [{ a: [v, lo], b: [v, hi] }];
  }
  const lo = Math.min(...span.map((s) => s[0]));
  const hi = Math.max(...span.map((s) => s[2]));
  return [{ a: [lo, v], b: [hi, v] }];
}

/**
 * Snaps a moving rectangle: shape and page guides win over the grid. Returns
 * the extra offset to apply, the guides to draw and the gaps to its neighbours.
 */
export function snapRect(r: Rect, s: Snapper): { d: Pt; guides: Guide[]; measures: Measure[] } {
  const cx = (r[0] + r[2]) / 2;
  const cy = (r[1] + r[3]) / 2;
  const bx = best([r[0], cx, r[2]], s.xs, s.tol);
  const by = best([r[1], cy, r[3]], s.ys, s.tol);
  const dx = bx ? bx.d : s.grid ? gridSnap(r[0], s.grid) : 0;
  const dy = by ? by.d : s.grid ? gridSnap(r[1], s.grid) : 0;
  const moved: Rect = [r[0] + dx, r[1] + dy, r[2] + dx, r[3] + dy];
  const guides = [...(bx ? guidesAt("x", bx.v, moved, s.xs) : []), ...(by ? guidesAt("y", by.v, moved, s.ys) : [])];
  return { d: [dx, dy], guides, measures: gaps(moved, s.others) };
}

/** Snaps a single point (a resize handle, a line end, a drawing corner). */
export function snapPoint(p: Pt, s: Snapper): { p: Pt; guides: Guide[] } {
  const bx = best([p[0]], s.xs, s.tol);
  const by = best([p[1]], s.ys, s.tol);
  const x = bx ? bx.v : s.grid ? p[0] + gridSnap(p[0], s.grid) : p[0];
  const y = by ? by.v : s.grid ? p[1] + gridSnap(p[1], s.grid) : p[1];
  const dot: Rect = [x, y, x, y];
  const guides = [...(bx ? guidesAt("x", x, dot, s.xs) : []), ...(by ? guidesAt("y", y, dot, s.ys) : [])];
  return { p: [x, y], guides };
}

export function formatInches(v: number): string {
  return `${(Math.round(v * 100) / 100).toFixed(2).replace(/\.?0+$/, "") || "0"} in`;
}

/** Gaps from `r` to its nearest neighbour on each side that overlaps it across the gap. */
function gaps(r: Rect, others: Rect[]): Measure[] {
  const out: Measure[] = [];
  const overlapY = (o: Rect) => Math.min(r[3], o[3]) - Math.max(r[1], o[1]);
  const overlapX = (o: Rect) => Math.min(r[2], o[2]) - Math.max(r[0], o[0]);
  const nearest = (cands: { gap: number; o: Rect }[]) =>
    cands.filter((c) => c.gap > 1e-6).sort((a, b) => a.gap - b.gap)[0];
  const right = nearest(others.filter((o) => overlapY(o) > 0).map((o) => ({ gap: o[0] - r[2], o })));
  const left = nearest(others.filter((o) => overlapY(o) > 0).map((o) => ({ gap: r[0] - o[2], o })));
  const up = nearest(others.filter((o) => overlapX(o) > 0).map((o) => ({ gap: o[1] - r[3], o })));
  const down = nearest(others.filter((o) => overlapX(o) > 0).map((o) => ({ gap: r[1] - o[3], o })));
  const midY = (o: Rect) => (Math.max(r[1], o[1]) + Math.min(r[3], o[3])) / 2;
  const midX = (o: Rect) => (Math.max(r[0], o[0]) + Math.min(r[2], o[2])) / 2;
  if (right) out.push({ a: [r[2], midY(right.o)], b: [right.o[0], midY(right.o)], label: formatInches(right.gap) });
  if (left) out.push({ a: [left.o[2], midY(left.o)], b: [r[0], midY(left.o)], label: formatInches(left.gap) });
  if (up) out.push({ a: [midX(up.o), r[3]], b: [midX(up.o), up.o[1]], label: formatInches(up.gap) });
  if (down) out.push({ a: [midX(down.o), down.o[3]], b: [midX(down.o), r[1]], label: formatInches(down.gap) });
  return out;
}
