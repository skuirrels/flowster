// The page grid and the rulers along the top and left of the canvas. Both
// measure page inches from the page's bottom-left corner, as Visio's do, and
// pick their spacing from the zoom so lines stay a readable distance apart.

import type { View } from "./renderer";

/** Ruler thickness, CSS pixels. */
export const RULER = 20;

/** Candidate major spacings, inches. */
const STEPS = [1 / 16, 1 / 8, 1 / 4, 1 / 2, 1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000];

/**
 * Major spacing (inches) at least `minPx` apart on screen, and how many
 * minor divisions fit between majors at least `minorPx` apart.
 */
export function spacing(zoom: number, minPx: number, minorPx: number): { major: number; divisions: number } {
  const major = STEPS.find((s) => s * zoom >= minPx) ?? STEPS[STEPS.length - 1];
  // Fives split into fives; everything else into quarters or halves.
  const options = String(major).startsWith("5") ? [5] : [4, 2];
  const divisions = options.find((d) => (major / d) * zoom >= minorPx) ?? 1;
  return { major, divisions };
}

export interface RulerTheme {
  background: string;
  text: string;
  tick: string;
  border: string;
  page: string;
}

/** Grid lines over the paper, under the drawing. Expects a CSS-pixel transform. */
export function drawGrid(ctx: CanvasRenderingContext2D, view: View, page: { width: number; height: number }, cssW: number, cssH: number) {
  const { zoom: z, ox, oy } = view;
  const { major, divisions } = spacing(z, 48, 12);
  const minor = major / divisions;
  // The visible part of the page, in page inches (x right, y up from the bottom).
  const px0 = Math.max(0, -ox / z);
  const px1 = Math.min(page.width, (cssW - ox) / z);
  const py0 = Math.max(0, page.height - (cssH - oy) / z);
  const py1 = Math.min(page.height, page.height + oy / z);
  if (px1 <= px0 || py1 <= py0) return;
  const top = oy;
  const bottom = oy + page.height * z;
  const left = ox;
  const right = ox + page.width * z;
  const snap = (v: number) => Math.round(v) + 0.5;
  ctx.save();
  ctx.lineWidth = 1;
  for (const [isMajor, colour] of [
    [false, "rgba(16, 24, 40, 0.06)"],
    [true, "rgba(16, 24, 40, 0.13)"],
  ] as const) {
    ctx.beginPath();
    const step = isMajor ? major : minor;
    if (isMajor || divisions > 1) {
      for (let i = Math.ceil(px0 / step); i * step <= px1; i++) {
        if (!isMajor && i % divisions === 0) continue;
        const x = snap(ox + i * step * z);
        ctx.moveTo(x, top);
        ctx.lineTo(x, bottom);
      }
      for (let i = Math.ceil(py0 / step); i * step <= py1; i++) {
        if (!isMajor && i % divisions === 0) continue;
        const y = snap(oy + (page.height - i * step) * z);
        ctx.moveTo(left, y);
        ctx.lineTo(right, y);
      }
    }
    ctx.strokeStyle = colour;
    ctx.stroke();
  }
  ctx.restore();
}

function label(v: number): string {
  return String(Math.round(v * 1000) / 1000);
}

/** The rulers, drawn over everything along the top and left edges. Expects a CSS-pixel transform. */
export function drawRulers(
  ctx: CanvasRenderingContext2D,
  view: View,
  page: { width: number; height: number },
  cssW: number,
  cssH: number,
  theme: RulerTheme,
) {
  const { zoom: z, ox, oy } = view;
  const { major, divisions } = spacing(z, 56, 6);
  const minor = major / divisions;
  ctx.save();
  ctx.fillStyle = theme.background;
  ctx.fillRect(0, 0, cssW, RULER);
  ctx.fillRect(0, 0, RULER, cssH);
  // The page's extent, shaded on each ruler.
  ctx.fillStyle = theme.page;
  ctx.fillRect(Math.max(RULER, ox), 0, Math.max(0, Math.min(cssW, ox + page.width * z) - Math.max(RULER, ox)), RULER);
  ctx.fillRect(0, Math.max(RULER, oy), RULER, Math.max(0, Math.min(cssH, oy + page.height * z) - Math.max(RULER, oy)));

  ctx.font = `10px "Segoe UI", system-ui, sans-serif`;
  ctx.fillStyle = theme.text;
  ctx.strokeStyle = theme.tick;
  ctx.lineWidth = 1;
  ctx.beginPath();
  // Horizontal: x inches from the page's left edge.
  ctx.save();
  ctx.beginPath();
  ctx.rect(RULER, 0, cssW - RULER, RULER);
  ctx.clip();
  ctx.beginPath();
  ctx.textBaseline = "top";
  ctx.textAlign = "left";
  for (let i = Math.floor((RULER - ox) / z / minor); i * minor <= (cssW - ox) / z; i++) {
    const v = i * minor;
    const x = Math.round(ox + v * z) + 0.5;
    const isMajor = i % divisions === 0;
    const len = isMajor ? RULER : i % (divisions / 2) === 0 && divisions % 2 === 0 ? 7 : 4;
    ctx.moveTo(x, RULER - len);
    ctx.lineTo(x, RULER);
    if (isMajor) ctx.fillText(label(v), x + 3, 2);
  }
  ctx.stroke();
  ctx.restore();

  // Vertical: y inches up from the page's bottom edge; labels read bottom to top.
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, RULER, RULER, cssH - RULER);
  ctx.clip();
  ctx.beginPath();
  const H = page.height;
  const yAt = (v: number) => oy + (H - v) * z;
  const lo = H - (cssH - oy) / z;
  const hi = H - (RULER - oy) / z;
  for (let i = Math.floor(lo / minor); i * minor <= hi; i++) {
    const v = i * minor;
    const y = Math.round(yAt(v)) + 0.5;
    const isMajor = i % divisions === 0;
    const len = isMajor ? RULER : i % (divisions / 2) === 0 && divisions % 2 === 0 ? 7 : 4;
    ctx.moveTo(RULER - len, y);
    ctx.lineTo(RULER, y);
    if (isMajor) {
      ctx.save();
      ctx.translate(2, y - 3);
      ctx.rotate(-Math.PI / 2);
      ctx.textBaseline = "top";
      ctx.textAlign = "left";
      ctx.fillText(label(v), 0, 0);
      ctx.restore();
    }
  }
  ctx.stroke();
  ctx.restore();

  // Edges and the corner square.
  ctx.fillStyle = theme.background;
  ctx.fillRect(0, 0, RULER, RULER);
  ctx.strokeStyle = theme.border;
  ctx.beginPath();
  ctx.moveTo(0, RULER - 0.5);
  ctx.lineTo(cssW, RULER - 0.5);
  ctx.moveTo(RULER - 0.5, 0);
  ctx.lineTo(RULER - 0.5, cssH);
  ctx.stroke();
  ctx.restore();
}
