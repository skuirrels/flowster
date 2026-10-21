#!/usr/bin/env node
// Writes the licence notices of everything Flowster ships: the Rust crates in the desktop host and
// the WASM engine, and the UI's npm packages. Help > About shows the file. tauri.conf.json runs this
// after the UI build, so every installer the release kit builds carries it.
//
//   node scripts/third-party-notices.mjs [--out <file>]   # default ui/dist/third-party-notices.txt
//
// Only normal dependencies count: build and dev dependencies aren't shipped. Each distinct licence
// text is written once, with the packages that use it; NOTICE files are kept per package. Warns
// about packages with no licence file and about licences that aren't permissive.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outArg = process.argv.indexOf("--out");
const out = outArg > 0 ? process.argv[outArg + 1] : join(root, "ui", "dist", "third-party-notices.txt");
if (!existsSync(dirname(out))) {
  console.error(`third-party-notices: ${dirname(out)} doesn't exist; build the UI first`);
  process.exit(1);
}

const LICENCE_FILE = /^(licen[cs]e|copying|notice|copyright|unlicense)([-._].*)?$/i;
const PERMISSIVE = new Set([
  "MIT", "MIT-0", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib",
  "Unicode-3.0", "Unicode-DFS-2016", "BSL-1.0", "CC0-1.0", "Unlicense", "0BSD", "CDLA-Permissive-2.0",
]);

/** Licence and notice files at the top of a package folder (and in a LICENSES/ folder). */
function licenceFiles(dir, extra) {
  const files = [];
  const scan = (d) => {
    for (const f of readdirSync(d)) {
      const p = join(d, f);
      if (LICENCE_FILE.test(f) && statSync(p).isFile()) files.push(p);
    }
  };
  scan(dir);
  if (existsSync(join(dir, "LICENSES"))) scan(join(dir, "LICENSES"));
  if (extra && existsSync(join(dir, extra)) && !files.includes(join(dir, extra))) files.push(join(dir, extra));
  return files;
}

/** The non-workspace crates `rootName` links on `platform`, through normal dependencies only. */
function crates(rootName, platform) {
  const meta = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--locked", "--filter-platform", platform], {
      cwd: root,
      maxBuffer: 512 << 20,
    }),
  );
  const members = new Set(meta.workspace_members);
  const packages = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const start = meta.packages.find((p) => p.name === rootName && members.has(p.id));
  if (!start) throw new Error(`no workspace package ${rootName}`);
  const seen = new Set();
  const stack = [start.id];
  while (stack.length) {
    const id = stack.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const d of nodes.get(id).deps) if (d.dep_kinds.some((k) => k.kind === null)) stack.push(d.pkg);
  }
  return [...seen]
    .filter((id) => !members.has(id))
    .map((id) => packages.get(id))
    .map((p) => {
      const dir = dirname(p.manifest_path);
      return { name: p.name, version: p.version, licence: p.license ?? "(see licence file)", files: licenceFiles(dir, p.license_file) };
    });
}

/** The folder of npm package `name` as Node would resolve it from `from` (works with pnpm's layout). */
function resolvePackage(from, name) {
  for (let d = from; ; d = dirname(d)) {
    const candidate = join(d, "node_modules", name);
    if (existsSync(join(candidate, "package.json"))) return realpathSync(candidate);
    if (dirname(d) === d) return null;
  }
}

/** The UI's production npm packages and everything they depend on. */
function npmPackages() {
  const found = new Map();
  const visit = (dir) => {
    const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    const key = `${pkg.name}@${pkg.version}`;
    if (found.has(key)) return;
    const licence = typeof pkg.license === "string" ? pkg.license : (pkg.license?.type ?? pkg.licenses?.map((l) => l.type).join(" OR ") ?? "(see licence file)");
    found.set(key, { name: pkg.name, version: pkg.version, licence, files: licenceFiles(dir) });
    for (const dep of Object.keys({ ...pkg.dependencies, ...pkg.optionalDependencies })) {
      const d = resolvePackage(dir, dep);
      if (d) visit(d);
    }
  };
  const ui = join(root, "ui");
  const pkg = JSON.parse(readFileSync(join(ui, "package.json"), "utf8"));
  for (const dep of Object.keys(pkg.dependencies ?? {})) {
    const d = resolvePackage(ui, dep);
    if (!d) throw new Error(`npm package ${dep} isn't installed; run pnpm install`);
    visit(d);
  }
  return [...found.values()];
}

const host = /host: (\S+)/.exec(execFileSync("rustc", ["-vV"], { encoding: "utf8" }))[1];
const byKey = new Map();
for (const c of [...crates("flowster-app", host), ...crates("flowster-wasm", "wasm32-unknown-unknown")]) {
  byKey.set(`${c.name}@${c.version}`, c);
}
const rust = [...byKey.values()].sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
const npm = npmPackages().sort((a, b) => a.name.localeCompare(b.name));

// Each distinct text once, with the packages that use it.
const texts = new Map();
for (const p of [...rust, ...npm]) {
  for (const f of p.files) {
    const text = readFileSync(f, "utf8").replace(/\r\n?/g, "\n").trim();
    if (!text) continue;
    // Copies of one licence often differ only in line wrapping.
    const key = text.replace(/\s+/g, " ");
    if (!texts.has(key)) texts.set(key, { text, users: [] });
    texts.get(key).users.push(`${p.name} ${p.version}`);
  }
}

const { version } = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
const table = (list) => {
  const w = Math.max(...list.map((p) => `${p.name} ${p.version}`.length));
  return list.map((p) => `  ${`${p.name} ${p.version}`.padEnd(w)}  ${p.licence}`).join("\n");
};
const noFile = [...rust, ...npm].filter((p) => p.files.length === 0);
const sections = [
  `Flowster ${version}: third-party notices`,
  "",
  "Flowster includes the open-source components below, each used under the licence shown.",
  "Their licence texts and notices follow the lists.",
  "",
  ...([...rust, ...npm].some((p) => /MPL/.test(p.licence))
    ? ["The MPL-2.0 components are used unmodified; their source code is available from crates.io at the versions listed.", ""]
    : []),
  `Rust crates (${rust.length})`,
  "",
  table(rust),
  "",
  `npm packages (${npm.length})`,
  "",
  table(npm),
  "",
];
if (noFile.length) {
  sections.push(
    "These packages declare their licence but ship no licence file; the standard text of that licence above applies:",
    "",
    table(noFile),
    "",
  );
}
for (const { text, users } of texts.values()) {
  sections.push("=".repeat(100), `Used by: ${users.join(", ")}`, "=".repeat(100), "", text, "");
}
writeFileSync(out, sections.join("\n"));

// Fine when some alternative of the SPDX expression is all permissive (parentheses are flattened,
// which is close enough for a warning).
const permissive = (expr) =>
  expr
    .replace(/[()]/g, " ")
    .split(/\s+OR\s+|\//)
    .some((alt) => alt.split(/\s+AND\s+/).every((a) => PERMISSIVE.has(a.trim())));
const copyleft = [...rust, ...npm].filter((p) => !permissive(p.licence));
console.log(`third-party-notices: ${rust.length} crates, ${npm.length} npm packages, ${texts.size} distinct texts → ${out}`);
if (noFile.length) console.warn(`third-party-notices: no licence file in ${noFile.map((p) => p.name).join(", ")}`);
if (copyleft.length) {
  console.warn(`third-party-notices: check these licences: ${copyleft.map((p) => `${p.name} (${p.licence})`).join(", ")}`);
}
