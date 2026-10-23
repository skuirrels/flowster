// Canvas text placement (SPEC §8.6): the renderer lays text out piece by piece
// from each piece's left edge, so it must draw the same on every frame whatever
// state the overlay or the rulers left in the 2D context. Run with `pnpm test`
// in ui/ (Vite loads the TypeScript; the context is a recording stand-in).

import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";

import { createServer } from "vite";

let server;
let renderer;
let edit;

before(async () => {
  server = await createServer({
    root: fileURLToPath(new URL("..", import.meta.url)),
    configFile: false,
    logLevel: "silent",
    server: { middlewareMode: true, hmr: false },
    appType: "custom",
  });
  renderer = await server.ssrLoadModule("/src/canvas/renderer.ts");
  edit = await server.ssrLoadModule("/src/canvas/edit.ts");
});

after(() => server?.close());

/** A 2D context that keeps its state (with save/restore) and records every fillText. */
function recordingContext() {
  const state = { textAlign: "start", textBaseline: "alphabetic", direction: "inherit", font: "10px sans-serif" };
  const stack = [];
  const drawn = [];
  const target = {
    canvas: { width: 800, height: 600 },
    save: () => stack.push({ ...state }),
    restore: () => Object.assign(state, stack.pop() ?? {}),
    // Every glyph is half the font size wide.
    measureText: (text) => ({ width: text.length * Number.parseFloat(/([\d.]+)px/.exec(state.font)[1]) * 0.5 }),
    fillText: (text, x, y) => drawn.push({ text, x, y, align: state.textAlign }),
  };
  const ctx = new Proxy(target, {
    get: (t, k) => (k in t ? t[k] : k in state ? state[k] : () => {}),
    set: (t, k, v) => {
      if (k in state) state[k] = v;
      else t[k] = v;
      return true;
    },
  });
  return { ctx, drawn };
}

const run = (text) => ({
  text,
  font: "Calibri",
  size: 8 / 72,
  color: [0, 0, 0, 255],
  bold: false,
  italic: false,
  underline: false,
  strike: false,
});

const para = (align, text) => ({
  align,
  lineSpacing: -1.2,
  spaceBefore: 0,
  spaceAfter: 0,
  indentLeft: 0,
  indentFirst: 0,
  runs: [run(text)],
});

/** A page with a centred pill label and a left-aligned table cell. */
const page = {
  page: { index: 0, name: "Page-1", width: 10, height: 8, background: false },
  shapes: [],
  warnings: [],
  items: [
    {
      kind: "text",
      shape: 1,
      transform: [1, 0, 0, 1, 1, 6],
      width: 1,
      height: 0.25,
      margins: [0.028, 0.028, 0.028, 0.028],
      valign: 1,
      background: null,
      paragraphs: [para(1, "Colibra (tbc)")],
    },
    {
      kind: "text",
      shape: 2,
      transform: [1, 0, 0, 1, 3, 6],
      width: 2,
      height: 0.4,
      margins: [0.056, 0.056, 0.056, 0.056],
      valign: 1,
      background: null,
      paragraphs: [para(0, "Value Stream")],
    },
  ],
};

const view = { zoom: 90, ox: 0, oy: 0 };
const theme = { pasteboard: "#ffffff", pageShadow: "transparent" };

test("text draws from each piece's left edge, frame after frame", () => {
  const { ctx, drawn } = recordingContext();
  renderer.render(ctx, page, view, 2, theme, () => {});
  const first = drawn.splice(0);
  // What the overlay draws between frames (its labels are centred).
  edit.drawOverlay(ctx, view, 2, page.page.height, { selected: [], handles: [] });
  renderer.render(ctx, page, view, 2, theme, () => {});
  assert.deepEqual(drawn, first);
  assert.ok(first.every((d) => d.align === "left"), JSON.stringify(first));

  // Text units are 1/100 in; 8 pt glyphs measure 100/18 units each here.
  const glyph = (8 / 72) * 100 * 0.5;
  const pill = first.filter((d) => d.text.includes("Colibra") || d.text.includes("(tbc)"));
  assert.equal(pill.length, 2);
  const maxW = (1 - 2 * 0.028) * 100;
  assert.ok(Math.abs(pill[0].x - (2.8 + (maxW - 13 * glyph) / 2)) < 1e-9, `centred at ${pill[0].x}`);
  // " (tbc)" starts where "Colibra" ends.
  assert.ok(Math.abs(pill[1].x - (pill[0].x + 7 * glyph)) < 1e-9);
  const cell = first.filter((d) => d.text.includes("Value") || d.text.includes("Stream"));
  assert.ok(Math.abs(cell[0].x - 5.6) < 1e-9, `left margin at ${cell[0].x}`);
  assert.ok(Math.abs(cell[1].x - (cell[0].x + 5 * glyph)) < 1e-9);
});

test("the overlay leaves the context's text state as it found it", () => {
  const { ctx } = recordingContext();
  ctx.textAlign = "left";
  ctx.textBaseline = "alphabetic";
  edit.drawOverlay(ctx, view, 1, page.page.height, {
    selected: [],
    handles: [],
    measures: [{ a: [1, 1], b: [4, 1], label: "3 in" }],
    readout: { at: [100, 100], text: "2 × 1 in" },
  });
  assert.equal(ctx.textAlign, "left");
  assert.equal(ctx.textBaseline, "alphabetic");
});
