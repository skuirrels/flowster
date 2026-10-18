// Fills for the canvas renderer: solid colours, gradients in paint space,
// rectangular gradients and 8 × 8 hatch patterns (see crates/flowster-render/src/paint.rs).

import type { Affine, Paint, Rgba, Stop } from "../engine/protocol";

export const css = ([r, g, b, a]: Rgba) => `rgba(${r},${g},${b},${(a / 255).toFixed(3)})`;

export const mul = (m: Affine, n: Affine): Affine => [
  m[0] * n[0] + m[2] * n[1],
  m[1] * n[0] + m[3] * n[1],
  m[0] * n[2] + m[2] * n[3],
  m[1] * n[2] + m[3] * n[3],
  m[0] * n[4] + m[2] * n[5] + m[4],
  m[1] * n[4] + m[3] * n[5] + m[5],
];

function invert([a, b, c, d, e, f]: Affine): Affine | null {
  const det = a * d - b * c;
  if (Math.abs(det) < 1e-15) return null;
  const [ia, ib, ic, id] = [d / det, -b / det, -c / det, a / det];
  return [ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)];
}

const matrix = ([a, b, c, d, e, f]: Affine) => new DOMMatrix([a, b, c, d, e, f]);

function addStops(g: CanvasGradient, stops: Stop[]) {
  for (const s of stops) g.addColorStop(Math.min(1, Math.max(0, s.offset)), css(s.color));
}

/** Stops reflected about the gradient's origin: offset s → 0.5 ± s/2. */
function mirrored(stops: Stop[]): Stop[] {
  const back = [...stops].reverse().map((s) => ({ offset: 0.5 - s.offset / 2, color: s.color }));
  const fwd = stops.map((s) => ({ offset: 0.5 + s.offset / 2, color: s.color }));
  return [...back, ...fwd];
}

const patterns = new Map<string, HTMLCanvasElement>();
function patternTile(bits: number[], fg: Rgba, bg: Rgba): HTMLCanvasElement {
  const key = `${bits.join(",")}|${fg.join(",")}|${bg.join(",")}`;
  let c = patterns.get(key);
  if (c) return c;
  c = document.createElement("canvas");
  c.width = 8;
  c.height = 8;
  const g = c.getContext("2d")!;
  g.fillStyle = css(bg);
  g.fillRect(0, 0, 8, 8);
  g.fillStyle = css(fg);
  // Paint space is +Y up and the page transform flips Y, so store rows bottom-up.
  bits.forEach((row, r) => {
    for (let col = 0; col < 8; col++) if (row & (0x80 >> col)) g.fillRect(col, 7 - r, 1, 1);
  });
  patterns.set(key, c);
  return c;
}

/** Fills `pagePath` (page coordinates) with `paint`; `pageToDevice` is the current view. */
export function fillPaint(ctx: CanvasRenderingContext2D, pagePath: Path2D, paint: Paint, pageToDevice: Affine): void {
  if (paint.type === "solid") {
    ctx.setTransform(...pageToDevice);
    ctx.fillStyle = css(paint.color);
    ctx.fill(pagePath, "evenodd");
    return;
  }
  const inv = invert(paint.space);
  if (!inv) return;
  // Draw in paint space so gradients and patterns follow the shape's box and rotation.
  const p = new Path2D();
  p.addPath(pagePath, matrix(inv));
  ctx.setTransform(...mul(pageToDevice, paint.space));

  switch (paint.type) {
    case "linear": {
      const g = ctx.createLinearGradient(paint.x0, paint.y0, paint.x1, paint.y1);
      addStops(g, paint.stops);
      ctx.fillStyle = g;
      ctx.fill(p, "evenodd");
      break;
    }
    case "radial": {
      const g = ctx.createRadialGradient(paint.cx, paint.cy, 0, paint.cx, paint.cy, Math.max(paint.r, 1e-9));
      addStops(g, paint.stops);
      ctx.fillStyle = g;
      ctx.fill(p, "evenodd");
      break;
    }
    case "rect": {
      // Gradient along X over the whole shape, then along Y inside the "bow tie"
      // where Y dominates. The two agree on the diagonals, so no seam shows.
      const { ox, oy, extent: e } = paint;
      const stops = mirrored(paint.stops);
      const gx = ctx.createLinearGradient(ox - e, oy, ox + e, oy);
      addStops(gx, stops);
      ctx.fillStyle = gx;
      ctx.fill(p, "evenodd");
      ctx.save();
      const bow = new Path2D();
      bow.moveTo(ox, oy);
      bow.lineTo(ox + 4, oy + 4);
      bow.lineTo(ox - 4, oy + 4);
      bow.closePath();
      bow.moveTo(ox, oy);
      bow.lineTo(ox + 4, oy - 4);
      bow.lineTo(ox - 4, oy - 4);
      bow.closePath();
      ctx.clip(bow);
      const gy = ctx.createLinearGradient(ox, oy - e, ox, oy + e);
      addStops(gy, stops);
      ctx.fillStyle = gy;
      ctx.fill(p, "evenodd");
      ctx.restore();
      break;
    }
    case "pattern": {
      const pat = ctx.createPattern(patternTile(paint.bits, paint.fg, paint.bg), "repeat");
      if (!pat) return;
      // Like Visio on screen, hatches keep one bit per CSS pixel at any zoom
      // (scaling them with the page turns them into moiré when zoomed out).
      const devicePerBit = Math.hypot(...mul(pageToDevice, paint.space).slice(0, 2));
      const k = (window.devicePixelRatio || 1) / Math.max(devicePerBit, 1e-9);
      pat.setTransform(new DOMMatrix([k, 0, 0, k, 0, 0]));
      ctx.imageSmoothingEnabled = false;
      ctx.fillStyle = pat;
      ctx.fill(p, "evenodd");
      ctx.imageSmoothingEnabled = true;
      break;
    }
  }
}
