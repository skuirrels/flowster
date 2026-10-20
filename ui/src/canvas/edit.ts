// Editing geometry for the canvas: hit-testing shapes and handles, the maths
// of resizing in a shape's own frame, and the selection/preview overlay.
// Everything here works in page inches (+Y up) except where noted.

import type { Affine, ShapeBox } from "../engine/protocol";
import type { View } from "./renderer";

export type Pt = [number, number];

export function apply(m: Affine, [x, y]: Pt): Pt {
  return [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
}

export function invert(m: Affine): Affine | null {
  const [a, b, c, d, e, f] = m;
  const det = a * d - b * c;
  if (Math.abs(det) < 1e-12) return null;
  return [d / det, -b / det, -c / det, a / det, (c * f - d * e) / det, (b * e - a * f) / det];
}

export function isOneD(b: ShapeBox): boolean {
  return b.begin !== undefined && b.end !== undefined;
}

function distToSegment(p: Pt, a: Pt, b: Pt): number {
  const [dx, dy] = [b[0] - a[0], b[1] - a[1]];
  const len2 = dx * dx + dy * dy;
  const t = len2 > 0 ? Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2)) : 0;
  return Math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
}

/** The top-most shape under `p` (page inches); `tol` is the pick slack in inches. */
export function hitShape(shapes: ShapeBox[], p: Pt, tol: number): ShapeBox | null {
  for (let i = shapes.length - 1; i >= 0; i--) {
    const b = shapes[i];
    if (isOneD(b)) {
      if (distToSegment(p, b.begin!, b.end!) <= tol) return b;
      continue;
    }
    const inv = invert(b.transform);
    if (!inv) continue;
    const [x, y] = apply(inv, p);
    // Local units are inches too, unless the shape is scaled by a group (top-level ones aren't).
    if (x >= -tol && x <= b.width + tol && y >= -tol && y <= b.height + tol) return b;
  }
  return null;
}

/** Corners and edge midpoints in local coordinates, clockwise from bottom-left. */
export const HANDLE_UNITS: Pt[] = [
  [0, 0],
  [0.5, 0],
  [1, 0],
  [1, 0.5],
  [1, 1],
  [0.5, 1],
  [0, 1],
  [0, 0.5],
];

/** Page positions of a 2-D shape's resize handles (or a 1-D shape's two ends). */
export function handles(b: ShapeBox): Pt[] {
  if (isOneD(b)) return [b.begin!, b.end!];
  return HANDLE_UNITS.map(([u, v]) => apply(b.transform, [u * b.width, v * b.height]));
}

/** Corners of a shape's frame on the page. */
export function corners(b: ShapeBox): Pt[] {
  return [
    [0, 0],
    [b.width, 0],
    [b.width, b.height],
    [0, b.height],
  ].map((p) => apply(b.transform, p as Pt));
}

export interface Bounds {
  pinX: number;
  pinY: number;
  width: number;
  height: number;
}

/**
 * New bounds of a 2-D shape when handle `h` is dragged to `p`: the opposite
 * side stays put, measured in the shape's own (possibly rotated) frame.
 */
export function resized(b: ShapeBox, h: number, p: Pt): { bounds: Bounds; frame: Pt[] } | null {
  const inv = invert(b.transform);
  if (!inv) return null;
  const [qx, qy] = apply(inv, p);
  const [u, v] = HANDLE_UNITS[h];
  const min = 0.01;
  let [x0, x1, y0, y1] = [0, b.width, 0, b.height];
  if (u === 0) x0 = Math.min(qx, x1 - min);
  if (u === 1) x1 = Math.max(qx, x0 + min);
  if (v === 0) y0 = Math.min(qy, y1 - min);
  if (v === 1) y1 = Math.max(qy, y0 + min);
  const [pinX, pinY] = apply(b.transform, [(x0 + x1) / 2, (y0 + y1) / 2]);
  const frame = (
    [
      [x0, y0],
      [x1, y0],
      [x1, y1],
      [x0, y1],
    ] as Pt[]
  ).map((q) => apply(b.transform, q));
  return { bounds: { pinX, pinY, width: x1 - x0, height: y1 - y0 }, frame };
}

/** Axis-aligned page bounds of a shape: [x0, y0, x1, y1]. */
export function pageBounds(b: ShapeBox): [number, number, number, number] {
  const pts = isOneD(b) ? [b.begin!, b.end!] : corners(b);
  const xs = pts.map((p) => p[0]);
  const ys = pts.map((p) => p[1]);
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
}

/** Page inches → CSS pixels. */
export function toScreen(view: View, pageHeight: number, [x, y]: Pt): Pt {
  return [view.ox + x * view.zoom, view.oy + (pageHeight - y) * view.zoom];
}

/** CSS pixels → page inches. */
export function toPage(view: View, pageHeight: number, [sx, sy]: Pt): Pt {
  return [(sx - view.ox) / view.zoom, pageHeight - (sy - view.oy) / view.zoom];
}

