// The drawing surface: owns the view transform (zoom/pan), redraws the display
// list on demand, and turns pointer gestures into editing commands (SPEC §9).
// React renders the frame; drawing and gestures are imperative.

import { useCallback, useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";

import { SHAPE_MIME } from "../chrome/ShapesPanel";
import type { Command, DisplayList, ShapeBox, ShapeKind } from "../engine/protocol";
import {
  arrowsFor,
  corners,
  drawOverlay,
  handles,
  hitArrow,
  hitShape,
  isOneD,
  pageBounds,
  resized,
  toPage,
  toScreen,
  type Direction,
  type Overlay,
  type Pt,
} from "./edit";
import { fitView, render, type Theme, type View } from "./renderer";

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
}

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
  | { kind: "move"; start: Pt; cur: Pt; ids: number[] }
  | { kind: "resize"; id: number; handle: number; cur: Pt }
  | { kind: "draw"; shape: ShapeKind; start: Pt; cur: Pt }
  | { kind: "connect"; from: number | null; start: Pt; cur: Pt }
  | { kind: "marquee"; start: Pt; cur: Pt; additive: boolean };

function themeColors(el: HTMLElement): Theme {
  const s = getComputedStyle(el);
  return {
    pasteboard: s.getPropertyValue("--pasteboard").trim() || "#e6e8ec",
    pageShadow: s.getPropertyValue("--page-shadow").trim() || "rgba(0,0,0,0.25)",
  };
}

const centre = (b: ShapeBox): Pt => {
  const [x0, y0, x1, y1] = pageBounds(b);
  return [(x0 + x1) / 2, (y0 + y1) / 2];
};

