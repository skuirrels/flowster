#!/usr/bin/env node
// Builds crates/flowster-wasm for the Engine Worker and for Node-based checks.
//
//   node scripts/build-wasm.mjs           # release (profile wasm-release)
//   node scripts/build-wasm.mjs --dev     # debug, faster to build
//
// Outputs:
//   ui/src/engine/wasm/   --target web     (imported by engine.worker.ts)
//   target/wasm-node/     --target nodejs  (determinism + spike scripts)
//
// Needs `wasm-bindgen-cli` at exactly the version pinned in Cargo.toml:
//   cargo install wasm-bindgen-cli --version 0.2.129 --locked

import { execFileSync } from "node:child_process";
import { mkdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const dev = process.argv.includes("--dev");
const profile = dev ? "dev" : "wasm-release";
const outDir = dev ? "debug" : "wasm-release";

function run(cmd, args) {
  console.log(`> ${cmd} ${args.join(" ")}`);
  execFileSync(cmd, args, { cwd: root, stdio: "inherit" });
}

run("cargo", ["build", "-p", "flowster-wasm", "--target", "wasm32-unknown-unknown", "--profile", profile]);

const wasm = join(root, "target", "wasm32-unknown-unknown", outDir, "flowster_wasm.wasm");
const targets = [
  ["web", join(root, "ui", "src", "engine", "wasm")],
  ["nodejs", join(root, "target", "wasm-node")],
];
for (const [target, dir] of targets) {
  mkdirSync(dir, { recursive: true });
  run("wasm-bindgen", [wasm, "--target", target, "--out-dir", dir, "--out-name", "flowster_wasm"]);
}

const size = statSync(join(targets[0][1], "flowster_wasm_bg.wasm")).size;
console.log(`flowster_wasm_bg.wasm: ${(size / 1024 / 1024).toFixed(2)} MiB (${profile})`);
