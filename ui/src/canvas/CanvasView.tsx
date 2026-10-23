// The drawing surface: owns the view transform (zoom/pan), redraws the display
// list on demand, and turns pointer gestures into editing commands (SPEC §9).
// React renders the frame; drawing and gestures are imperative.

import { useCallback, useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";

import { SHAPE_MIME } from "../chrome/ShapesPanel";
import type { Command, DisplayList, ShapeBox, ShapeKind } from "../engine/protocol";
import {
  arrowsFor,
  centreOf,
  corners,
  drawOverlay,
  handles,
  hitArrow,
  hitShape,
  isOneD,
  pageBounds,
  resized,
  rotateHandle,
  toPage,
  toScreen,
  type Direction,
  type Overlay,
  type Pt,
} from "./edit";
import { drawMinimap, miniSize, miniView, renderMiniPage } from "./minimap";
import { fitView, render, type Theme, type View } from "./renderer";
import { drawRulers, RULER, spacing, type RulerTheme } from "./rulers";
import { formatInches, snapPoint, snapRect, snapper, type Guide, type Measure, type Rect, type Snapper } from "./snap";

/** `select`, the text and connector tools, or a library shape to draw by dragging. */
export type Tool = "select" | "text" | "connector" | ShapeKind;

export interface CanvasHandle {
  fit(): void;
  zoomBy(factor: number): void;
  setZoom(pxPerInch: number): void;
  /** A shape's on-screen bounds in CSS pixels relative to the canvas: [x, y, w, h]. */
  screenBounds(id: number): [number, number, number, number] | null;
  /** The page point at the middle of the view. */
  viewCentre(): Pt | null;
  /** Zooms to show page bounds [x0, y0, x1, y1] (the selection). */
  zoomTo(bounds: Rect): void;
  /**
   * How to show the page when its next display list has a new size: fit it,
   * or keep the shapes where they are on screen. `moved` is how far the engine
   * moved every shape (Auto Size adding or dropping sheets on the left or bottom).
   */
  expectPageChange(change: { fit: boolean; moved?: Pt }): void;
}

/** The grid step that snapping uses at a zoom: the grid's minor lines. */
export function gridStep(zoom: number): { minor: number; major: number } {
  const { major, divisions } = spacing(zoom, 48, 12);
  return { minor: major / divisions, major };
}

export interface SnapOptions {
  grid: boolean;
  guides: boolean;
}

interface Mods {
  shift: boolean;
  alt: boolean;
  /** Ctrl/⌘ held: no snapping. */
  free: boolean;
}

const modsOf = (e: { shiftKey: boolean; altKey: boolean; ctrlKey: boolean; metaKey: boolean }): Mods => ({
  shift: e.shiftKey,
  alt: e.altKey,
  free: e.ctrlKey || e.metaKey,
});

const num = (v: number) => String(Math.round(v * 100) / 100);
const degrees = (rad: number) => {
  const d = Math.round(((rad * 180) / Math.PI) * 10) / 10;
  return `${Object.is(d, -0) ? 0 : d}°`;
};

const MIN_ZOOM = 2;
const MAX_ZOOM = 20000;
/** CSS pixels per inch at 100 %. */
export const PX_PER_INCH = 96;
/** Pick and handle slack, CSS pixels. */
const SLACK = 6;
/** A drag shorter than this (CSS pixels) is a click. */
const CLICK = 4;

type Gesture =
  | { kind: "pan"; x: number; y: number }
  | { kind: "move"; start: Pt; cur: Pt; ids: number[]; mods: Mods; snap: Snapper }
  | { kind: "resize"; id: number; handle: number; cur: Pt; mods: Mods; snap: Snapper }
  | { kind: "rotate"; id: number; start: Pt; cur: Pt; mods: Mods }
  | { kind: "draw"; shape: ShapeKind; start: Pt; cur: Pt; mods: Mods; snap: Snapper }
  | { kind: "connect"; from: number | null; start: Pt; cur: Pt }
  | { kind: "marquee"; start: Pt; cur: Pt; additive: boolean };

function themeColors(el: HTMLElement): Theme & { ruler: RulerTheme; raised: string; accent: string } {
  const s = getComputedStyle(el);
  const v = (name: string, fallback: string) => s.getPropertyValue(name).trim() || fallback;
  return {
    pasteboard: v("--pasteboard", "#e6e8ec"),
    pageShadow: v("--page-shadow", "rgba(0,0,0,0.25)"),
    raised: v("--raised", "#ffffff"),
    accent: v("--accent", "#0d8a7a"),
    ruler: {
      background: v("--sunken", "#f5f6f8"),
      page: v("--chrome", "#ffffff"),
      text: v("--text-2", "#4a5260"),
      tick: v("--muted", "#7a8494"),
      border: v("--border", "#e3e6eb"),
    },
  };
}

const centre = (b: ShapeBox): Pt => {
  const [x0, y0, x1, y1] = pageBounds(b);
  return [(x0 + x1) / 2, (y0 + y1) / 2];
};

const union = (rs: Rect[]): Rect => [
  Math.min(...rs.map((r) => r[0])),
  Math.min(...rs.map((r) => r[1])),
  Math.max(...rs.map((r) => r[2])),
  Math.max(...rs.map((r) => r[3])),
];

/** `p` turned to the nearest 45° direction from `a`, keeping its distance. */
function steps45(a: Pt, p: Pt): Pt {
  const len = Math.hypot(p[0] - a[0], p[1] - a[1]);
  const ang = Math.round(Math.atan2(p[1] - a[1], p[0] - a[0]) / (Math.PI / 4)) * (Math.PI / 4);
  return [a[0] + len * Math.cos(ang), a[1] + len * Math.sin(ang)];
}

/** An angle in (-π, π]. */
function normalAngle(a: number): number {
  const r = (((a + Math.PI) % (2 * Math.PI)) + 2 * Math.PI) % (2 * Math.PI) - Math.PI;
  return Math.abs(r) < 1e-9 ? 0 : r;
}

export function CanvasView({
  dl,
  tool,
  selection,
  onSelection,
  onCommand,
  onEditText,
  onDropShape,
  onZoomChange,
  grid,
  rulers,
  autoFit,
  onManualView,
  snap,
  minimap,
  onDuplicate,
  onQuickAdd,
  ref,
}: {
  dl: DisplayList;
  tool: Tool;
  selection: number[];
  onSelection: (ids: number[]) => void;
  onCommand: (cmd: Command) => void;
  onEditText: (id: number) => void;
  /** A Shapes-panel shape dropped at a page point. */
  onDropShape: (id: string, at: Pt) => void;
  onZoomChange: (pxPerInch: number) => void;
  grid: boolean;
  rulers: boolean;
  /** Fit the page whenever the canvas changes size. */
  autoFit: boolean;
  /** The user zoomed or panned (which ends auto-fit). */
  onManualView: () => void;
  snap: SnapOptions;
  minimap: boolean;
  /** Alt-drag: copies of `ids` moved by (dx, dy). */
  onDuplicate: (ids: number[], dx: number, dy: number) => void;
  /** A double-click on empty canvas: viewport point (for a popup) and page point. */
  onQuickAdd: (client: Pt, at: Pt) => void;
  ref?: Ref<CanvasHandle>;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const view = useRef<View | null>(null);
  const frame = useRef(0);
  const gesture = useRef<Gesture | null>(null);
  const space = useRef(false);
  const hotArrow = useRef<Direction | null>(null);
  const hover = useRef<number | null>(null);
  const lastPage = useRef<number | null>(null);
  const lastSize = useRef<Pt>([0, 0]);
  const pageChange = useRef<{ fit: boolean; moved?: Pt } | null>(null);
  const sel = useRef(selection);
  sel.current = selection;
  const [dropping, setDropping] = useState(false);
  const miniRef = useRef<HTMLCanvasElement>(null);
  const miniCache = useRef<{ dl: DisplayList; dpr: number; img: HTMLCanvasElement } | null>(null);
  const [mw, mh] = miniSize(dl.page);

  const H = dl.page.height;
  const page = dl.page.index;
  const boxOf = useCallback((id: number) => dl.shapes.find((b) => b.id === id) ?? null, [dl]);
  const outline = (b: ShapeBox): Pt[] => (isOneD(b) ? [b.begin!, b.end!] : corners(b));

  /** The AutoConnect arrows of the single selected 2-D shape. */
  const arrows = useCallback(() => {
    const v = view.current;
    if (!v || tool !== "select" || gesture.current || sel.current.length !== 1) return [];
    const b = boxOf(sel.current[0]);
    return b && !isOneD(b) ? arrowsFor(b, v, H, hotArrow.current) : [];
  }, [boxOf, H, tool]);

  /** Last pointer position, canvas CSS pixels (where readouts go). */
  const pointer = useRef<Pt>([0, 0]);

  /** Snap targets for a gesture that moves `ids`. */
  const makeSnap = useCallback(
    (ids: number[]): Snapper => {
      const z = view.current?.zoom ?? PX_PER_INCH;
      return snapper(dl.shapes, ids, dl.page, { guides: snap.guides, grid: snap.grid ? gridStep(z).minor : null, tol: 6 / z });
    },
    [dl, snap.grid, snap.guides],
  );

  /** A move with Shift (one axis) and snapping applied. */
  const resolveMove = useCallback(
    (g: Extract<Gesture, { kind: "move" }>) => {
      let [dx, dy] = [g.cur[0] - g.start[0], g.cur[1] - g.start[1]];
      const lock = g.mods.shift ? (Math.abs(dx) >= Math.abs(dy) ? "y" : "x") : null;
      if (lock === "y") dy = 0;
      if (lock === "x") dx = 0;
      let guides: Guide[] = [];
      let measures: Measure[] = [];
      const boxes = g.ids.map(boxOf).filter((b) => b !== null);
      if (boxes.length > 0 && !g.mods.free) {
        const r = union(boxes.map(pageBounds));
        const s = snapRect([r[0] + dx, r[1] + dy, r[2] + dx, r[3] + dy], g.snap);
        if (lock !== "x") dx += s.d[0];
        if (lock !== "y") dy += s.d[1];
        // Guides along a locked axis no longer apply.
        guides = s.guides.filter((l) => (lock === "x" ? l.a[0] !== l.b[0] : lock === "y" ? l.a[1] !== l.b[1] : true));
        measures = s.measures;
      }
      return { d: [dx, dy] as Pt, guides, measures };
    },
    [boxOf],
  );

  /** A resize with snapping; Shift keeps proportions (corners) or 45° steps (line ends). */
  const resolveResize = useCallback(
    (g: Extract<Gesture, { kind: "resize" }>) => {
      const b = boxOf(g.id);
      if (!b) return null;
      let p = g.cur;
      let guides: Guide[] = [];
      if (!g.mods.free) ({ p, guides } = snapPoint(p, g.snap));
      if (isOneD(b)) {
        if (g.mods.shift) p = steps45(g.handle === 0 ? b.end! : b.begin!, p);
        return { b, p, r: null, guides };
      }
      return { b, p, r: resized(b, g.handle, p, g.mods.shift), guides };
    },
    [boxOf],
  );

  /** A drawn box with snapping; Shift makes it square (or a line in 45° steps). */
  const resolveDraw = useCallback((g: Extract<Gesture, { kind: "draw" }>) => {
    let c = g.cur;
    let guides: Guide[] = [];
    if (!g.mods.free) ({ p: c, guides } = snapPoint(c, g.snap));
    const a = g.start;
    if (g.mods.shift) {
      if (g.shape === "line") c = steps45(a, c);
      else {
        const k = Math.max(Math.abs(c[0] - a[0]), Math.abs(c[1] - a[1]));
        c = [a[0] + Math.sign(c[0] - a[0] || 1) * k, a[1] + Math.sign(c[1] - a[1] || 1) * k];
      }
    }
    return { a, c, guides };
  }, []);

  /** A rotation about the shape's centre; it settles on right angles, or 15° steps with Shift. */
  const resolveRotate = useCallback(
    (g: Extract<Gesture, { kind: "rotate" }>) => {
      const b = boxOf(g.id);
      if (!b) return null;
      const c = centreOf(b);
      const turn = Math.atan2(g.cur[1] - c[1], g.cur[0] - c[0]) - Math.atan2(g.start[1] - c[1], g.start[0] - c[0]);
      let angle = normalAngle((b.angle ?? 0) + turn);
      const [step, tol] = g.mods.shift ? [15, 7.5] : [45, 2];
      const deg = (angle * 180) / Math.PI;
      const near = Math.round(deg / step) * step;
      if (Math.abs(deg - near) <= tol) angle = normalAngle((near * Math.PI) / 180);
      const delta = angle - (b.angle ?? 0);
      const frame = corners(b).map(([x, y]) => {
        const [dx, dy] = [x - c[0], y - c[1]];
        return [c[0] + dx * Math.cos(delta) - dy * Math.sin(delta), c[1] + dx * Math.sin(delta) + dy * Math.cos(delta)] as Pt;
      });
      return { b, angle, frame };
    },
    [boxOf],
  );

  const overlay = useCallback((): Overlay => {
    const g = gesture.current;
    const v = view.current;
    const selected = sel.current.map(boxOf).filter((b) => b !== null);
    const o: Overlay = { selected: selected.map(outline), handles: [], arrows: arrows() };
    if (!g && selected.length === 1) {
      o.handles = handles(selected[0]);
      if (v && tool === "select" && !isOneD(selected[0])) o.rotate = rotateHandle(selected[0], v, H);
    }
    const label = (text: string) => (o.readout = { at: [pointer.current[0] + 18, pointer.current[1] + 24], text });
    if (g?.kind === "move") {
      const { d, guides, measures } = resolveMove(g);
      const moving = selected.filter((b) => g.ids.includes(b.id));
      o.selected = moving.map((b) => outline(b).map(([x, y]) => [x + d[0], y + d[1]] as Pt));
      o.guides = guides;
      o.measures = measures;
      const one = moving.length === 1 && !isOneD(moving[0]) ? centreOf(moving[0]) : null;
      const text = one ? `X ${num(one[0] + d[0])}  Y ${num(one[1] + d[1])}` : `Δ ${num(d[0])}, ${num(d[1])} in`;
      label(g.mods.alt ? `${text}  + copy` : text);
    } else if (g?.kind === "resize") {
      const r = resolveResize(g);
      if (r && !r.r) {
        o.preview = { kind: "segment", pts: g.handle === 0 ? [r.p, r.b.end!] : [r.b.begin!, r.p] };
        const t = hover.current !== null ? boxOf(hover.current) : null;
        if (r.b.connector && t) o.target = corners(t);
        else o.guides = r.guides;
        const other = g.handle === 0 ? r.b.end! : r.b.begin!;
        label(formatInches(Math.hypot(r.p[0] - other[0], r.p[1] - other[1])));
      } else if (r?.r) {
        o.preview = { kind: "polygon", pts: r.r.frame };
        o.guides = r.guides;
        label(`${num(r.r.bounds.width)} × ${num(r.r.bounds.height)} in`);
      }
    } else if (g?.kind === "rotate") {
      const r = resolveRotate(g);
      if (r) {
        o.selected = [];
        o.preview = { kind: "polygon", pts: r.frame };
        label(degrees(r.angle));
      }
    } else if (g?.kind === "draw") {
      const { a, c, guides } = resolveDraw(g);
      const pts: Pt[] = [a, [c[0], a[1]], c, [a[0], c[1]]];
      o.preview =
        g.shape === "line" ? { kind: "segment", pts: [a, c] } : { kind: g.shape === "ellipse" ? "ellipse" : "polygon", pts };
      o.guides = guides;
      label(
        g.shape === "line"
          ? formatInches(Math.hypot(c[0] - a[0], c[1] - a[1]))
          : `${num(Math.abs(c[0] - a[0]))} × ${num(Math.abs(c[1] - a[1]))} in`,
      );
    } else if (g?.kind === "connect") {
      o.preview = { kind: "segment", pts: [g.start, g.cur] };
      const t = hover.current !== null ? boxOf(hover.current) : null;
      if (t) o.target = corners(t);
    } else if (g?.kind === "marquee" && view.current) {
      const [s0, s1] = [toScreen(view.current, H, g.start), toScreen(view.current, H, g.cur)];
      o.marquee = [s0[0], s0[1], s1[0], s1[1]];
    } else if (!g && tool === "connector" && hover.current !== null) {
      const t = boxOf(hover.current);
      if (t) o.target = corners(t);
    }
    return o;
  }, [arrows, boxOf, H, tool, resolveMove, resolveResize, resolveRotate, resolveDraw]);

  /** Renders now: after a resize has cleared the canvas. Otherwise use `draw`. */
  const paint = useCallback(() => {
    cancelAnimationFrame(frame.current);
    const canvas = canvasRef.current;
    const v = view.current;
    if (!canvas || !v) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio || 1;
    const theme = themeColors(canvas);
    render(ctx, dl, v, dpr, theme, () => (frame.current = requestAnimationFrame(paint)), grid);
    drawOverlay(ctx, v, dpr, H, overlay());
    if (rulers) {
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      drawRulers(ctx, v, dl.page, canvas.clientWidth, canvas.clientHeight, theme.ruler);
    }
    const mini = miniRef.current;
    const mctx = mini?.getContext("2d");
    if (mini && mctx) {
      const [w, h] = [Math.round(mw * dpr), Math.round(mh * dpr)];
      if (mini.width !== w || mini.height !== h) [mini.width, mini.height] = [w, h];
      let cache = miniCache.current;
      if (!cache || cache.dl !== dl || cache.dpr !== dpr) {
        const reload = () => {
          miniCache.current = null;
          frame.current = requestAnimationFrame(paint);
        };
        cache = { dl, dpr, img: renderMiniPage(dl, mw, mh, dpr, theme.raised, reload) };
        miniCache.current = cache;
      }
      drawMinimap(mctx, cache.img, v, [canvas.clientWidth, canvas.clientHeight], miniView(dl.page, mw, mh), dpr, theme.accent);
    }
  }, [dl, H, overlay, grid, rulers, mw, mh]);

  /** Renders on the next frame, coalescing repeated requests. */
  const draw = useCallback(() => {
    cancelAnimationFrame(frame.current);
    frame.current = requestAnimationFrame(paint);
  }, [paint]);

  useEffect(draw, [selection, tool, draw, minimap]);

  const setView = useCallback(
    (v: View) => {
      view.current = v;
      onZoomChange(v.zoom);
      draw();
    },
    [draw, onZoomChange],
  );

  /** Called on a view change the user made (zoom, pan), as opposed to a fit. */
  const manual = useRef(onManualView);
  manual.current = onManualView;

  const zoomAt = useCallback(
    (factor: number, cx: number, cy: number) => {
      const v = view.current;
      if (!v) return;
      const zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, v.zoom * factor));
      const k = zoom / v.zoom;
      manual.current();
      setView({ zoom, ox: cx - (cx - v.ox) * k, oy: cy - (cy - v.oy) * k });
    },
    [setView],
  );

  const inset = rulers ? RULER : 0;
  const fit = useCallback(() => {
    const c = canvasRef.current;
    if (c) setView(fitView(dl.page, c.clientWidth, c.clientHeight, inset));
  }, [dl.page, inset, setView]);

  useImperativeHandle(
    ref,
    () => ({
      fit,
      zoomBy: (f) => {
        const c = canvasRef.current;
        if (c) zoomAt(f, c.clientWidth / 2, c.clientHeight / 2);
      },
      setZoom: (z) => {
        const c = canvasRef.current;
        if (c && view.current) zoomAt(z / view.current.zoom, c.clientWidth / 2, c.clientHeight / 2);
      },
      screenBounds: (id) => {
        const b = boxOf(id);
        const v = view.current;
        if (!b || !v) return null;
        const [x0, y0, x1, y1] = pageBounds(b);
        const [sx0, sy0] = toScreen(v, H, [x0, y1]);
        const [sx1, sy1] = toScreen(v, H, [x1, y0]);
        return [sx0, sy0, sx1 - sx0, sy1 - sy0];
      },
      viewCentre: () => {
        const c = canvasRef.current;
        const v = view.current;
        return c && v ? toPage(v, H, [c.clientWidth / 2, c.clientHeight / 2]) : null;
      },
      zoomTo: ([x0, y0, x1, y1]) => {
        const c = canvasRef.current;
        if (!c) return;
        const pad = 64;
        const [w, h] = [c.clientWidth - inset, c.clientHeight - inset];
        const fitZoom = Math.min((w - 2 * pad) / Math.max(x1 - x0, 0.25), (h - 2 * pad) / Math.max(y1 - y0, 0.25));
        const zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, fitZoom));
        const [cx, cy] = [(x0 + x1) / 2, (y0 + y1) / 2];
        manual.current();
        setView({ zoom, ox: inset + w / 2 - cx * zoom, oy: inset + h / 2 - (H - cy) * zoom });
      },
      expectPageChange: (change) => {
        pageChange.current = change;
      },
    }),
    [fit, zoomAt, boxOf, H, inset, setView],
  );

  // Size the backing store to the element and the device pixel ratio. One
  // observer for the canvas's life: observing again fires it again, and
  // assigning the size clears the canvas, so that happens only on a real
  // change and is repainted in the same frame (no blank flash).
  // Auto-fit follows the window and the side panels (the canvas's width), but
  // not the toolbar wrapping onto a second row when a shape is selected (a
  // height-only change), which must not re-zoom the drawing.
  const latest = useRef({ fit, paint, autoFit });
  latest.current = { fit, paint, autoFit };
  useEffect(() => {
    const canvas = canvasRef.current!;
    let windowSize = `${window.innerWidth}x${window.innerHeight}`;
    let width = canvas.clientWidth;
    const ro = new ResizeObserver(() => {
      const dpr = window.devicePixelRatio || 1;
      const w = Math.max(1, Math.round(canvas.clientWidth * dpr));
      const h = Math.max(1, Math.round(canvas.clientHeight * dpr));
      if (canvas.width === w && canvas.height === h && view.current) return;
      canvas.width = w;
      canvas.height = h;
      const { fit, paint, autoFit } = latest.current;
      const now = `${window.innerWidth}x${window.innerHeight}`;
      if (!view.current || (autoFit && (now !== windowSize || canvas.clientWidth !== width))) fit();
      windowSize = now;
      width = canvas.clientWidth;
      paint();
    });
    ro.observe(canvas);
    return () => ro.disconnect();
  }, []);

  // Turning auto-fit on, or the rulers on or off while it is on, fits at once.
  const fitRef = useRef(fit);
  fitRef.current = fit;
  useEffect(() => {
    if (autoFit) fitRef.current();
  }, [autoFit, inset]);

  // Fit when a different page arrives; just redraw when the same page updates.
  // When the page changes size (page setup, Auto Size), fit it if asked to,
  // or else keep the shapes still: the view is anchored at the page's top-left
  // corner, so a taller page or shapes the engine moved would otherwise shift.
  useEffect(() => {
    const [w0, h0] = lastSize.current;
    lastSize.current = [dl.page.width, dl.page.height];
    const change = pageChange.current;
    pageChange.current = null;
    const v = view.current;
    if (lastPage.current !== dl.page.index || !v) {
      lastPage.current = dl.page.index;
      fit();
    } else if (dl.page.width !== w0 || dl.page.height !== h0 || change?.moved) {
      if (change?.fit) {
        fit();
      } else {
        const [dx, dy] = change?.moved ?? [0, 0];
        view.current = { ...v, ox: v.ox - dx * v.zoom, oy: v.oy - (dl.page.height - h0 - dy) * v.zoom };
        draw();
      }
    } else {
      draw();
    }
  }, [dl, draw, fit]);

  // Wheel: scroll pans, Ctrl/⌘ + wheel (and trackpad pinch) zooms at the cursor.
  useEffect(() => {
    const canvas = canvasRef.current!;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const v = view.current;
      if (!v) return;
      const rect = canvas.getBoundingClientRect();
      if (e.ctrlKey || e.metaKey) {
        zoomAt(Math.exp(-e.deltaY * 0.0015), e.clientX - rect.left, e.clientY - rect.top);
      } else {
        const dx = e.shiftKey && !e.deltaX ? e.deltaY : e.deltaX;
        const dy = e.shiftKey && !e.deltaX ? 0 : e.deltaY;
        manual.current();
        setView({ ...v, ox: v.ox - dx, oy: v.oy - dy });
      }
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [setView, zoomAt]);

  // Space held = pan with the left button, shown by a grab hand. In a text field a space is typed.
  useEffect(() => {
    const typing = (t: EventTarget | null) =>
      t instanceof HTMLTextAreaElement || t instanceof HTMLInputElement || t instanceof HTMLSelectElement || (t instanceof HTMLElement && t.isContentEditable);
    const release = () => {
      if (!space.current) return;
      space.current = false;
      // The next pointer move restores the hover cursor; a pan in progress keeps its hand until it ends.
      if (gesture.current?.kind !== "pan" && canvasRef.current) canvasRef.current.style.cursor = "default";
    };
    const down = (e: KeyboardEvent) => {
      if (e.code !== "Space" || space.current || typing(e.target)) return;
      space.current = true;
      if (canvasRef.current && !gesture.current) canvasRef.current.style.cursor = "grab";
    };
    const up = (e: KeyboardEvent) => {
      if (e.code === "Space") release();
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    // Switching away while space is held never delivers its keyup.
    window.addEventListener("blur", release);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", release);
    };
  }, []);

  const local = (e: { clientX: number; clientY: number }): Pt => {
    const r = canvasRef.current!.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top];
  };

  /** The shape under a page point other than `except` (connector targets skip connectors). */
  const targetAt = (p: Pt, except: number | null): ShapeBox | null => {
    const v = view.current;
    if (!v) return null;
    const candidates = dl.shapes.filter((b) => b.id !== except && !isOneD(b));
    return hitShape(candidates, p, SLACK / v.zoom);
  };

  const onPointerDown = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const v = view.current;
    if (!v) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    e.currentTarget.focus();
    const sp = local(e);
    const p = toPage(v, H, sp);
    const mods = modsOf(e);
    pointer.current = sp;
    if (e.button === 1 || space.current) {
      manual.current();
      gesture.current = { kind: "pan", x: e.clientX, y: e.clientY };
      e.currentTarget.style.cursor = "grabbing";
      return;
    }
    // The rulers are not part of the drawing.
    if (e.button !== 0 || (rulers && (sp[0] < RULER || sp[1] < RULER))) return;
    if (tool === "connector") {
      const from = targetAt(p, null);
      gesture.current = { kind: "connect", from: from?.id ?? null, start: from ? centre(from) : p, cur: p };
      draw();
      return;
    }
    if (tool !== "select") {
      const s = makeSnap([]);
      const start = mods.free ? p : snapPoint(p, s).p;
      gesture.current = { kind: "draw", shape: tool, start, cur: start, mods, snap: s };
      return;
    }
    // The rotate handle of the single selected shape?
    const one = sel.current.length === 1 ? boxOf(sel.current[0]) : null;
    if (one && !isOneD(one) && overRotate(one, sp)) {
      gesture.current = { kind: "rotate", id: one.id, start: p, cur: p, mods };
      draw();
      return;
    }
    // AutoConnect arrows of the selected shape.
    const dir = hitArrow(arrows(), sp);
    if (dir && sel.current.length === 1) {
      onCommand({ kind: "autoConnect", page, shape: sel.current[0], direction: dir });
      hotArrow.current = null;
      return;
    }
    // A resize handle of the single selected shape?
    if (sel.current.length === 1) {
      const b = boxOf(sel.current[0]);
      if (b) {
        const i = handles(b).findIndex((h) => {
          const [hx, hy] = toScreen(v, H, h);
          return Math.abs(hx - sp[0]) <= SLACK && Math.abs(hy - sp[1]) <= SLACK;
        });
        if (i >= 0) {
          gesture.current = { kind: "resize", id: b.id, handle: i, cur: p, mods, snap: makeSnap([b.id]) };
          draw();
          return;
        }
      }
    }
    const hit = hitShape(dl.shapes, p, SLACK / v.zoom);
    if (hit) {
      let ids = sel.current;
      if (e.shiftKey) {
        ids = ids.includes(hit.id) ? ids.filter((i) => i !== hit.id) : [...ids, hit.id];
        onSelection(ids);
      } else if (!ids.includes(hit.id)) {
        ids = [hit.id];
        onSelection(ids);
      }
      gesture.current = ids.includes(hit.id) ? { kind: "move", start: p, cur: p, ids, mods, snap: makeSnap(ids) } : null;
    } else {
      if (!e.shiftKey) onSelection([]);
      gesture.current = { kind: "marquee", start: p, cur: p, additive: e.shiftKey };
    }
    draw();
  };

  const onPointerMove = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const v = view.current;
    const g = gesture.current;
    const canvas = e.currentTarget;
    if (!v) return;
    const sp = local(e);
    const p = toPage(v, H, sp);
    pointer.current = sp;
    if (!g) {
      if (space.current) {
        canvas.style.cursor = "grab";
        return;
      }
      // Hover feedback: arrows, connector targets, cursors.
      const one = tool === "select" && sel.current.length === 1 ? boxOf(sel.current[0]) : null;
      if (one && !isOneD(one) && overRotate(one, sp)) {
        canvas.style.cursor = "grab";
        return;
      }
      const dir = hitArrow(arrows(), sp);
      const changed = dir !== hotArrow.current;
      hotArrow.current = dir;
      if (tool === "connector") {
        const t = targetAt(p, null)?.id ?? null;
        if (t !== hover.current) {
          hover.current = t;
          draw();
        }
        canvas.style.cursor = "crosshair";
        return;
      }
      if (changed) draw();
      const over = tool === "select" && hitShape(dl.shapes, p, SLACK / v.zoom);
      canvas.style.cursor = dir ? "pointer" : tool === "text" ? "text" : tool !== "select" ? "crosshair" : over ? "move" : "default";
      return;
    }
    if (g.kind === "pan") {
      setView({ ...v, ox: v.ox + e.clientX - g.x, oy: v.oy + e.clientY - g.y });
      gesture.current = { ...g, x: e.clientX, y: e.clientY };
      return;
    }
    if (g.kind === "connect" || g.kind === "resize") {
      const self = g.kind === "resize" ? g.id : g.from;
      hover.current = targetAt(p, self)?.id ?? null;
    }
    gesture.current = "mods" in g ? { ...g, cur: p, mods: modsOf(e) } : { ...g, cur: p };
    canvas.style.cursor = g.kind === "rotate" ? "grabbing" : g.kind === "move" && e.altKey ? "copy" : canvas.style.cursor;
    draw();
  };

  /** Is a screen point on the rotate handle of `b`? */
  const overRotate = (b: ShapeBox, sp: Pt) => {
    const v = view.current;
    if (!v || tool !== "select") return false;
    const { at } = rotateHandle(b, v, H);
    return Math.hypot(at[0] - sp[0], at[1] - sp[1]) <= 8;
  };

  const onPointerUp = () => {
    const v = view.current;
    const g = gesture.current;
    gesture.current = null;
    const target = hover.current;
    hover.current = null;
    if (g?.kind === "pan" && canvasRef.current) canvasRef.current.style.cursor = space.current ? "grab" : "default";
    if (!v || !g) return;
    const px = (a: Pt, b: Pt) => Math.hypot(a[0] - b[0], a[1] - b[1]) * v.zoom;
    if (g.kind === "move" && px(g.start, g.cur) > CLICK / 2) {
      const { d } = resolveMove(g);
      if (d[0] === 0 && d[1] === 0) {
        // Snapped back to where it was.
      } else if (g.mods.alt) onDuplicate(g.ids, d[0], d[1]);
      else onCommand({ kind: "moveShapes", page, shapes: g.ids, dx: d[0], dy: d[1] });
    } else if (g.kind === "resize") {
      const r = resolveResize(g);
      if (r && !r.r) {
        const b = r.b;
        const [bg, en] = g.handle === 0 ? [r.p, b.end!] : [b.begin!, r.p];
        // A connector end dropped on a shape glues to it; elsewhere it comes free.
        const beginGlue = g.handle === 0 ? (target ?? undefined) : b.beginGlue;
        const endGlue = g.handle === 1 ? (target ?? undefined) : b.endGlue;
        onCommand({ kind: "setEndpoints", page, shape: b.id, beginX: bg[0], beginY: bg[1], endX: en[0], endY: en[1], beginGlue, endGlue });
      } else if (r?.r) {
        onCommand({ kind: "setBounds", page, shape: r.b.id, ...r.r.bounds });
      }
    } else if (g.kind === "rotate") {
      const r = resolveRotate(g);
      if (r && Math.abs(r.angle - (r.b.angle ?? 0)) > 1e-9) {
        onCommand({ kind: "setCells", page, shapes: [r.b.id], cells: [{ cell: "Angle", value: String(r.angle) }] });
      }
    } else if (g.kind === "connect") {
      if (px(g.start, g.cur) > CLICK || target !== null) {
        const end = target !== null ? centre(boxOf(target)!) : g.cur;
        onCommand({ kind: "addConnector", page, from: g.from, to: target, x0: g.start[0], y0: g.start[1], x1: end[0], y1: end[1] });
      }
    } else if (g.kind === "draw") {
      const { a, c } = resolveDraw(g);
      const dragged = px(g.start, g.cur) > CLICK;
      if (g.shape === "line") {
        if (dragged) onCommand({ kind: "addShape", page, shape: "line", x: a[0], y: a[1], w: c[0] - a[0], h: c[1] - a[1] });
      } else if (dragged) {
        const [x, y] = [Math.min(a[0], c[0]), Math.min(a[1], c[1])];
        onCommand({ kind: "addShape", page, shape: g.shape, x, y, w: Math.abs(c[0] - a[0]), h: Math.abs(c[1] - a[1]) });
      } else {
        // A click drops a default-sized shape centred on the pointer.
        const [w, h] = g.shape === "text" ? [1.5, 0.4] : [1.5, 1];
        onCommand({ kind: "addShape", page, shape: g.shape, x: a[0] - w / 2, y: a[1] - h / 2, w, h });
      }
    } else if (g.kind === "marquee" && px(g.start, g.cur) > CLICK) {
      const [x0, x1] = [Math.min(g.start[0], g.cur[0]), Math.max(g.start[0], g.cur[0])];
      const [y0, y1] = [Math.min(g.start[1], g.cur[1]), Math.max(g.start[1], g.cur[1])];
      const inside = dl.shapes
        .filter((b) => {
          const [bx0, by0, bx1, by1] = pageBounds(b);
          return bx0 >= x0 && bx1 <= x1 && by0 >= y0 && by1 <= y1;
        })
        .map((b) => b.id);
      onSelection(g.additive ? [...new Set([...sel.current, ...inside])] : inside);
    }
    draw();
  };

  /** Centres the main view on the page point under a minimap pointer. */
  const jump = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const v = view.current;
    const c = canvasRef.current;
    if (!v || !c) return;
    const r = e.currentTarget.getBoundingClientRect();
    const mv = miniView(dl.page, mw, mh);
    const x = (e.clientX - r.left - mv.ox) / mv.zoom;
    const fromTop = (e.clientY - r.top - mv.oy) / mv.zoom;
    manual.current();
    setView({ ...v, ox: c.clientWidth / 2 - x * v.zoom, oy: c.clientHeight / 2 - fromTop * v.zoom });
  };

  return (
    <>
    {minimap && (
      <canvas
        ref={miniRef}
        className="minimap"
        style={{ width: mw, height: mh }}
        aria-label="Minimap: click or drag to move the view"
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          jump(e);
        }}
        onPointerMove={(e) => {
          if (e.buttons & 1) jump(e);
        }}
      />
    )}
    <canvas
      ref={canvasRef}
      className={`drawing-canvas${dropping ? " dropping" : ""}`}
      tabIndex={0}
      aria-label={`Page ${dl.page.name}`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerLeave={() => {
        if (hotArrow.current || hover.current !== null) {
          hotArrow.current = null;
          if (!gesture.current) hover.current = null;
          draw();
        }
      }}
      onPointerCancel={(e) => {
        if (gesture.current?.kind === "pan") e.currentTarget.style.cursor = space.current ? "grab" : "default";
        gesture.current = null;
        draw();
      }}
      onDoubleClick={(e) => {
        const v = view.current;
        if (!v) return;
        const sp = local(e);
        if (rulers && (sp[0] < RULER || sp[1] < RULER)) return;
        const p = toPage(v, H, sp);
        const hit = tool === "select" ? hitShape(dl.shapes, p, SLACK / v.zoom) : null;
        if (hit) onEditText(hit.id);
        else if (tool === "select") onQuickAdd([e.clientX, e.clientY], p);
      }}
      onDragOver={(e) => {
        if (!e.dataTransfer.types.includes(SHAPE_MIME)) return;
        e.preventDefault();
        e.stopPropagation();
        e.dataTransfer.dropEffect = "copy";
        setDropping(true);
      }}
      onDragLeave={() => setDropping(false)}
      onDrop={(e) => {
        const id = e.dataTransfer.getData(SHAPE_MIME);
        const v = view.current;
        setDropping(false);
        if (!id || !v) return;
        e.preventDefault();
        e.stopPropagation();
        onDropShape(id, toPage(v, H, local(e)));
      }}
    />
    </>
  );
}