export function CanvasView({
  dl,
  tool,
  selection,
  onSelection,
  onCommand,
  onEditText,
  onDropShape,
  onZoomChange,
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
  const sel = useRef(selection);
  sel.current = selection;
  const [dropping, setDropping] = useState(false);

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

  const overlay = useCallback((): Overlay => {
    const g = gesture.current;
    const selected = sel.current.map(boxOf).filter((b) => b !== null);
    const o: Overlay = { selected: selected.map(outline), handles: [], arrows: arrows() };
    if (!g && selected.length === 1) o.handles = handles(selected[0]);
    if (g?.kind === "move") {
      const [dx, dy] = [g.cur[0] - g.start[0], g.cur[1] - g.start[1]];
      o.selected = selected.filter((b) => g.ids.includes(b.id)).map((b) => outline(b).map(([x, y]) => [x + dx, y + dy] as Pt));
    } else if (g?.kind === "resize") {
      const b = boxOf(g.id);
      if (b && isOneD(b)) {
        o.preview = { kind: "segment", pts: g.handle === 0 ? [g.cur, b.end!] : [b.begin!, g.cur] };
        const t = hover.current !== null ? boxOf(hover.current) : null;
        if (b.connector && t) o.target = corners(t);
      } else if (b) {
        const r = resized(b, g.handle, g.cur);
        if (r) o.preview = { kind: "polygon", pts: r.frame };
      }
    } else if (g?.kind === "draw") {
      const [a, c] = [g.start, g.cur];
      const pts: Pt[] = [a, [c[0], a[1]], c, [a[0], c[1]]];
      o.preview =
        g.shape === "line" ? { kind: "segment", pts: [a, c] } : { kind: g.shape === "ellipse" ? "ellipse" : "polygon", pts };
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
  }, [arrows, boxOf, H, tool]);

  const draw = useCallback(() => {
    cancelAnimationFrame(frame.current);
    frame.current = requestAnimationFrame(() => {
      const canvas = canvasRef.current;
      const v = view.current;
      if (!canvas || !v) return;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      const dpr = window.devicePixelRatio || 1;
      render(ctx, dl, v, dpr, themeColors(canvas), draw);
      drawOverlay(ctx, v, dpr, H, overlay());
    });
  }, [dl, H, overlay]);

  useEffect(draw, [selection, tool, draw]);

  const setView = useCallback(
    (v: View) => {
      view.current = v;
      onZoomChange(v.zoom);
      draw();
    },
    [draw, onZoomChange],
  );

  const zoomAt = useCallback(
    (factor: number, cx: number, cy: number) => {
      const v = view.current;
      if (!v) return;
      const zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, v.zoom * factor));
      const k = zoom / v.zoom;
      setView({ zoom, ox: cx - (cx - v.ox) * k, oy: cy - (cy - v.oy) * k });
    },
    [setView],
  );

  const fit = useCallback(() => {
    const c = canvasRef.current;
    if (c) setView(fitView(dl.page, c.clientWidth, c.clientHeight));
  }, [dl.page, setView]);

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
    }),
    [fit, zoomAt, boxOf, H],
  );

  // Size the backing store to the element and the device pixel ratio.
  useEffect(() => {
    const canvas = canvasRef.current!;
    const ro = new ResizeObserver(() => {
      const dpr = window.devicePixelRatio || 1;
      canvas.width = Math.max(1, Math.round(canvas.clientWidth * dpr));
      canvas.height = Math.max(1, Math.round(canvas.clientHeight * dpr));
      if (!view.current) fit();
      else draw();
    });
    ro.observe(canvas);
    return () => ro.disconnect();
  }, [draw, fit]);

  // Fit when a different page arrives; just redraw when the same page updates.
  useEffect(() => {
    if (lastPage.current !== dl.page.index || !view.current) {
      lastPage.current = dl.page.index;
      fit();
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
        setView({ ...v, ox: v.ox - dx, oy: v.oy - dy });
      }
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [setView, zoomAt]);

  // Space held = pan with the left button.
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.code === "Space" && !(e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLInputElement)) {
        space.current = true;
      }
    };
    const up = (e: KeyboardEvent) => {
      if (e.code === "Space") space.current = false;
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
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
    if (e.button === 1 || space.current) {
      gesture.current = { kind: "pan", x: e.clientX, y: e.clientY };
      return;
    }
    if (e.button !== 0) return;
    if (tool === "connector") {
      const from = targetAt(p, null);
      gesture.current = { kind: "connect", from: from?.id ?? null, start: from ? centre(from) : p, cur: p };
      draw();
      return;
    }
    if (tool !== "select") {
      gesture.current = { kind: "draw", shape: tool, start: p, cur: p };
      return;
    }
    // AutoConnect arrows of the selected shape.
    const dir = hitArrow(arrows(), sp);
    if (dir && sel.current.length === 1) {
      onCommand({ kind: "autoConnect", page, shape: sel.current[0], direction: dir });
      hotArrow.current = null;
      return;
    }
    // A handle of the single selected shape?
    if (sel.current.length === 1) {
      const b = boxOf(sel.current[0]);
      if (b) {
        const i = handles(b).findIndex((h) => {
          const [hx, hy] = toScreen(v, H, h);
          return Math.abs(hx - sp[0]) <= SLACK && Math.abs(hy - sp[1]) <= SLACK;
        });
        if (i >= 0) {
          gesture.current = { kind: "resize", id: b.id, handle: i, cur: p };
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
      gesture.current = ids.includes(hit.id) ? { kind: "move", start: p, cur: p, ids } : null;
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
    if (!g) {
      // Hover feedback: arrows, connector targets, cursors.
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
    gesture.current = { ...g, cur: p };
    draw();
  };

  const onPointerUp = () => {
    const v = view.current;
    const g = gesture.current;
    gesture.current = null;
    const target = hover.current;
    hover.current = null;
    if (!v || !g) return;
    const px = (a: Pt, b: Pt) => Math.hypot(a[0] - b[0], a[1] - b[1]) * v.zoom;
    if (g.kind === "move" && px(g.start, g.cur) > CLICK / 2) {
      onCommand({ kind: "moveShapes", page, shapes: g.ids, dx: g.cur[0] - g.start[0], dy: g.cur[1] - g.start[1] });
    } else if (g.kind === "resize") {
      const b = boxOf(g.id);
      if (b && isOneD(b)) {
        const [bg, en] = g.handle === 0 ? [g.cur, b.end!] : [b.begin!, g.cur];
        // A connector end dropped on a shape glues to it; elsewhere it comes free.
        const beginGlue = g.handle === 0 ? (target ?? undefined) : b.beginGlue;
        const endGlue = g.handle === 1 ? (target ?? undefined) : b.endGlue;
        onCommand({ kind: "setEndpoints", page, shape: b.id, beginX: bg[0], beginY: bg[1], endX: en[0], endY: en[1], beginGlue, endGlue });
      } else if (b) {
        const r = resized(b, g.handle, g.cur);
        if (r) onCommand({ kind: "setBounds", page, shape: b.id, ...r.bounds });
      }
    } else if (g.kind === "connect") {
      if (px(g.start, g.cur) > CLICK || target !== null) {
        const end = target !== null ? centre(boxOf(target)!) : g.cur;
        onCommand({ kind: "addConnector", page, from: g.from, to: target, x0: g.start[0], y0: g.start[1], x1: end[0], y1: end[1] });
      }
    } else if (g.kind === "draw") {
      const [a, c] = [g.start, g.cur];
      const dragged = px(a, c) > CLICK;
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

  return (
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
      onPointerCancel={() => {
        gesture.current = null;
        draw();
      }}
      onDoubleClick={(e) => {
        const v = view.current;
        if (!v) return;
        const hit = tool === "select" ? hitShape(dl.shapes, toPage(v, H, local(e)), SLACK / v.zoom) : null;
        if (hit) onEditText(hit.id);
        else fit();
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
  );
}
