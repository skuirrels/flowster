// Canvas 2D renderer for engine display lists (SPEC §8.6, P0 backend).
// Page coordinates are inches with +Y up; the view maps them to CSS pixels.

import type { Affine, DisplayList, Item, Paragraph, Run, Seg } from "../engine/protocol";
import { css, fillPaint, mul } from "./paint";
import { drawGrid } from "./rulers";

export interface View {
  /** CSS pixels per inch. */
  zoom: number;
  /** CSS-pixel position of the page's top-left corner. */
  ox: number;
  oy: number;
}

/** Text frames use 1/100 inch units so fonts never get sub-pixel sizes. */
const TEXT_UNITS = 100;
const FONT_FALLBACKS = `Calibri, Carlito, "Segoe UI", "Helvetica Neue", Arial, sans-serif`;


const paths = new WeakMap<Seg[], Path2D>();
function toPath(d: Seg[]): Path2D {
  let p = paths.get(d);
  if (p) return p;
  p = new Path2D();
  for (const s of d) {
    if (s.k === "M") p.moveTo(s.x, s.y);
    else if (s.k === "L") p.lineTo(s.x, s.y);
    else if (s.k === "C") p.bezierCurveTo(s.x1, s.y1, s.x2, s.y2, s.x, s.y);
    else p.closePath();
  }
  paths.set(d, p);
  return p;
}

const images = new Map<string, HTMLImageElement>();
function imageElement(mime: string, data: string, onLoad?: () => void): HTMLImageElement {
  const key = `${mime}:${data.length}:${data.slice(0, 64)}:${data.slice(-64)}`;
  let img = images.get(key);
  if (!img) {
    img = new Image();
    if (onLoad) img.onload = onLoad;
    img.src = `data:${mime};base64,${data}`;
    images.set(key, img);
  }
  return img;
}

function image(mime: string, data: string, onLoad: () => void): HTMLImageElement | null {
  const img = imageElement(mime, data, onLoad);
  return img.complete && img.naturalWidth > 0 ? img : null;
}

/** Resolves once the page's pictures are decoded (or failed), so an export draws them. */
export async function preloadImages(dl: DisplayList): Promise<void> {
  await Promise.all(
    dl.items.map((it) => (it.kind === "image" && it.data ? imageElement(it.mime, it.data).decode().catch(() => {}) : null)),
  );
}

/** The view that fits the page in the canvas, less `inset` CSS pixels along the top and left (the rulers). */
export function fitView(page: { width: number; height: number }, cssW: number, cssH: number, inset = 0): View {
  const pad = 32;
  const [w, h] = [cssW - inset, cssH - inset];
  const zoom = Math.max(0.01, Math.min((w - 2 * pad) / page.width, (h - 2 * pad) / page.height));
  return { zoom, ox: inset + (w - page.width * zoom) / 2, oy: inset + (h - page.height * zoom) / 2 };
}

export interface Theme {
  pasteboard: string;
  pageShadow: string;
}

/** Draws the page, its grid when asked, and its display list. `dpr` is the canvas backing-store scale. */
export function render(
  ctx: CanvasRenderingContext2D,
  dl: DisplayList,
  view: View,
  dpr: number,
  theme: Theme,
  onAsyncLoad: () => void,
  grid = false,
): void {
  const { width: W, height: H } = dl.page;
  const cw = ctx.canvas.width;
  const ch = ctx.canvas.height;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.fillStyle = theme.pasteboard;
  ctx.fillRect(0, 0, cw, ch);

  // Paper.
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.save();
  ctx.shadowColor = theme.pageShadow;
  ctx.shadowBlur = 8 * dpr;
  ctx.shadowOffsetY = 2 * dpr;
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(view.ox, view.oy, W * view.zoom, H * view.zoom);
  ctx.restore();
  if (grid) drawGrid(ctx, view, dl.page, cw / dpr, ch / dpr);

  // Page inches (+Y up) → device pixels.
  const z = view.zoom;
  const pageToDevice: Affine = [z * dpr, 0, 0, -z * dpr, view.ox * dpr, (view.oy + H * z) * dpr];
  const onePx = 1 / (z * dpr);

  for (const item of dl.items) {
    drawItem(ctx, item, pageToDevice, onePx, onAsyncLoad);
  }
}

