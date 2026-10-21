// A page drawn off-screen by the canvas renderer, at print or export
// resolution: PNG export and printing use it, so they look like the screen.

import type { DisplayList } from "../engine/protocol";
import { preloadImages, render } from "./renderer";

/** Largest bitmap side, pixels: browsers refuse bigger canvases. */
const MAX_SIDE = 16384;

/** The page at `dpi` dots per inch (lowered if the bitmap would be too large). */
export async function pageImage(dl: DisplayList, dpi: number): Promise<HTMLCanvasElement> {
  await Promise.all([preloadImages(dl), document.fonts.ready]);
  const { width, height } = dl.page;
  const scale = Math.min(dpi, MAX_SIDE / Math.max(width, height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(width * scale));
  canvas.height = Math.max(1, Math.round(height * scale));
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("could not draw the page");
  render(ctx, dl, { zoom: scale, ox: 0, oy: 0 }, 1, { pasteboard: "#ffffff", pageShadow: "transparent" }, () => {});
  return canvas;
}

export async function pagePng(dl: DisplayList, dpi: number): Promise<ArrayBuffer> {
  const canvas = await pageImage(dl, dpi);
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob) throw new Error("could not encode the PNG");
  return blob.arrayBuffer();
}
