// release-kit: build a Tauri app's installers on this machine and publish them to the app's
// download site and update feeds. Every app and every machine uses the same commands (the app's
// Makefile wraps them):
//
//   version              this checkout's version and what's published for each platform
//   bump [x.y.z]         set the next version in every file that records it (default: published + 1)
//   build                this OS's installers (and the app's extras) into release/<version>/
//   linux                on Windows: the Linux installers, built in WSL (Ubuntu-22.04)
//   linux-setup          on Windows: install the Linux build tools in that WSL distro
//   publish [--dry-run]  add release/<version>/ to the download site and update feeds, and push
//   release              build, linux (on Windows), publish, clean
//   clean                delete published builds and Tauri's bundle output
//
// Options: --force (build or publish a version that isn't newer than the published one),
// --allow-dirty (build with uncommitted changes), --no-linux (`release` on Windows without Linux).
//
// Installers can't be cross-compiled: Windows builds Windows (and Linux in WSL), a Mac builds both
// macOS architectures. The app describes itself in release.config.mjs (see README.md here). This
// folder is a copy of the release-kit repository: change it there, then sync it into every app.

import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { basename, dirname, join, relative } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { GROUPS_BY_OS, FEED_GROUPS, buildSite, catalog, newer } from "./site.mjs";

export const KIT_VERSION = "1.0.4";

const kitDir = dirname(fileURLToPath(import.meta.url));
const appDir = join(kitDir, "..", "..");
const config = (await import(pathToFileURL(join(appDir, "release.config.mjs")).href)).default;
const confPath = join(appDir, "src-tauri", "tauri.conf.json");
const conf = () => JSON.parse(readFileSync(confPath, "utf8"));
const NAME = conf().productName;
const SITE_URL = config.siteUrl.replace(/\/?$/, "/");
const SITE_REPO = process.env.SITE_REPO ?? config.siteRepo;
const OS = { win32: "windows", darwin: "macos" }[process.platform] ?? "linux";
const isWindows = OS === "windows";
const WSL_DISTRO = process.env.RELEASE_WSL_DISTRO ?? "Ubuntu-22.04";

const [command = "help", ...rest] = process.argv.slice(2);
const flags = new Set(rest.filter((a) => a.startsWith("--")));
const args = rest.filter((a) => !a.startsWith("--"));

function fail(message) {
  console.error(`release: ${message}`);
  process.exit(1);
}

/** Quotes one argument for cmd.exe. */
const q = (a) => (/^[\w.,:=\/@+-]+$/.test(a) ? a : `"${a.replace(/"/g, '\\"')}"`);

function spawn(cmd, cmdArgs, options) {
  // npm, npx and pnpm are .cmd files on Windows, which only start through a shell.
  if (isWindows && /^(np[mx]|pnpm)$/.test(cmd)) return spawnSync([cmd, ...cmdArgs].map(q).join(" "), { shell: true, ...options });
  return spawnSync(cmd, cmdArgs, options);
}

function run(cmd, cmdArgs, options = {}) {
  console.log(`> ${cmd} ${cmdArgs.join(" ")}`);
  const r = spawn(cmd, cmdArgs, { stdio: "inherit", cwd: appDir, ...options });
  if (r.status !== 0) fail(`${cmd} failed (exit ${r.status ?? r.signal})`);
}

/** A command's trimmed output, or null if it failed. */
function output(cmd, cmdArgs, options = {}) {
  const r = spawn(cmd, cmdArgs, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], cwd: appDir, ...options });
  return r.status === 0 ? r.stdout.trim() : null;
}

const version = () => conf().version;
const releaseDir = (v) => join(appDir, "release", v);
const hostTriple = () => /host: (\S+)/.exec(output("rustc", ["-vV"]) ?? "")?.[1] ?? fail("rustc not found");
const targetDir = () => JSON.parse(output("cargo", ["metadata", "--format-version", "1", "--no-deps"]) ?? fail("cargo metadata failed")).target_directory;
const find = (dir, re) => (existsSync(dir) ? readdirSync(dir).filter((f) => re.test(f)).map((f) => join(dir, f)) : []);
const entries = () => catalog(NAME, config.extras?.files ?? []);

