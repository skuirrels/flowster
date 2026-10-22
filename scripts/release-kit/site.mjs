// The download site and update feeds, built by `release.mjs publish` in a checkout of the releases
// repo's gh-pages branch. Publishing adds to the site: this build's platforms replace their own
// files and feeds, other platforms' stay as they are (each download keeps its own version), so
// every machine publishes on its own.
//
// The site holds:
//   stable names               downloads (Tessera-Windows-Setup.exe …) and macOS update bundles
//   updates/<os>-<arch>.json   one Tauri update feed per platform, which installed copies read first
//   <combined feed>            one feed for copies installed before the per-platform feeds
//   files.json                 each file's version and platform
//   release.json               name, version and downloads, for the GitHub release workflow below
//   index.html, icon.svg, SHA256SUMS.txt, README.md, RELEASE-NOTES.md, .github/workflows/release.yml
//   CHANGELOG.md               the app's CHANGELOG.md, when it has one
// Anything else is removed (older site layouts are moved into this one first).

import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, relative } from "node:path";

export const GROUPS_BY_OS = { windows: ["windows-x86_64"], macos: ["darwin-aarch64", "darwin-x86_64"], linux: ["linux-x86_64"] };
export const FEED_GROUPS = Object.values(GROUPS_BY_OS).flat();

const parse = (v) => String(v).split(".").map((n) => Number(n) || 0);
/** Whether version a is higher than b. */
export function newer(a, b) {
  const [x, y] = [parse(a), parse(b)];
  for (let i = 0; i < 3; i++) if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0);
  return false;
}

/**
 * Every file the site can hold: `from` matches the build output (or an older site's file), `file`
 * is its name on the site, `feeds` the updater keys it's the update bundle for. The first visible
 * file of a platform is its recommended download.
 */
export function catalog(name, extras = []) {
  return [
    { platform: "windows", from: /_x64-setup\.exe$/, file: `${name}-Windows-Setup.exe`, label: "Windows installer", detail: "64-bit · .exe",
      feeds: ["windows-x86_64", "windows-x86_64-nsis"], badge: ["Windows", "x64 Setup .exe", "0078D4", "windows"] },
    { platform: "windows", from: /_x64_en-US\.msi$/, file: `${name}-Windows.msi`, label: "Windows MSI package", detail: "64-bit · .msi · for managed deployment",
      feeds: ["windows-x86_64-msi"], badge: ["Windows", "x64 .msi", "005A9E", "windows"] },
    { platform: "macos", from: /_aarch64\.dmg$/, file: `${name}-macOS-AppleSilicon.dmg`, label: "Apple Silicon", detail: "M1 and later · .dmg", arch: "arm",
      badge: ["macOS", "Apple Silicon .dmg", "111111", "apple"] },
    { platform: "macos", from: /_x64\.dmg$/, file: `${name}-macOS-Intel.dmg`, label: "Intel", detail: "Older Macs · .dmg", arch: "x86",
      badge: ["macOS", "Intel .dmg", "555555", "apple"] },
    { platform: "macos", from: /_aarch64\.app\.tar\.gz$/, file: `${name}-macOS-AppleSilicon.app.tar.gz`, hidden: true, feeds: ["darwin-aarch64", "darwin-aarch64-app"] },
    { platform: "macos", from: /_x64\.app\.tar\.gz$/, file: `${name}-macOS-Intel.app.tar.gz`, hidden: true, feeds: ["darwin-x86_64", "darwin-x86_64-app"] },
    // Universal macOS builds of earlier releases, listed until the Mac publishes per-architecture ones.
    { platform: "macos", from: /_universal\.dmg$/, file: `${name}-macOS.dmg`, label: "Apple Silicon and Intel", detail: ".dmg", legacy: true,
      badge: ["macOS", "Universal .dmg", "111111", "apple"] },
    { platform: "macos", from: /_universal\.app\.tar\.gz$/, file: `${name}-macOS.app.tar.gz`, hidden: true, legacy: true },
    { platform: "linux", from: /\.AppImage$/, file: `${name}-Linux-x86_64.AppImage`, label: "AppImage", detail: "Any distribution · no install",
      feeds: ["linux-x86_64", "linux-x86_64-appimage"], badge: ["Linux", "x64 AppImage", "E95420", "linux"] },
    { platform: "linux", from: /\.deb$/, file: `${name}-Linux-amd64.deb`, label: "Debian / Ubuntu", detail: ".deb package",
      feeds: ["linux-x86_64-deb"], badge: ["Linux", "x64 .deb", "A81D33", "debian"] },
    { platform: "linux", from: /\.rpm$/, file: `${name}-Linux-x86_64.rpm`, label: "Fedora / RHEL / openSUSE", detail: ".rpm package",
      feeds: ["linux-x86_64-rpm"], badge: ["Linux", "x64 .rpm", "294172", "fedora"] },
    ...extras.map((e) => ({ ...e, extra: true })),
  ];
}