function drawItem(ctx: CanvasRenderingContext2D, item: Item, pageToDevice: Affine, onePx: number, onAsyncLoad: () => void) {
  if (item.kind === "path") {
    const p = toPath(item.d);
    // Shadows carry a blur radius in inches; canvas blur() takes a device-pixel radius.
    const blurred = (item.blur ?? 0) > 0;
    if (blurred) ctx.filter = `blur(${((item.blur ?? 0) / 2 / onePx).toFixed(2)}px)`;
    if (item.fill) fillPaint(ctx, p, item.fill, pageToDevice);
    ctx.setTransform(...pageToDevice);
    if (item.stroke) {
      const s = item.stroke;
      ctx.strokeStyle = css(s.color);
      ctx.lineWidth = Math.max(s.width, onePx);
      ctx.lineCap = s.cap;
      ctx.lineJoin = "round";
      ctx.setLineDash(s.dash);
      ctx.stroke(p);
      ctx.setLineDash([]);
    }
    if (blurred) ctx.filter = "none";
  } else if (item.kind === "image") {
    // Images are top-down; flip inside the shape frame.
    const m = mul(pageToDevice, mul(item.transform, [1, 0, 0, -1, item.x, item.y + item.h]));
    ctx.setTransform(...m);
    const img = item.data ? image(item.mime, item.data, onAsyncLoad) : null;
    if (img) {
      ctx.drawImage(img, 0, 0, item.w, item.h);
    } else {
      ctx.fillStyle = "#eeeeee";
      ctx.fillRect(0, 0, item.w, item.h);
      ctx.strokeStyle = "#999999";
      ctx.lineWidth = onePx;
      ctx.strokeRect(0, 0, item.w, item.h);
    }
  } else {
    drawText(ctx, item, pageToDevice);
  }
}

type TextItem = Extract<Item, { kind: "text" }>;

interface Piece {
  run: Run;
  text: string;
  width: number;
}

interface Line {
  para: Paragraph;
  pieces: Piece[];
  width: number;
  height: number;
  ascent: number;
  first: boolean;
}

function fontFor(run: Run): string {
  const style = `${run.italic ? "italic " : ""}${run.bold ? "bold " : ""}`;
  return `${style}${run.size * TEXT_UNITS}px "${run.font}", ${FONT_FALLBACKS}`;
}

/** Sets a run's font. Visio does not apply pair kerning, so neither do we. */
function setFont(ctx: CanvasRenderingContext2D, run: Run): void {
  ctx.font = fontFor(run);
  ctx.fontKerning = "none";
}

function measure(ctx: CanvasRenderingContext2D, run: Run, text: string): number {
  setFont(ctx, run);
  return ctx.measureText(text).width;
}

/**
 * Where a line's text starts (text units): the left indent, and on the first
 * line the first-line indent, moved past the bullet when there is one (a
 * hanging indent leaves room for it already).
 */
function lineIndent(ctx: CanvasRenderingContext2D, para: Paragraph, first: boolean): number {
  const u = TEXT_UNITS;
  if (!first) return para.indentLeft * u;
  const at = (para.indentFirst + para.indentLeft) * u;
  if (!para.bullet || !para.runs[0]) return at;
  if (para.textAfterBullet) return at + para.textAfterBullet * u;
  return Math.max(para.indentLeft * u, at + 2 * measure(ctx, para.runs[0], para.bullet));
}

/** Where a first line's bullet goes, relative to where its text starts (text units). */
function bulletOffset(ctx: CanvasRenderingContext2D, para: Paragraph): number {
  return (para.indentFirst + para.indentLeft) * TEXT_UNITS - lineIndent(ctx, para, true);
}