/** The platforms ("windows", "macos", "linux") a folder of build outputs has files for. */
const platformsIn = (dir) => new Set(readdirSync(dir).map((f) => entries().find((e) => e.from?.test(f))?.platform).filter(Boolean));

// ---- published versions -------------------------------------------------------------------------

async function fetchJson(path) {
  try {
    const res = await fetch(SITE_URL + path, { signal: AbortSignal.timeout(20_000), cache: "no-store" });
    return res.ok ? await res.json() : null;
  } catch {
    return null;
  }
}

/**
 * The version published for each platform feed (`updates/<os>-<arch>.json`), falling back to the
 * combined feed for a site that has no per-platform feeds yet.
 */
async function publishedVersions() {
  const combined = await fetchJson(config.combinedFeed);
  const groups = {};
  await Promise.all(
    FEED_GROUPS.map(async (g) => {
      const feed = await fetchJson(`updates/${g}.json`);
      const inCombined = Object.keys(combined?.platforms ?? {}).some((k) => k === g || k.startsWith(`${g}-`));
      groups[g] = feed?.version ?? (inCombined ? combined.version : null);
    }),
  );
  return { groups, combined: combined?.version ?? null };
}

/** Refuses a version that isn't newer than what the platforms' feeds already have. */
async function checkNewer(platforms, { allowEqual }) {
  if (flags.has("--force")) return;
  const v = version();
  const { groups } = await publishedVersions();
  for (const p of platforms) {
    for (const g of GROUPS_BY_OS[p]) {
      const published = groups[g];
      if (!published) continue;
      if (newer(published, v) || (!allowEqual && published === v)) {
        fail(`${NAME} ${v} isn't newer than ${published}, already published for ${g}; run \`make bump\` first (or pass --force)`);
      }
    }
  }
}

// ---- version, bump ------------------------------------------------------------------------------

async function showVersion() {
  const { groups, combined } = await publishedVersions();
  console.log(`${NAME} (release-kit ${KIT_VERSION})`);
  console.log(`  this checkout   ${version()}`);
  for (const g of FEED_GROUPS) console.log(`  ${g.padEnd(15)} ${groups[g] ?? "(not published)"}`);
  console.log(`  ${config.combinedFeed.padEnd(15)} ${combined ?? "(not published)"}  (feed of older installed copies)`);
  const mine = GROUPS_BY_OS[OS].map((g) => groups[g]).filter(Boolean);
  if (mine.some((p) => !newer(version(), p))) console.log("Run `make bump` before building: installed copies only update to a higher version.");
}

function setJsonVersion(file, next) {
  const text = readFileSync(file, "utf8");
  const updated = text.replace(/"version"\s*:\s*"[^"]*"/, `"version": "${next}"`);
  if (updated === text && !text.includes(`"version": "${next}"`)) fail(`no "version" in ${file}`);
  writeFileSync(file, updated);
}