/** What the overlay shows on top of the drawing. */
export interface Overlay {
  /** Outlines of selected shapes (page polygons or 1-D segments). */
  selected: Pt[][];
  /** Handle positions (page). */
  handles: Pt[];
  /** A dashed preview: a polygon, a closed ellipse frame, or a segment. */
  preview?: { kind: "polygon" | "ellipse" | "segment"; pts: Pt[] };
  /** Rubber-band rectangle, screen pixels. */
  marquee?: [number, number, number, number];
  /** AutoConnect arrows: screen position and pointing direction. */
  arrows?: Arrow[];
  /** A shape about to be connected to (page polygon). */
  target?: Pt[];
}

export type Direction = "up" | "down" | "left" | "right";

export interface Arrow {
  at: Pt;
  dir: Direction;
  hot: boolean;
}

/** Distance of the AutoConnect arrows outside a shape, CSS pixels. */
const ARROW_GAP = 20;

/** Where the AutoConnect arrows of a shape go (screen pixels). */
export function arrowsFor(b: ShapeBox, view: View, pageHeight: number, hot: Direction | null): Arrow[] {
  const [x0, y0, x1, y1] = pageBounds(b);
  const [l, t] = toScreen(view, pageHeight, [x0, y1]);
  const [r, bt] = toScreen(view, pageHeight, [x1, y0]);
  const [cx, cy] = [(l + r) / 2, (t + bt) / 2];
  const at: [Direction, Pt][] = [
    ["up", [cx, t - ARROW_GAP]],
    ["down", [cx, bt + ARROW_GAP]],
    ["left", [l - ARROW_GAP, cy]],
    ["right", [r + ARROW_GAP, cy]],
  ];
  return at.map(([dir, p]) => ({ at: p, dir, hot: dir === hot }));
}

/** The arrow under a screen point, if any. */
export function hitArrow(arrows: Arrow[], [x, y]: Pt): Direction | null {
  return arrows.find((a) => Math.hypot(a.at[0] - x, a.at[1] - y) <= 10)?.dir ?? null;
}

/** Selection colour: the brand accent, readable on light and dark pages. */
const ACCENT = "#0d8a7a";

export function drawOverlay(ctx: CanvasRenderingContext2D, view: View, dpr: number, pageHeight: number, o: Overlay) {
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  const s = (p: Pt) => toScreen(view, pageHeight, p);
  ctx.lineWidth = 1;
  ctx.strokeStyle = ACCENT;
  for (const poly of o.selected) {
    ctx.beginPath();
    poly.forEach((p, i) => (i === 0 ? ctx.moveTo(...s(p)) : ctx.lineTo(...s(p))));
    if (poly.length > 2) ctx.closePath();
    ctx.setLineDash([4, 3]);
    ctx.stroke();
  }
  ctx.setLineDash([]);
  ctx.lineWidth = 1.5;
  for (const h of o.handles) {
    const [x, y] = s(h);
    ctx.beginPath();
    ctx.roundRect(x - 4, y - 4, 8, 8, 2);
    ctx.fillStyle = "#ffffff";
    ctx.fill();
    ctx.stroke();
  }
  ctx.lineWidth = 1;
  if (o.preview) {
    const pts = o.preview.pts.map(s);
    ctx.beginPath();
    if (o.preview.kind === "ellipse") {
      const [a, , c] = pts;
      ctx.ellipse((a[0] + c[0]) / 2, (a[1] + c[1]) / 2, Math.abs(c[0] - a[0]) / 2, Math.abs(c[1] - a[1]) / 2, 0, 0, Math.PI * 2);
    } else {
      pts.forEach((p, i) => (i === 0 ? ctx.moveTo(...p) : ctx.lineTo(...p)));
      if (o.preview.kind === "polygon") ctx.closePath();
    }
    ctx.setLineDash([5, 4]);
    ctx.strokeStyle = ACCENT;
    ctx.stroke();
    ctx.setLineDash([]);
  }
  if (o.target) {
    ctx.beginPath();
    o.target.forEach((p, i) => (i === 0 ? ctx.moveTo(...s(p)) : ctx.lineTo(...s(p))));
    ctx.closePath();
    ctx.lineWidth = 2;
    ctx.strokeStyle = ACCENT;
    ctx.stroke();
    ctx.fillStyle = "rgba(13, 138, 122, 0.07)";
    ctx.fill();
    ctx.lineWidth = 1;
  }
  for (const a of o.arrows ?? []) {
    const [x, y] = a.at;
    const rot = { up: -Math.PI / 2, down: Math.PI / 2, left: Math.PI, right: 0 }[a.dir];
    ctx.save();
    ctx.translate(x, y);
    ctx.rotate(rot);
    ctx.beginPath();
    ctx.moveTo(7, 0);
    ctx.lineTo(-5, -6.5);
    ctx.lineTo(-5, 6.5);
    ctx.closePath();
    ctx.fillStyle = a.hot ? ACCENT : "rgba(13, 138, 122, 0.45)";
    ctx.fill();
    ctx.restore();
  }
  if (o.marquee) {
    const [x0, y0, x1, y1] = o.marquee;
    ctx.fillStyle = "rgba(13, 138, 122, 0.08)";
    ctx.fillRect(Math.min(x0, x1), Math.min(y0, y1), Math.abs(x1 - x0), Math.abs(y1 - y0));
    ctx.strokeRect(Math.min(x0, x1), Math.min(y0, y1), Math.abs(x1 - x0), Math.abs(y1 - y0));
  }
}