// GitHub refuses files over 100 MB in a push, and the site is published by pushing.
const MAX_PUSH_BYTES = 99 * 1024 * 1024;

const readJson = (path, fallback) => {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return fallback;
  }
};
const writeJson = (path, value) => {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify(value, null, 2) + "\n");
};

function walk(dir, root = dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? walk(p, root) : [relative(root, p).replace(/\\/g, "/")];
  });
}

const sha256 = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");

export function buildSite({ site, out, name, version, sha, notes, changelog, siteUrl, combinedFeed, icon, page, extras, macMin, platforms }) {
  const entries = catalog(name, extras.files ?? []);
  const byFile = new Map(entries.map((e) => [e.file, e]));
  const feedsDir = join(site, "updates");

  // Files on the site and their versions. Older files.json held only versions; files missing from
  // it (macOS update bundles) take the version of the feed that points at them.
  const feedVersionOf = (file) => {
    for (const g of walk(feedsDir)) {
      const feed = readJson(join(feedsDir, g), {});
      if (Object.values(feed.platforms ?? {}).some((p) => String(p.url).endsWith(`/${file}`))) return feed.version;
    }
    return null;
  };
  const files = {};
  const listed = readJson(join(site, "files.json"), {});
  for (const e of entries) {
    if (!existsSync(join(site, e.file))) continue;
    const info = listed[e.file];
    const v = typeof info === "string" ? info : info?.version;
    files[e.file] = { version: v ?? feedVersionOf(e.file) ?? readJson(join(site, combinedFeed), {}).version ?? "?", platform: e.platform };
  }

  // An older layout (Mapmeister's downloads/<tag>/<versioned name>) moves to stable names.
  const moved = new Map();
  for (const path of walk(join(site, "downloads"))) {
    const e = entries.find((x) => x.from?.test(basename(path)));
    if (!e || files[e.file]) continue;
    renameSync(join(site, "downloads", path), join(site, e.file));
    files[e.file] = { version: /(\d+\.\d+\.\d+)/.exec(basename(path))?.[1] ?? "?", platform: e.platform };
    moved.set(`downloads/${path}`, e.file);
  }
  rmSync(join(site, "downloads"), { recursive: true, force: true });
  const relink = (feed) => {
    for (const p of Object.values(feed?.platforms ?? {})) {
      for (const [from, to] of moved) if (String(p.url).endsWith(from)) p.url = siteUrl + to;
    }
    return feed;
  };
  const previous = relink(readJson(join(site, combinedFeed), null));

  // This build's platforms replace all their files (including formats no longer built).
  for (const [file, info] of Object.entries(files)) {
    if (!platforms.has(info.platform)) continue;
    rmSync(join(site, file), { force: true });
    delete files[file];
  }
  const signatures = {};
  const copied = [];
  for (const f of readdirSync(out).sort()) {
    const e = entries.find((x) => x.from?.test(f));
    if (!e || f.endsWith(".sig")) continue;
    const size = statSync(join(out, f)).size;
    if (size > MAX_PUSH_BYTES) {
      console.warn(`release: ${f} is ${size >> 20} MB, over GitHub's 100 MB push limit; it isn't published`);
      continue;
    }
    copyFileSync(join(out, f), join(site, e.file));
    files[e.file] = { version, platform: e.platform };
    copied.push(e.file);
    if (existsSync(join(out, `${f}.sig`))) signatures[e.file] = readFileSync(join(out, `${f}.sig`), "utf8").trim();
  }
  if (!copied.some((f) => !byFile.get(f).extra)) throw new Error(`no installers in ${out}`);

  // Update feeds: one per platform for this build's platforms; other platforms' are left alone.
  const now = new Date();
  now.setMilliseconds(0);
  const manifest = (v, platformsOf) => ({ version: v, notes: notes || `${name} ${v}`, pub_date: now.toISOString().replace(".000Z", "Z"), platforms: platformsOf });
  const built = {};
  for (const file of copied) {
    for (const key of byFile.get(file).feeds ?? []) if (signatures[file]) built[key] = { signature: signatures[file], url: siteUrl + file };
  }
  const groupOf = (key) => key.split("-").slice(0, 2).join("-");
  for (const p of platforms) {
    for (const g of GROUPS_BY_OS[p]) {
      const keys = Object.fromEntries(Object.entries(built).filter(([k]) => groupOf(k) === g));
      if (!Object.keys(keys).length) {
        console.warn(`release: no signed update bundle for ${g}; its installed copies won't be offered this version`);
        continue;
      }
      writeJson(join(feedsDir, `${g}.json`), manifest(version, keys));
    }
  }
  // A site from before per-platform feeds: other platforms' feeds start from the combined feed.
  for (const g of FEED_GROUPS) {
    if (existsSync(join(feedsDir, `${g}.json`)) || !previous) continue;
    const keys = Object.fromEntries(Object.entries(previous.platforms ?? {}).filter(([k]) => groupOf(k) === g));
    if (Object.keys(keys).length) writeJson(join(feedsDir, `${g}.json`), { ...previous, platforms: keys });
  }
  // The combined feed holds one version: platforms published for the same version join it, a newer
  // version starts it afresh (never another version's bundle), and it never moves backwards.
  let combined;
  if (previous && newer(previous.version, version)) {
    combined = previous;
    console.log(`release: ${combinedFeed} stays at ${previous.version} (newer than ${version})`);
  } else if (previous?.version === version) {
    combined = manifest(version, { ...previous.platforms, ...built });
  } else {
    combined = manifest(version, built);
  }
  writeJson(join(site, combinedFeed), combined);

  // Page, checksums, README, release notes and the GitHub release workflow.
  const visible = entries.filter((e) => files[e.file] && !e.hidden);
  const siteVersion = Object.values(files).map((f) => f.version).reduce((a, b) => (newer(b, a) ? b : a), version);
  const checksums = Object.fromEntries(visible.map((e) => [e.file, sha256(join(site, e.file))]));
  writeFileSync(join(site, "SHA256SUMS.txt"), visible.map((e) => `${checksums[e.file]}  ${e.file}\n`).join(""));
  if (icon && existsSync(icon)) copyFileSync(icon, join(site, "icon.svg"));
  writeJson(join(site, "files.json"), Object.fromEntries(Object.keys(files).sort().map((f) => [f, files[f]])));
  writeJson(join(site, "release.json"), { name, version: siteVersion, files: visible.map((e) => e.file) });
  const ctx = { name, version: siteVersion, sha, notes, changelog, siteUrl, page, extras, macMin, files, visible, checksums, site, now };
  writeFileSync(join(site, "index.html"), renderPage(ctx));
  writeFileSync(join(site, "README.md"), renderReadme(ctx));
  writeFileSync(join(site, "RELEASE-NOTES.md"), renderReleaseNotes(ctx));
  if (changelog) writeFileSync(join(site, "CHANGELOG.md"), `${changelog}\n`);
  mkdirSync(join(site, ".github", "workflows"), { recursive: true });
  writeFileSync(join(site, ".github", "workflows", "release.yml"), WORKFLOW);
  writeFileSync(join(site, ".nojekyll"), "");

  // Remove everything else (older layouts, signatures, stale files).
  const keep = new Set([
    ...Object.keys(files),
    combinedFeed, "files.json", "release.json", "index.html", "icon.svg", "SHA256SUMS.txt", "README.md", "RELEASE-NOTES.md",
    ".nojekyll", ".github/workflows/release.yml", ...(changelog ? ["CHANGELOG.md"] : []),
  ]);
  for (const path of walk(site)) {
    if (!keep.has(path) && !/^updates\/[\w-]+\.json$/.test(path)) rmSync(join(site, path), { force: true });
  }
  return { files: copied, version: siteVersion };
}