/** Sets `version = "…"` in a Cargo.toml's [package] or [workspace.package] section. */
function setCargoVersion(file, next) {
  const text = readFileSync(file, "utf8");
  const updated = text.replace(/(^\[(?:workspace\.)?package\][^\[]*?^version\s*=\s*)"[^"]*"/ms, `$1"${next}"`);
  if (updated === text && !text.includes(`version = "${next}"`)) fail(`no package version in ${file}`);
  writeFileSync(file, updated);
}

async function bump() {
  let next = args[0];
  if (!next) {
    const { groups, combined } = await publishedVersions();
    const base = [version(), combined, ...Object.values(groups)].filter(Boolean).reduce((a, b) => (newer(b, a) ? b : a));
    const [major, minor, patch] = base.split(".").map(Number);
    next = `${major}.${minor}.${patch + 1}`;
  }
  const parts = /^(\d+)\.(\d+)\.(\d+)$/.exec(next);
  if (!parts) fail(`expected a version like 1.2.3, got ${next}`);
  // WiX (.msi) only accepts major and minor up to 255 and a patch up to 65535.
  const [major, minor, patch] = parts.slice(1).map(Number);
  if (major > 255 || minor > 255 || patch > 65535) fail(`${next} can't be used for an .msi (limits: 255.255.65535)`);

  const files = ["src-tauri/tauri.conf.json", ...(config.versionFiles ?? [])];
  for (const file of files) {
    if (file.endsWith(".json")) setJsonVersion(join(appDir, file), next);
    else setCargoVersion(join(appDir, file), next);
  }
  // Cargo.lock records the workspace crates' versions; `--locked` builds need it current.
  if (files.some((f) => f.endsWith("Cargo.toml"))) {
    if (spawnSync("cargo", ["update", "--workspace", "--offline"], { cwd: appDir, stdio: "ignore" }).status !== 0) {
      run("cargo", ["update", "--workspace"]);
    }
  }
  console.log(`${NAME} ${next} in ${files.join(", ")}${files.some((f) => f.endsWith("Cargo.toml")) ? " and Cargo.lock" : ""}.`);
  console.log(`Commit and push it, so every machine builds the same version:\n  git commit -am "Version ${next}" && git push`);
}

// ---- build --------------------------------------------------------------------------------------

/** Tauri targets and bundle formats built on each OS. */
const BUILDS = {
  windows: [{ target: null, bundles: "nsis,msi" }],
  macos: [
    { target: "aarch64-apple-darwin", bundles: "app,dmg", arch: "aarch64" },
    { target: "x86_64-apple-darwin", bundles: "app,dmg", arch: "x64" },
  ],
  linux: [{ target: null, bundles: "appimage,deb,rpm" }],
};

function checkCommitted() {
  const dirty = output("git", ["status", "--porcelain", "--untracked-files=no"]);
  if (dirty === null) return;
  if (dirty && !flags.has("--allow-dirty")) {
    fail(`uncommitted changes:\n${dirty}\nCommit them first (every machine must build the same commit), or pass --allow-dirty`);
  }
  if (output("git", ["branch", "-r", "--contains", "HEAD"]) === "") {
    console.warn("release: HEAD isn't pushed yet; push it so the other machine builds the same commit");
  }
}

/** The updater signing key: from the environment, or <slug>-updater.key in ~/.tauri (or RELEASE_KEY_DIR). */
function buildEnv() {
  const env = { ...process.env };
  if (!env.TAURI_SIGNING_PRIVATE_KEY) {
    const keyFile = join(process.env.RELEASE_KEY_DIR ?? join(homedir(), ".tauri"), `${config.slug}-updater.key`);
    if (!existsSync(keyFile)) fail(`no updater signing key: set TAURI_SIGNING_PRIVATE_KEY or put the key at ${keyFile}`);
    env.TAURI_SIGNING_PRIVATE_KEY = readFileSync(keyFile, "utf8").trim();
    const pub = `${keyFile}.pub`;
    if (existsSync(pub) && readFileSync(pub, "utf8").trim() !== conf().plugins?.updater?.pubkey) {
      console.warn(`release: plugins.updater.pubkey in tauri.conf.json isn't ${pub}; installed copies will refuse these updates`);
    }
    if (!env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD && existsSync(`${keyFile}.password`)) {
      env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = readFileSync(`${keyFile}.password`, "utf8").trim();
    }
  }
  // The AppImage tools are AppImages themselves, which need FUSE unless told to extract (WSL has none).
  if (OS === "linux") env.APPIMAGE_EXTRACT_AND_RUN = "1";
  return env;
}

/** The app's package manager: pnpm when it has a pnpm-lock.yaml, otherwise npm. */
const PM = existsSync(join(appDir, "pnpm-lock.yaml"))
  ? { lock: "pnpm-lock.yaml", installed: ".modules.yaml", install: ["pnpm", ["install", "--frozen-lockfile"]], tauri: ["pnpm", ["exec", "tauri"]] }
  : { lock: "package-lock.json", installed: ".package-lock.json", install: ["npm", ["ci"]], tauri: ["npx", ["tauri"]] };

/** Packages when missing or older than the lockfile (after a pull), and the Mac's Rust targets. */
function prerequisites() {
  const installed = join(appDir, "node_modules", PM.installed);
  const lock = join(appDir, PM.lock);
  if (!existsSync(installed) || (existsSync(lock) && statSync(lock).mtimeMs > statSync(installed).mtimeMs)) run(...PM.install);
  if (OS === "macos") {
    const have = output("rustup", ["target", "list", "--installed"]) ?? "";
    const missing = BUILDS.macos.map((b) => b.target).filter((t) => !have.includes(t));
    if (missing.length) run("rustup", ["target", "add", ...missing]);
  }
}

/** Copies a build output into `out` as `name`, with its updater signature if it has one. */
function take(from, out, name) {
  if (!from || !existsSync(from)) fail(`expected ${from} after the build`);
  copyFileSync(from, join(out, name));
  if (existsSync(`${from}.sig`)) copyFileSync(`${from}.sig`, join(out, `${name}.sig`));
  console.log(`  ${join(out, name)}`);
}

/** Packs a folder as <file>.zip on Windows or <file>.tar.gz elsewhere; returns the archive's path. */
function archive(folder, file) {
  if (isWindows) {
    const zip = `${file}.zip`;
    rmSync(zip, { force: true });
    run("powershell", ["-NoProfile", "-Command", `Compress-Archive -Force -Path '${folder}' -DestinationPath '${zip}'`]);
    return zip;
  }
  run("tar", ["-C", dirname(folder), "-czf", `${file}.tar.gz`, basename(folder)]);
  return `${file}.tar.gz`;
}

async function build() {
  const v = version();
  checkCommitted();
  await checkNewer([OS], { allowEqual: false });
  const pubkey = conf().plugins?.updater?.pubkey;
  if (conf().bundle?.createUpdaterArtifacts && (!pubkey || /REPLACE/i.test(pubkey))) {
    fail(`plugins.updater.pubkey in src-tauri/tauri.conf.json isn't set: paste the contents of ~/.tauri/${config.slug}-updater.key.pub`);
  }
  const env = buildEnv();
  prerequisites();
  const out = releaseDir(v);
  mkdirSync(out, { recursive: true });
  const target = targetDir();

  for (const { target: triple, bundles, arch } of BUILDS[OS]) {
    run(PM.tauri[0], [...PM.tauri[1], "build", "--bundles", bundles, ...(triple ? ["--target", triple] : [])], { env });
    const dir = join(target, ...(triple ? [triple] : []), "release", "bundle");
    console.log(`Collected into ${out}:`);
    if (OS === "windows") {
      take(join(dir, "nsis", `${NAME}_${v}_x64-setup.exe`), out, `${NAME}_${v}_x64-setup.exe`);
      take(join(dir, "msi", `${NAME}_${v}_x64_en-US.msi`), out, `${NAME}_${v}_x64_en-US.msi`);
    } else if (OS === "macos") {
      take(find(join(dir, "dmg"), /\.dmg$/).find((f) => basename(f).includes(v)), out, `${NAME}_${v}_${arch}.dmg`);
      take(join(dir, "macos", `${NAME}.app.tar.gz`), out, `${NAME}_${v}_${arch}.app.tar.gz`);
    } else {
      for (const [sub, re] of [["appimage", /\.AppImage$/], ["deb", /\.deb$/], ["rpm", /\.rpm$/]]) {
        const file = find(join(dir, sub), re).find((f) => basename(f).includes(v));
        take(file, out, basename(file ?? `${sub} package`));
      }
    }
  }

  if (config.extras?.build) {
    await config.extras.build({
      os: OS,
      version: v,
      out,
      appDir,
      targetDir: target,
      // null builds for this machine (into target/release, shared with the app's build).
      targets: BUILDS[OS].map((b) => b.target),
      triple: (t) => t ?? hostTriple(),
      run,
      archive,
      env,
    });
  }
  console.log(`\nBuilt ${NAME} ${v} for ${OS} into ${out}.`);
}

// ---- linux (from Windows, in WSL) ---------------------------------------------------------------

function wslDistros() {
  const r = spawnSync("wsl.exe", ["-l", "-q"]);
  if (r.status !== 0) return [];
  // wsl.exe writes UTF-16 unless WSL_UTF8 is set.
  const text = r.stdout.includes(0) ? r.stdout.toString("utf16le") : r.stdout.toString("utf8");
  return text.split(/\r?\n/).map((s) => s.replace(/\0/g, "").trim()).filter(Boolean);
}

function requireDistro() {
  if (!isWindows) fail("this builds Linux in WSL from Windows; on Linux run `make installers`");
  if (!wslDistros().includes(WSL_DISTRO)) {
    fail(`the WSL distro ${WSL_DISTRO} isn't installed. Once, on this PC:
  wsl --install -d ${WSL_DISTRO}      (and create the user it asks for)
  make linux-setup
Linux packages are built on Ubuntu 22.04 so they run on older distributions too (its glibc is older).`);
  }
}

/** C:\Users\x or C:/Users/x -> /mnt/c/Users/x */
const wslPath = (p) => p.replace(/\\/g, "/").replace(/^([A-Za-z]):/, (_, d) => `/mnt/${d.toLowerCase()}`);
const sh = (s) => `'${s.replace(/'/g, "'\\''")}'`;

/** Runs a bash script in the WSL distro (from a file, so nothing is lost to Windows quoting). */
function runInWsl(script) {
  const work = mkdtempSync(join(tmpdir(), `${config.slug}-wsl-`));
  const file = join(work, "run.sh");
  writeFileSync(file, script.replace(/\r\n/g, "\n"));
  try {
    run("wsl.exe", ["-d", WSL_DISTRO, "--", "bash", "-l", wslPath(file)]);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

async function linux() {
  requireDistro();
  checkCommitted();
  if (flags.has("--allow-dirty")) console.warn("release: the Linux build uses the last commit; uncommitted changes aren't in it");
  const v = version();
  const repoRoot = output("git", ["rev-parse", "--show-toplevel"]) ?? fail("the Linux build needs a git checkout");
  const sha = output("git", ["rev-parse", "HEAD"]);
  const rel = relative(repoRoot, appDir).replace(/\\/g, "/") || ".";
  const out = releaseDir(v);
  mkdirSync(out, { recursive: true });
  const keyDir = process.env.RELEASE_KEY_DIR ?? join(homedir(), ".tauri");
  // A clone inside WSL (its own node_modules and target/; building on /mnt/c is slow), set to this commit.
  runInWsl(`set -euo pipefail
src=${sh(wslPath(repoRoot))}
dir="$HOME/release-builds/${config.slug}"
[ -d "$dir/.git" ] || git -c safe.directory='*' clone --quiet --no-checkout "$src" "$dir"
cd "$dir"
git -c safe.directory='*' fetch --quiet "$src" HEAD
git checkout --quiet --force FETCH_HEAD
[ "$(git rev-parse HEAD)" = ${sha} ] || { echo "release: the WSL copy isn't at ${sha.slice(0, 7)}" >&2; exit 1; }
cd ${sh(rel)}${PM.lock === "pnpm-lock.yaml" ? `
# WSL also sees the Windows PATH: a pnpm under /mnt/ is the Windows one, which can't build here.
case "$(command -v pnpm || true)" in
  "" | /mnt/*) echo "release: WSL has no Linux pnpm; run: make linux-setup" >&2; exit 1 ;;
esac
# Corepack fetches the app's pnpm version without asking.
export COREPACK_ENABLE_DOWNLOAD_PROMPT=0` : ""}
RELEASE_KEY_DIR=${sh(wslPath(keyDir))} node scripts/release-kit/release.mjs build ${flags.has("--force") ? "--force" : ""}
cp -f release/${v}/* ${sh(wslPath(out))}/
rm -rf release
`);
  console.log(`\nBuilt ${NAME} ${v} for linux (WSL ${WSL_DISTRO}) into ${out}.`);
}

function linuxSetup() {
  requireDistro();
  // Through runInWsl, which drops the CRs a Windows checkout may have added. config.linuxSetup is
  // the app's own additions (run in its folder of the Windows checkout), e.g. extra Rust targets.
  const app = config.linuxSetup ? `
cd ${sh(wslPath(appDir))}
${config.linuxSetup}
` : "";
  runInWsl(readFileSync(join(kitDir, "provision-wsl.sh"), "utf8") + app);
}

// ---- publish ------------------------------------------------------------------------------------

/** Clones the site's gh-pages branch into `dir` (empty for the first publish), retrying a dropped connection. */
function cloneSite(dir) {
  for (let attempt = 1; ; attempt++) {
    console.log(`> git clone gh-pages of ${SITE_REPO}${attempt > 1 ? ` (attempt ${attempt})` : ""}`);
    const r = spawnSync("git", ["clone", "--quiet", "--depth", "1", "--branch", "gh-pages", SITE_REPO, dir], { encoding: "utf8", stdio: ["ignore", "inherit", "pipe"] });
    if (r.status === 0) break;
    rmSync(dir, { recursive: true, force: true });
    if (/Remote branch gh-pages not found/i.test(r.stderr)) {
      console.warn("release: the releases repo has no gh-pages branch yet; starting the site afresh");
      mkdirSync(dir, { recursive: true });
      return;
    }
    if (attempt === 3) fail(`couldn't download the current site:\n${r.stderr}`);
  }
  rmSync(join(dir, ".git"), { recursive: true, force: true });
}

/** One commit holding only the current files, force-pushed: the releases repo's history stays small. */
function pushSite(site, message) {
  const git = (...a) => run("git", a, { cwd: site });
  const who = ["-c", `user.name=${NAME} release`, "-c", "user.email=release@users.noreply.github.com"];
  git("init", "--quiet", "-b", "gh-pages");
  git("add", "-A");
  git(...who, "commit", "--quiet", "-m", message);
  const push = () => spawnSync("git", ["push", "--quiet", "--force", SITE_REPO, "gh-pages"], { cwd: site, encoding: "utf8" });
  let pushed = push();
  for (let attempt = 2; pushed.status !== 0 && !/workflow/i.test(pushed.stderr ?? "") && attempt <= 3; attempt++) {
    console.warn(`release: push failed (${(pushed.stderr ?? "").trim().split(/\r?\n/).pop()}); retrying`);
    pushed = push();
  }
  if (pushed.status !== 0 && /workflow/i.test(pushed.stderr ?? "")) {
    // Credentials without the "workflow" scope can't push the releases repo's workflow file (which
    // turns each publish into a GitHub release there). Publish the site without it.
    console.warn("release: the releases repo refused its workflow file (your git credentials lack the workflow scope); publishing without it");
    rmSync(join(site, ".github"), { recursive: true, force: true });
    git("add", "-A");
    git(...who, "commit", "--quiet", "--amend", "--no-edit");
    pushed = push();
  }
  if (pushed.status !== 0) fail(`pushing the site failed:\n${pushed.stderr}`);
}

async function publish() {
  const dryRun = flags.has("--dry-run");
  const v = version();
  const out = releaseDir(v);
  if (!existsSync(out) || !readdirSync(out).length) fail(`nothing in ${out}: run \`make installers\` first`);
  const platforms = platformsIn(out);
  if (!platforms.size) fail(`no installers in ${out}`);
  await checkNewer(platforms, { allowEqual: true });

  const work = mkdtempSync(join(tmpdir(), `${config.slug}-site-`));
  const site = join(work, "site");
  cloneSite(site);
  const sha = output("git", ["rev-parse", "HEAD"]) ?? "";
  const notesFile = join(appDir, "release-notes", `${v}.md`);
  const changelogFile = join(appDir, "CHANGELOG.md");
  const summary = buildSite({
    site,
    out,
    name: NAME,
    version: v,
    sha,
    notes: existsSync(notesFile) ? readFileSync(notesFile, "utf8").trim() : "",
    changelog: existsSync(changelogFile) ? readFileSync(changelogFile, "utf8").trim() : "",
    siteUrl: SITE_URL,
    combinedFeed: config.combinedFeed,
    icon: join(appDir, config.icon),
    page: config.page ?? {},
    extras: config.extras ?? {},
    macMin: conf().bundle?.macOS?.minimumSystemVersion ?? "10.15",
    platforms,
  });

  if (dryRun) {
    console.log(`\nDry run: the site is in ${site} (not pushed). Open index.html there.`);
    return null;
  }
  pushSite(site, `${NAME} ${v}: ${[...platforms].join(", ")}${sha ? ` (${sha.slice(0, 7)})` : ""}`);
  rmSync(work, { recursive: true, force: true });
  console.log(`\nPublished ${NAME} ${v} for ${[...platforms].join(", ")}: ${summary.files.join(", ")}`);
  console.log(`${SITE_URL} (live in a minute or two)`);
  return { version: v, platforms };
}

// ---- clean --------------------------------------------------------------------------------------

function sizeOf(path) {
  const st = statSync(path);
  return st.isDirectory() ? readdirSync(path).reduce((sum, n) => sum + sizeOf(join(path, n)), 0) : st.size;
}

/**
 * Deletes what's left once releases are out: release/<version>/ folders whose platforms are all
 * published (unpublished builds are kept), Tauri's bundle output (rebuilt every time; the compiled
 * code stays, so the next build is still incremental) and site checkouts left by dry runs.
 */
async function clean(justPublished = null) {
  const { groups } = await publishedVersions();
  const isPublished = (v, p) =>
    (justPublished?.version === v && justPublished.platforms.has(p)) || GROUPS_BY_OS[p].every((g) => groups[g] && !newer(v, groups[g]));
  const doomed = [];
  const root = join(appDir, "release");
  if (existsSync(root)) {
    for (const v of readdirSync(root)) {
      if (!/^\d+\.\d+\.\d+$/.test(v)) continue;
      const platforms = platformsIn(join(root, v));
      if (platforms.size && [...platforms].every((p) => isPublished(v, p))) doomed.push(join(root, v));
      else console.log(`keeping release/${v} (not published yet)`);
    }
  }
  const target = targetDir();
  for (const { target: triple } of BUILDS[OS]) {
    const dir = join(target, ...(triple ? [triple] : []), "release", "bundle");
    if (existsSync(dir)) doomed.push(dir);
  }
  for (const name of readdirSync(tmpdir())) if (name.startsWith(`${config.slug}-site-`)) doomed.push(join(tmpdir(), name));
  let freed = 0;
  for (const path of doomed) {
    try {
      const size = sizeOf(path);
      rmSync(path, { recursive: true, force: true });
      freed += size;
      console.log(`removed ${path}`);
    } catch (err) {
      console.warn(`couldn't remove ${path}: ${err.message ?? err}`);
    }
  }
  console.log(doomed.length ? `Freed ${Math.round(freed / (1 << 20))} MB.` : "Nothing to clean.");
}

// ---- release ------------------------------------------------------------------------------------

async function release() {
  await build();
  if (isWindows && !flags.has("--no-linux")) await linux();
  const published = await publish();
  if (published) await clean(published);
}

// ---- main ---------------------------------------------------------------------------------------

const commands = {
  version: showVersion,
  bump,
  build,
  linux,
  "linux-setup": linuxSetup,
  publish,
  release,
  clean: () => clean(),
};
if (!commands[command]) {
  const header = readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n");
  console.log(header.slice(0, header.findIndex((l) => !l.startsWith("//"))).map((l) => l.slice(3)).join("\n"));
  process.exit(command === "help" ? 0 : 1);
}
await commands[command]();
