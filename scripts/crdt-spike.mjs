#!/usr/bin/env node
// Runs the CRDT spike (docs/adr/0001-crdt-library.md) in WASM under Node (V8,
// the same engine as WebView2) and natively, then prints a comparison table.
//
//   node scripts/crdt-spike.mjs [--shapes 10000] [--json out.json]
//
// Prerequisite: `node scripts/build-wasm.mjs` (release build for real numbers).

import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const arg = (name, def) => {
  const i = process.argv.indexOf(name);
  return i > 0 ? process.argv[i + 1] : def;
};
const shapes = Number(arg("--shapes", "10000"));
const jsonOut = arg("--json", null);

const native = JSON.parse(
  execFileSync(
    "cargo",
    ["run", "--quiet", "--release", "-p", "flowster-cli", "--", "bench", "crdt", "--shapes", String(shapes), "--json"],
    { cwd: root, encoding: "utf8", maxBuffer: 1 << 26 },
  ),
);

const require = createRequire(import.meta.url);
const wasm = require(join(root, "target", "wasm-node", "flowster_wasm.js"));
const wasmReports = ["loro", "yrs"].map((lib) => JSON.parse(wasm.crdtSpike(lib, shapes)));

const all = [...native, ...wasmReports];
if (jsonOut) writeFileSync(jsonOut, JSON.stringify(all, null, 2));

const mib = (b) => (b / 1024 / 1024).toFixed(1);
const kib = (b) => (b / 1024).toFixed(1);
const rows = [
  ["plain model heap (MiB)", (r) => mib(r.plain_model_bytes)],
  ["build from scratch (ms)", (r) => r.build_ms.toFixed(0)],
  ["built doc heap (MiB)", (r) => mib(r.built_heap_bytes)],
  ["snapshot size (KiB)", (r) => kib(r.snapshot_bytes)],
  ["snapshot export (ms)", (r) => r.snapshot_export_ms.toFixed(1)],
  ["snapshot import (ms)", (r) => r.load_import_ms.toFixed(1)],
  ["import + read all (ms)", (r) => r.load_open_ms.toFixed(1)],
  ["loaded replica heap (MiB)", (r) => mib(r.loaded_heap_bytes)],
  ["loaded replica peak (MiB)", (r) => mib(r.loaded_peak_bytes)],
  ["memory vs plain model", (r) => `${r.memory_ratio_vs_plain.toFixed(2)}x`],
  ["update: move 1 shape (B)", (r) => r.update_bytes.move_one],
  ["update: drag 10 shapes (B)", (r) => r.update_bytes.move_ten],
  ["update: resize (B)", (r) => r.update_bytes.resize],
  ["update: type 1 char (B)", (r) => r.update_bytes.text_insert_char],
  ["update: type 8-char word (B)", (r) => r.update_bytes.text_insert_word],
  ["update: bold range (B)", (r) => r.update_bytes.text_bold],
  ["update: z-order (B)", (r) => r.update_bytes.z_order],
  ["update: create shape (B)", (r) => r.update_bytes.create_shape],
  ["update: delete shape (B)", (r) => r.update_bytes.delete_shape],
  ["remote apply: move (µs)", (r) => r.apply_move_us.toFixed(1)],
  ["remote apply: keystroke (µs)", (r) => r.apply_text_us.toFixed(1)],
  ["converged", (r) => (r.converged ? "yes" : "NO")],
];
const head = ["metric", ...all.map((r) => `${r.library} ${r.target}`)];
console.log(`\n${shapes} shapes, ${all[0].pages} pages\n`);
console.log(`| ${head.join(" | ")} |`);
console.log(`|${head.map((_, i) => (i ? "---:" : "---")).join("|")}|`);
for (const [label, f] of rows) console.log(`| ${[label, ...all.map(f)].join(" | ")} |`);
for (const r of all) for (const n of r.notes) console.log(`note (${r.library} ${r.target}): ${n}`);
if (all.some((r) => !r.converged)) process.exit(1);