// ---- page ---------------------------------------------------------------------------------------

const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const mb = (path) => `${(statSync(path).size / 1048576).toFixed(1)} MB`;

/** A small Markdown subset for release notes: headings, bullets, paragraphs, `code`, **bold**, links. */
function markdown(md) {
  const inline = (s) =>
    esc(s)
      .replace(/`([^`]+)`/g, "<code>$1</code>")
      .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
      .replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, '<a href="$2">$1</a>');
  const html = [];
  let list = false;
  for (const line of String(md || "").split(/\r?\n/)) {
    const t = line.trim();
    const bullet = /^[-*] (.*)/.exec(t);
    if (bullet) {
      if (!list) html.push("<ul>");
      list = true;
      html.push(`<li>${inline(bullet[1])}</li>`);
      continue;
    }
    if (list) html.push("</ul>");
    list = false;
    const h = /^#{1,4} (.*)/.exec(t);
    if (h) html.push(`<h4>${inline(h[1])}</h4>`);
    else if (t) html.push(`<p>${inline(t)}</p>`);
  }
  if (list) html.push("</ul>");
  return html.join("\n");
}

/** The change log without its title, headings one level down: `## 1.2.3 · date` becomes `### 1.2.3 · date`. */
const changelogBody = (md) => md.replace(/^# .*\r?\n+/, "").replace(/^## /gm, "### ");

/** The page's change log: each version a heading with its date muted. */
const changelogHtml = (md) =>
  markdown(changelogBody(md)).replace(/<h4>(.*?) · (.*?)<\/h4>/g, '<h4>$1 <span class="date">$2</span></h4>');

/** How installed copies find updates (page.updateNote overrides it for an app without a Help menu item). */
const updateNote = (ctx) =>
  ctx.page.updateNote ?? `${ctx.name} checks for a newer version when it starts, and Help ▸ Check for Updates checks on demand.`;

const OS_CARDS = [
  ["windows", "Windows", () => "Windows 10 and 11, 64-bit",
    '<path d="M3 5.5 10.5 4.4v7.1H3zm0 13 7.5 1.1v-7H3zM11.6 4.3 21 3v8.5h-9.4zm0 15.4L21 21v-8.4h-9.4z"/>'],
  ["macos", "macOS", (ctx) => `macOS ${ctx.macMin} or newer`,
    '<path d="M16.4 12.6c0-2.5 2-3.6 2.1-3.7-1.2-1.7-3-1.9-3.6-2-1.5-.2-3 .9-3.8.9s-2-.9-3.3-.9C6.1 7 4.5 8 3.6 9.6c-1.8 3.1-.5 7.7 1.3 10.2.9 1.2 1.9 2.6 3.2 2.6s1.8-.8 3.3-.8 2 .8 3.3.8 2.2-1.3 3-2.5c1-1.4 1.4-2.8 1.4-2.9-.1 0-2.7-1-2.7-4.4zM13.9 5.1c.7-.8 1.2-2 1-3.1-1 0-2.2.7-2.9 1.5-.6.7-1.2 1.9-1 3 1.1.1 2.2-.6 2.9-1.4z"/>'],
  ["linux", "Linux", () => "x86-64 · Ubuntu 22.04 or newer, Debian, Fedora, …",
    '<path d="M12 2c-2.2 0-3.6 1.8-3.6 4.4 0 1.5.3 2.4-.6 3.7C6.6 11.8 5 13.9 5 16.4c0 .7.1 1.3.4 1.8-.8.4-1.9.6-1.9 1.6 0 1.3 1.9 1.1 3.3 1.7 1.2.5 2.2.5 2.8-.3.8.2 1.6.3 2.4.3s1.6-.1 2.4-.3c.6.8 1.6.8 2.8.3 1.4-.6 3.3-.4 3.3-1.7 0-1-1.1-1.2-1.9-1.6.3-.5.4-1.1.4-1.8 0-2.5-1.6-4.6-2.8-6.3-.9-1.3-.6-2.2-.6-3.7C15.6 3.8 14.2 2 12 2zm-1.6 4.3c.5 0 .8.6.8 1.2 0 .3-.1.6-.2.8l-.4-.1c.1-.2.1-.4.1-.6 0-.4-.2-.7-.4-.7s-.4.3-.4.7v.2l-.4-.2v-.1c0-.7.4-1.2.9-1.2zm3.1 0c.6 0 1 .5 1 1.2 0 .2 0 .3-.1.5l-.3.1v-.4c0-.5-.3-.8-.6-.8s-.5.4-.5.8c0 .2 0 .4.1.5l-.5.2c-.1-.2-.2-.5-.2-.8 0-.7.5-1.3 1.1-1.3zM12 9.1c.9 0 2.3.6 2.3 1s-1.3 1.2-2.3 1.2-2.3-.8-2.3-1.2 1.4-1 2.3-1zm-3.1 3.5c.5 1.6 1.6 3 3.1 3s2.6-1.4 3.1-3c.8 1 1.6 2.5 1.6 3.9 0 2.4-2.1 3.4-4.7 3.4s-4.7-1-4.7-3.4c0-1.4.8-2.9 1.6-3.9z"/>'],
];

function downloadLink(ctx, e, primary) {
  const v = ctx.files[e.file].version;
  const meta = [e.detail, v !== ctx.version ? `version ${v}` : "", mb(join(ctx.site, e.file))].filter(Boolean).join(" · ");
  return `          <li>
            <a class="dl${primary ? " primary" : ""}" href="${esc(e.file)}" download data-arch="${e.arch ?? ""}">
              <span class="name">${esc(e.label)}</span>
              <span class="meta">${esc(meta)}</span>
              <svg class="arrow" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4v12m0 0-5-5m5 5 5-5M5 20h14"/></svg>
            </a>
            <details><summary>SHA-256</summary><code>${ctx.checksums[e.file]}</code></details>
          </li>`;
}

function renderPage(ctx) {
  const { name, version, page } = ctx;
  const cards = OS_CARDS.map(([os, title, sub, icon]) => {
    const list = ctx.visible.filter((e) => e.platform === os && !e.extra);
    if (!list.length) return "";
    return `      <article class="card" id="${os}">
        <span class="badge">Recommended for you</span>
        <header>
          <svg viewBox="0 0 24 24" aria-hidden="true">${icon}</svg>
          <div><h3>${title}</h3><p>${esc(sub(ctx))}</p></div>
        </header>
        <ul class="files">
${list.map((e, i) => downloadLink(ctx, e, i === 0)).join("\n")}
        </ul>
      </article>`;
  }).filter(Boolean).join("\n");
  const extraFiles = ctx.visible.filter((e) => e.extra);
  const tones = ["amber", "teal", "pink", "blue"];
  const features = (page.features ?? [])
    .map(([title, text], i) => `        <div class="feature ${tones[i % tones.length]}"><b>${esc(title)}</b><span>${esc(text)}</span></div>`)
    .join("\n");
  const accent = page.accent ?? ["#1d4ed8", "#6b9bff"];
  const files = (os) => ctx.visible.filter((e) => e.platform === os && !e.extra).map((e) => e.file);
  const linux = files("linux");
  const date = `${ctx.now.getUTCDate()} ${ctx.now.toLocaleString("en-GB", { month: "long", timeZone: "UTC" })} ${ctx.now.getUTCFullYear()}`;

  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(name)} · Download</title>
<meta name="description" content="${esc(name)} ${esc(version)}: ${esc(page.description ?? "")} Installers for Windows, macOS and Linux.">
<link rel="icon" href="icon.svg" type="image/svg+xml">
<style>
  :root {
    --bg: #f5f7fb; --panel: #ffffff; --text: #18202f; --muted: #5d6779; --border: #dde3ec;
    --accent: ${accent[0]}; --accent-2: color-mix(in srgb, ${accent[0]} 10%, #fff); --glow: color-mix(in srgb, ${accent[0]} 18%, transparent);
    --shadow: 0 10px 30px rgba(20, 40, 80, .08);
    --amber: #f59e0b; --teal: #14b8a6; --pink: #ec4899; --blue: #3b82f6;
    color-scheme: light;
  }
  @media (prefers-color-scheme: dark) {
    :root:not([data-theme="light"]) {
      --bg: #0c111a; --panel: #141b27; --text: #e6eaf1; --muted: #98a3b6; --border: #253043;
      --accent: ${accent[1]}; --accent-2: color-mix(in srgb, ${accent[1]} 14%, #141b27); --glow: color-mix(in srgb, ${accent[1]} 22%, transparent);
      --shadow: 0 10px 30px rgba(0, 0, 0, .35);
      --amber: #fbbf24; --teal: #2dd4bf; --pink: #f472b6; --blue: #60a5fa;
      color-scheme: dark;
    }
  }
  * { box-sizing: border-box; }
  body { margin: 0; background: var(--bg); color: var(--text); font: 16px/1.55 system-ui, -apple-system, "Segoe UI", "Helvetica Neue", Arial, sans-serif; }
  a { color: var(--accent); }
  code, pre { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
  .wrap { max-width: 1080px; margin: 0 auto; padding: 0 20px; }
  .hero { position: relative; overflow: hidden; padding: 72px 0 48px; text-align: center;
          background: radial-gradient(800px 320px at 50% -60px, var(--glow), transparent 70%); }
  .hero::before { content: ""; position: absolute; inset: 0; pointer-events: none; opacity: .5;
          background-image: linear-gradient(var(--border) 1px, transparent 1px), linear-gradient(90deg, var(--border) 1px, transparent 1px);
          background-size: 44px 44px; mask-image: radial-gradient(600px 260px at 50% 0, #000, transparent 75%);
          -webkit-mask-image: radial-gradient(600px 260px at 50% 0, #000, transparent 75%); }
  .hero .wrap { position: relative; }
  .hero img { width: 104px; height: 104px; filter: drop-shadow(0 10px 24px var(--glow)); animation: float 6s ease-in-out infinite; }
  @keyframes float { 50% { transform: translateY(-6px); } }
  @media (prefers-reduced-motion: reduce) { .hero img { animation: none; } }
  h1 { font-size: clamp(2.4rem, 6vw, 3.4rem); margin: 14px 0 6px; letter-spacing: -.03em; }
  .tagline { font-size: 1.2rem; color: var(--muted); max-width: 640px; margin: 0 auto 28px; }
  .cta { display: inline-flex; flex-direction: column; align-items: center; gap: 2px; padding: 14px 34px; border-radius: 12px;
         background: ${accent[0]}; color: #fff; text-decoration: none; font-weight: 600; font-size: 1.1rem;
         box-shadow: 0 8px 24px var(--glow); transition: transform .15s, filter .15s; }
  .cta small { font-weight: 400; opacity: .85; font-size: .82rem; }
  .cta:hover { transform: translateY(-1px); filter: brightness(1.08); }
  .version { margin-top: 16px; color: var(--muted); font-size: .9rem; }
  .other { margin-top: 6px; font-size: .9rem; }
  section { padding: 34px 0; }
  h2 { font-size: 1.5rem; margin: 0 0 18px; letter-spacing: -.01em; }
  .grid { display: grid; gap: 18px; grid-template-columns: repeat(auto-fit, minmax(290px, 1fr)); align-items: start; }
  .card { position: relative; background: var(--panel); border: 1px solid var(--border); border-radius: 14px; padding: 18px; box-shadow: var(--shadow); }
  .card.mine { border-color: var(--accent); box-shadow: 0 0 0 3px var(--glow), var(--shadow); }
  .card header { display: flex; align-items: center; gap: 12px; margin-bottom: 14px; }
  .card header svg { width: 34px; height: 34px; fill: var(--text); flex: none; }
  .card h3 { margin: 0; font-size: 1.2rem; }
  .card header p { margin: 0; color: var(--muted); font-size: .85rem; }
  .badge { display: none; position: absolute; top: -11px; right: 14px; font-size: .72rem; font-weight: 600; color: #fff;
           background: var(--accent); border-radius: 20px; padding: 3px 10px; white-space: nowrap; }
  .card.mine .badge { display: inline-block; }
  .files { list-style: none; margin: 0; padding: 0; display: grid; gap: 10px; }
  .dl { position: relative; display: flex; flex-direction: column; padding: 11px 44px 11px 14px; border: 1px solid var(--border); border-radius: 10px;
        text-decoration: none; color: var(--text); background: var(--bg); transition: border-color .15s, transform .15s; }
  .dl:hover { border-color: var(--accent); transform: translateY(-1px); }
  .dl.primary { border-color: var(--accent); background: var(--accent-2); }
  .dl .name { font-weight: 600; }
  .dl .meta { font-size: .82rem; color: var(--muted); }
  .dl .arrow { position: absolute; right: 14px; top: 50%; width: 18px; height: 18px; margin-top: -9px; fill: none;
               stroke: var(--muted); stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
  .dl:hover .arrow { stroke: var(--accent); }
  details { margin: 4px 2px 0; font-size: .75rem; color: var(--muted); }
  summary { cursor: pointer; }
  details code { display: block; word-break: break-all; font-size: .72rem; }
  .features { display: grid; gap: 14px; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); }
  .feature { background: var(--panel); border: 1px solid var(--border); border-radius: 12px; padding: 16px; border-top: 3px solid var(--tone); }
  .feature.amber { --tone: var(--amber); } .feature.teal { --tone: var(--teal); }
  .feature.pink { --tone: var(--pink); } .feature.blue { --tone: var(--blue); }
  .feature b { display: block; margin-bottom: 4px; }
  .feature span { color: var(--muted); font-size: .92rem; }
  .extras { display: grid; gap: 10px; grid-template-columns: repeat(auto-fit, minmax(250px, 1fr)); list-style: none; margin: 0; padding: 0; }
  .help { display: grid; gap: 14px; grid-template-columns: repeat(auto-fit, minmax(290px, 1fr)); }
  .help div, .notes { background: var(--panel); border: 1px solid var(--border); border-radius: 12px; padding: 14px 16px; font-size: .92rem; min-width: 0; }
  .notes { padding: 6px 20px; font-size: 1rem; }
  .notes h4 { margin: 16px 0 6px; }
  .notes h4 .date { font-weight: 400; color: var(--muted); font-size: .85rem; margin-left: 6px; }
  .notes ul { margin: 0 0 8px; padding-left: 20px; }
  .notes li { margin: 3px 0; }
  .help h3 { margin: 0 0 6px; font-size: 1rem; }
  pre { background: var(--bg); border: 1px solid var(--border); border-radius: 8px; padding: 8px 10px; overflow-x: auto; font-size: .8rem; }
  .update { display: flex; gap: 14px; align-items: center; background: var(--panel); border: 1px solid var(--border); border-radius: 12px; padding: 14px 18px; }
  .update svg { width: 28px; height: 28px; flex: none; fill: none; stroke: var(--teal); stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
  footer { padding: 34px 0 50px; color: var(--muted); font-size: .85rem; text-align: center; }
  @media (max-width: 520px) { .hero { padding-top: 44px; } .wrap { padding: 0 16px; } }
</style>
</head>
<body>
  <div class="hero">
    <div class="wrap">
      <img src="icon.svg" alt="">
      <h1>${esc(name)}</h1>
      <p class="tagline">${page.tagline ?? ""}</p>
      <a class="cta" id="cta" href="#downloads">Download ${esc(name)} <small id="cta-sub">Windows · macOS · Linux</small></a>
      <div class="version">Version ${esc(version)} · ${esc(date)} · ${esc(page.license ?? "free download")}</div>
      <div class="other"><a href="#downloads">All downloads</a> · <a href="SHA256SUMS.txt">Checksums</a>${ctx.changelog ? ` · <a href="#changelog">Change log</a>` : ""}</div>
    </div>
  </div>

  <main class="wrap">
    <section id="downloads">
      <h2>Downloads</h2>
      <div class="grid">
${cards}
      </div>
    </section>

    <section>
      <div class="update">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M21 12a9 9 0 0 1-15.5 6.2M3 12a9 9 0 0 1 15.5-6.2M18.5 2v3.8h-3.8M5.5 22v-3.8h3.8"/></svg>
        <div><b>Installed copies update themselves.</b> ${esc(updateNote(ctx))}</div>
      </div>
    </section>
${features ? `
    <section>
      <h2>What you get</h2>
      <div class="features">
${features}
      </div>
    </section>
` : ""}${extraFiles.length ? `
    <section>
      <h2>${esc(ctx.extras.title ?? "More downloads")}</h2>
      ${ctx.extras.intro ? `<p>${ctx.extras.intro}</p>` : ""}
      <ul class="extras">
${extraFiles.map((e) => downloadLink(ctx, e, false)).join("\n")}
      </ul>
    </section>
` : ""}
    <section>
      <h2>Install notes</h2>
      <div class="help">
${files("windows").length ? `        <div><h3>Windows</h3>Run the installer. If SmartScreen says “Windows protected your PC”, choose <b>More info → Run anyway</b> (the build is not code-signed yet). The .msi suits managed installs.</div>
` : ""}${files("macos").length ? `        <div><h3>macOS</h3>Open the .dmg and drag ${esc(name)} to Applications. The first time, <b>right-click the app → Open</b> (the build is not notarized yet).</div>
` : ""}${linux.length ? `        <div><h3>Linux</h3>Make the AppImage executable and run it, or install the package for your distribution.
          <pre>${linux.map((f) => (f.endsWith(".AppImage") ? `chmod +x ${f} && ./${f}` : f.endsWith(".deb") ? `sudo apt install ./${f}` : `sudo dnf install ./${f}`)).map(esc).join("\n")}</pre></div>
` : ""}      </div>
    </section>
${ctx.notes ? `
    <section>
      <h2>What's new in ${esc(version)}</h2>
      <div class="notes">${markdown(ctx.notes)}</div>
    </section>
` : ""}${ctx.changelog ? `
    <section id="changelog">
      <h2>Change log</h2>
      <div class="notes">${changelogHtml(ctx.changelog)}</div>
    </section>
` : ""}  </main>

  <footer>${esc(name)} ${esc(version)} · built from ${esc(ctx.sha.slice(0, 7)) || "a local checkout"}${page.footer ? ` · ${page.footer}` : ""}</footer>

<script>
  // Highlight the visitor's system and point the main button at its recommended installer.
  (function () {
    var ua = navigator.userAgent || "";
    var platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
    var os = /Win/i.test(platform + ua) ? "windows" : /Mac|iPhone|iPad/i.test(platform + ua) ? "macos" : /Linux|X11|CrOS/i.test(platform + ua) ? "linux" : null;
    var card = os && document.getElementById(os);
    if (!card) return;
    card.classList.add("mine");
    var pick = card.querySelector(".dl.primary") || card.querySelector(".dl");
    var finish = function (link) {
      var cta = document.getElementById("cta");
      cta.href = link.getAttribute("href");
      cta.setAttribute("download", "");
      cta.firstChild.textContent = "Download for " + card.querySelector("h3").textContent + " ";
      var meta = link.querySelector(".meta").textContent.split(" · ");
      document.getElementById("cta-sub").textContent = link.querySelector(".name").textContent + " · " + meta[meta.length - 1];
    };
    // Apple Silicon vs Intel: ask the browser when it can tell, otherwise assume Apple Silicon.
    if (os === "macos" && navigator.userAgentData && navigator.userAgentData.getHighEntropyValues) {
      navigator.userAgentData.getHighEntropyValues(["architecture"]).then(function (v) {
        finish(card.querySelector('.dl[data-arch="' + (v.architecture === "x86" ? "x86" : "arm") + '"]') || pick);
      }).catch(function () { finish(pick); });
    } else {
      finish(pick);
    }
  })();
</script>
</body>
</html>
`;
}

/** The releases repo's front page: one-click downloads (files listed as code would open a viewer). */
function renderReadme(ctx) {
  const shield = (t) => encodeURIComponent(t.replace(/-/g, "--").replace(/_/g, "__"));
  const badges = ctx.visible
    .filter((e) => e.badge)
    .map((e) => {
      const [platform, kind, colour, logo] = e.badge;
      const v = ctx.files[e.file].version;
      const alt = `${platform} ${kind} (${mb(join(ctx.site, e.file))}${v !== ctx.version ? `, version ${v}` : ""})`;
      return `[![${alt}](https://img.shields.io/badge/${shield(platform)}-${shield(kind)}-${colour}?logo=${logo}&logoColor=white)](${ctx.siteUrl}${e.file})`;
    })
    .join("\n");
  return `# Download ${ctx.name}

${ctx.page.description ?? ""} Version **${ctx.version}**.

Click a badge to download:

${badges}

The same files are attached to the [latest release](../../releases/latest) and listed on the download page, ${ctx.siteUrl}

- **Windows:** the Setup .exe is recommended; the .msi suits managed installs. If SmartScreen warns, choose **More info → Run anyway**.
- **macOS:** Apple Silicon is for M1 and later Macs, Intel for older ones. Open the .dmg and drag ${ctx.name} to Applications. The first time, right-click ${ctx.name} and choose **Open** (the builds aren't notarized yet).
- **Linux:** the AppImage runs on any distribution; use the .deb on Debian/Ubuntu and the .rpm on Fedora/openSUSE.

Installed copies update themselves: ${updateNote(ctx)}
${ctx.changelog ? `
## Change log

What changed in each version: [CHANGELOG.md](CHANGELOG.md), also on the [download page](${ctx.siteUrl}#changelog).
` : ""}`;
}

function renderReleaseNotes(ctx) {
  const list = ctx.visible.map((e) => `- \`${e.file}\`${e.label ? `: ${e.label}` : ""}${ctx.files[e.file].version !== ctx.version ? ` (version ${ctx.files[e.file].version})` : ""}`);
  return `${ctx.name} ${ctx.version}${ctx.sha ? `, built from ${ctx.sha.slice(0, 7)}` : ""}.

${ctx.notes ? `${ctx.notes}\n\n` : ""}${list.join("\n")}

Installed copies update themselves: ${updateNote(ctx)} Download page: ${ctx.siteUrl}
${ctx.changelog ? `
## Change log

${changelogBody(ctx.changelog)}
` : ""}`;
}

/** Runs in the releases repo (public, so it costs no Actions minutes): its latest GitHub release is this site's files. */
const WORKFLOW = `name: GitHub release
on:
  push:
    branches: [gh-pages]
permissions:
  contents: write
concurrency:
  group: release
  cancel-in-progress: true
jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Publish the downloads as the latest release
        env:
          GH_TOKEN: \${{ github.token }}
        run: |
          name=$(jq -r .name release.json)
          version=$(jq -r .version release.json)
          tag="v$version"
          mapfile -t files < <(jq -r '.files[]' release.json)
          if [ \${#files[@]} -eq 0 ]; then echo "::error::no downloads to publish"; exit 1; fi
          gh release delete "$tag" --yes --cleanup-tag 2>/dev/null || true
          gh release create "$tag" --title "$name $version" --notes-file RELEASE-NOTES.md --latest --target "$GITHUB_SHA" "\${files[@]}"
          # Only the newest release is kept.
          gh release list --limit 100 --json tagName -q '.[].tagName' | grep -vx "$tag" | while read -r old; do
            gh release delete "$old" --yes --cleanup-tag
          done
`;
