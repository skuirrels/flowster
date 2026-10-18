// The drawing surface: owns the view transform (zoom/pan) and redraws the
// display list on demand. React renders the frame; drawing is imperative.

import { useCallback, useEffect, useImperativeHandle, useRef, type Ref } from "react";

import type { DisplayList } from "../engine/protocol";
import { fitView, render, type Theme, type View } from "./renderer";

export interface CanvasHandle {
  fit(): void;
  zoomBy(factor: number): void;
  setZoom(pxPerInch: number): void;
}

const MIN_ZOOM = 2;
const MAX_ZOOM = 20000;
/** CSS pixels per inch at 100 %. */
export const PX_PER_INCH = 96;

function themeColors(el: HTMLElement): Theme {
  const s = getComputedStyle(el);
  return {
    pasteboard: s.getPropertyValue("--pasteboard").trim() || "#e6e8ec",
    pageShadow: s.getPropertyValue("--page-shadow").trim() || "rgba(0,0,0,0.25)",
  };
}

export function CanvasView({
  dl,
  onZoomChange,
  ref,
}: {
  dl: DisplayList;
  onZoomChange: (pxPerInch: number) => void;
  ref?: Ref<CanvasHandle>;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const view = useRef<View | null>(null);
  const frame = useRef(0);
  const drag = useRef<{ x: number; y: number } | null>(null);
  const lastPage = useRef<number | null>(null);

  const draw = useCallback(() => {
    cancelAnimationFrame(frame.current);
    frame.current = requestAnimationFrame(() => {
      const canvas = canvasRef.current;
      const v = view.current;
      if (!canvas || !v) return;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      render(ctx, dl, v, window.devicePixelRatio || 1, themeColors(canvas), draw);
    });
  }, [dl]);

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
    }),
    [fit, zoomAt],
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

  return (
    <canvas
      ref={canvasRef}
      className={`drawing-canvas${drag.current ? " panning" : ""}`}
      tabIndex={0}
      aria-label={`Page ${dl.page.name}`}
      onPointerDown={(e) => {
        (e.target as HTMLCanvasElement).setPointerCapture(e.pointerId);
        drag.current = { x: e.clientX, y: e.clientY };
      }}
      onPointerMove={(e) => {
        const d = drag.current;
        const v = view.current;
        if (!d || !v) return;
        setView({ ...v, ox: v.ox + e.clientX - d.x, oy: v.oy + e.clientY - d.y });
        drag.current = { x: e.clientX, y: e.clientY };
      }}
      onPointerUp={() => {
        drag.current = null;
      }}
      onDoubleClick={fit}
    />
  );
}
