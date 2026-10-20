// The minimap: the whole page in miniature in a corner of the canvas, with
// the part in view outlined; clicking or dragging on it moves the view there.
// The page picture is cached and redrawn only when the drawing changes.

import type { DisplayList } from "../engine/protocol";
import { render, type View } from "./renderer";

/** Largest minimap size, CSS pixels. */
const MAX_W = 184;
const MAX_H = 132;
const PAD = 6;

/** The minimap's CSS size for a page. */
export function miniSize(page: { width: number; height: number }): [number, number] {
  const k = Math.min((MAX_W - 2 * PAD) / page.width, (MAX_H - 2 * PAD) / page.height);
  return [Math.round(page.width * k + 2 * PAD), Math.round(page.height * k + 2 * PAD)];
}

/** Page inches → minimap CSS pixels. */
export function miniView(page: { width: number; height: number }, w: number, h: number): View {
  const zoom = Math.min((w - 2 * PAD) / page.width, (h - 2 * PAD) / page.height);
  return { zoom, ox: (w - page.width * zoom) / 2, oy: (h - page.height * zoom) / 2 };
}

/** The page drawn at minimap scale (device pixels), for caching. */
export function renderMiniPage(dl: DisplayList, w: number, h: number, dpr: number, background: string, onAsyncLoad: () => void) {
  const c = document.createElement("canvas");
  c.width = Math.round(w * dpr);
  c.height = Math.round(h * dpr);
  const ctx = c.getContext("2d");
  if (ctx) render(ctx, dl, miniView(dl.page, w, h), dpr, { pasteboard: background, pageShadow: "rgba(0,0,0,0.2)" }, onAsyncLoad);
  return c;
}

/** The cached page plus the outline of what the main view shows. */
export function drawMinimap(
  ctx: CanvasRenderingContext2D,
  page: HTMLCanvasElement,
  main: View,
  mainSize: [number, number],
  mini: View,
  dpr: number,
  accent: string,
) {
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, ctx.canvas.width, ctx.canvas.height);
  ctx.drawImage(page, 0, 0);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  // The main view's corners in page inches (y down from the page top), then in minimap pixels.
  const [x0, x1] = [-main.ox / main.zoom, (mainSize[0] - main.ox) / main.zoom];
  const [t0, t1] = [-main.oy / main.zoom, (mainSize[1] - main.oy) / main.zoom];
  const r = [mini.ox + x0 * mini.zoom, mini.oy + t0 * mini.zoom, (x1 - x0) * mini.zoom, (t1 - t0) * mini.zoom];
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, 0, ctx.canvas.width / dpr, ctx.canvas.height / dpr);
  ctx.clip();
  ctx.globalAlpha = 0.12;
  ctx.fillStyle = accent;
  ctx.fillRect(r[0], r[1], r[2], r[3]);
  ctx.globalAlpha = 1;
  ctx.strokeStyle = accent;
  ctx.lineWidth = 1.5;
  ctx.strokeRect(r[0] + 0.75, r[1] + 0.75, Math.max(0, r[2] - 1.5), Math.max(0, r[3] - 1.5));
  ctx.restore();
}
