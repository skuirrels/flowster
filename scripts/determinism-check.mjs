#!/usr/bin/env node
// Determinism harness (SPEC §20.3, §20.11): the derived-state hash computed by
// the native build and by the WASM build must be identical, and must equal the
// golden hash committed in crates/flowster-core/src/determinism.rs (which is
// how CI catches divergence between Windows, macOS and Linux).
//
// Prerequisite: `node scripts/build-wasm.mjs` (writes target/wasm-node/).

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

const native = JSON.parse(
  execFileSync("cargo", ["run", "--quiet", "--release", "-p", "flowster-cli", "--", "determinism", "--json"], {
    cwd: root,
    encoding: "utf8",
  }),
);

const require = createRequire(import.meta.url);
const wasmModule = require(join(root, "target", "wasm-node", "flowster_wasm.js"));
const wasm = JSON.parse(wasmModule.derivedStateHash());

const src = readFileSync(join(root, "crates", "flowster-core", "src", "determinism.rs"), "utf8");
const golden = /GOLDEN_HASH: &str = "([0-9a-f]{16})"/.exec(src)?.[1];

const platform = `${process.platform}-${process.arch}`;
console.log(`platform : ${platform}`);
console.log(`native   : ${native.hash}  (${native.values} values)`);
console.log(`wasm32   : ${wasm.hash}  (${wasm.values} values)`);
console.log(`golden   : ${golden}`);

let ok = true;
if (native.hash !== wasm.hash || native.values !== wasm.values) {
  console.error("FAIL: native and WASM derived state differ");
  ok = false;
}
if (native.hash !== golden) {
  console.error("FAIL: derived state differs from the golden hash (platform-dependent math?)");
  ok = false;
}
if (!ok) process.exit(1);
console.log("OK: derived state is identical");