/** Greedy word wrap of one paragraph into lines no wider than `maxW` (text units). */
function layoutParagraph(ctx: CanvasRenderingContext2D, para: Paragraph, maxW: number): Line[] {
  const lines: Line[] = [];
  let cur: Line | null = null;
  const lineHeight = (size: number) =>
    (para.lineSpacing < 0 ? size * -para.lineSpacing : Math.max(para.lineSpacing, size)) * TEXT_UNITS;
  const newLine = (): Line => {
    const l: Line = { para, pieces: [], width: 0, height: 0, ascent: 0, first: lines.length === 0 };
    lines.push(l);
    return l;
  };
  for (const run of para.runs) {
    // Keep spaces attached to the following word so wrapped lines start clean.
    // Like Visio, a line may also break after a slash or a dash ("PO/" | "DO").
    const tokens = run.text.split(/(?=\s)|(?<=[/\-–—])(?=[^\s/\-–—])/u);
    for (const token of tokens) {
      const parts = token.split("\n");
      parts.forEach((part, i) => {
        if (i > 0 || !cur) cur = newLine();
        if (!part) return;
        const w = measure(ctx, run, part);
        const line: Line = cur!;
        const indent = lineIndent(ctx, para, line.first);
        if (line.pieces.length > 0 && line.width + w > maxW - indent) {
          const trimmed = part.replace(/^\s+/u, "");
          cur = newLine();
          cur.pieces.push({ run, text: trimmed, width: measure(ctx, run, trimmed) });
          cur.width += cur.pieces[0].width;
        } else {
          line.pieces.push({ run, text: part, width: w });
          line.width += w;
        }
      });
    }
  }
  if (lines.length === 0) newLine();
  const fallbackSize = para.runs[0]?.size ?? 1 / 6;
  for (const l of lines) {
    const size = l.pieces.reduce((m, p) => Math.max(m, p.run.size), 0) || fallbackSize;
    l.height = lineHeight(size);
    l.ascent = size * TEXT_UNITS * 0.8 + (l.height - size * TEXT_UNITS) / 2;
  }
  return lines;
}

function drawText(ctx: CanvasRenderingContext2D, t: TextItem, pageToDevice: Affine) {
  const u = TEXT_UNITS;
  // Block-local (inches, +Y up) → frame (1/100 in, +Y down from the block's top-left).
  const frame: Affine = [1 / u, 0, 0, -1 / u, 0, t.height];
  ctx.setTransform(...mul(pageToDevice, mul(t.transform, frame)));
  const [ml, mr, mt, mb] = t.margins.map((m) => m * u);
  const w = t.width * u;
  const h = t.height * u;
  const maxW = Math.max(1, w - ml - mr);

  const lines: Line[] = [];
  for (const para of t.paragraphs) {
    const ls = layoutParagraph(ctx, para, maxW);
    if (ls.length) {
      ls[0].height += para.spaceBefore * u;
      ls[ls.length - 1].height += para.spaceAfter * u;
    }
    lines.push(...ls);
  }
  const total = lines.reduce((s, l) => s + l.height, 0);
  const avail = h - mt - mb;
  let y = mt + (t.valign === 0 ? 0 : t.valign === 2 ? avail - total : (avail - total) / 2);

  // The layout above positions every piece by its left edge; set that here
  // rather than trust what the last frame's overlay left in the context.
  ctx.textBaseline = "alphabetic";
  ctx.textAlign = "left";
  ctx.direction = "ltr";
  for (const line of lines) {
    const indent = lineIndent(ctx, line.para, line.first);
    const free = maxW - indent - line.width;
    let x = ml + indent + (line.para.align === 2 ? free : line.para.align === 1 ? free / 2 : 0);
    if (t.background && line.width > 0) {
      ctx.fillStyle = css(t.background);
      ctx.fillRect(x - 2, y, line.width + 4, line.height);
    }
    const baseline = y + line.ascent;
    const first = line.para.runs[0];
    if (line.first && line.para.bullet && first) {
      setFont(ctx, first);
      ctx.fillStyle = css(first.color);
      ctx.fillText(line.para.bullet, x + bulletOffset(ctx, line.para), baseline);
    }
    for (const p of line.pieces) {
      setFont(ctx, p.run);
      ctx.fillStyle = css(p.run.color);
      ctx.fillText(p.text, x, baseline);
      if (p.run.underline || p.run.strike) {
        const lw = Math.max(1, p.run.size * u * 0.06);
        const yy = p.run.underline ? baseline + lw * 1.5 : baseline - p.run.size * u * 0.3;
        ctx.fillRect(x, yy, p.width, lw);
      }
      x += p.width;
    }
    y += line.height;
  }
}
