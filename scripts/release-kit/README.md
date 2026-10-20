# Building and publishing releases (release-kit)

Every Tauri app (Mapmeister, Tessera, …) is released the same way, from two machines, with no
GitHub Actions minutes:

| Machine | `make release` builds | |
|---|---|---|
| Windows PC | Windows: setup `.exe` + `.msi` | then Linux, in WSL (Ubuntu 22.04): `.AppImage` + `.deb` + `.rpm` |
| Mac | macOS Apple Silicon and Intel: a `.dmg` each | |

Each `make release` publishes its platforms to the app's download site (GitHub Pages of a public
`<app>-releases` repo) and its update feeds. Then it cleans up.

## A release

1. On either machine, set the next version, commit it and push it. Both machines then build the same version.
   ```bash
   make bump
   ```
   ```bash
   git commit -am "Version 1.2.3" && git push
   ```
   You can also add release notes in `release-notes/<version>.md` before committing. They appear on the download page and in the update dialog.
2. On the Windows PC:
   ```bash
   git pull && make release
   ```
3. On the Mac:
   ```bash
   git pull && make release
   ```

The order doesn't matter, and neither machine waits for the other. Each platform has its own
update feed, so installed copies of a platform are offered the new version as soon as that
platform is published. They are never offered another platform's build.

## The commands

Each app's Makefile has the same targets. They run `node scripts/release-kit/release.mjs <command>`,
so they also work without make.

| `make …` | |
|---|---|
| `version` | this checkout's version and what's published for each platform |
| `bump` | the next version (published + 1, or `make bump VERSION=1.3.0`) in every file that records it, plus `Cargo.lock` |
| `installers` | this OS's installers (and the app's extras, e.g. Mapmeister's CLI tools) into `release/<version>/` |
| `linux` | Windows only: the Linux installers, built in WSL from the last commit, into the same folder |
| `publish` | adds `release/<version>/` to the download site and update feeds, and pushes |
| `publish-dry-run` | builds the site in a temporary folder without pushing; it prints where |
| `release` | `installers`, `linux` (on Windows), `publish`, `clean` |
| `clean` | deletes published `release/<version>/` folders, Tauri's bundle output and dry-run sites |

`ARGS=--force` builds or publishes a version that isn't newer than the published one.
`ARGS=--allow-dirty` builds with uncommitted changes, and `ARGS=--no-linux` skips the WSL build
in `make release`.

The safeguards:
- **No uncommitted changes.** A build refuses uncommitted changes, because both machines must build the same commit.
- **Versions only move forward.** A build refuses a version that isn't newer than the one already published for its platform. Publishing never moves a platform's feed backwards.

## What gets published

- **Installers under stable names**, e.g. `Tessera-Windows-Setup.exe`, `Tessera-macOS-AppleSilicon.dmg`, `Tessera-Linux-x86_64.AppImage`. Links to them never change.
- **Per-platform update feeds** at `updates/<os>-<arch>.json`. Installed copies read these first (`updates/{{target}}-{{arch}}.json` in `plugins.updater.endpoints`).
- **A combined feed** (`latest.json` or `update.json`) for copies installed before the per-platform feeds. It holds one version, never moves backwards, and lists the platforms published for that version.
- **Everything else:** `index.html` (the download page), `SHA256SUMS.txt`, `files.json` (each file's version), `release.json`, a `README.md` with download badges, and a workflow. On each publish, that workflow makes the releases repo's latest GitHub release hold the same files. The repo is public, so it costs no minutes.

Downloads not yet rebuilt for the newest version stay on the page, labelled with their version.
GitHub refuses files over 100 MB in a push, so a file that size is left out with a warning.

## Setting up a machine

Both machines:
- Node 22+, Rust (stable), git and GNU make.
  - On Windows, install make with `winget install ezwinports.make`.
- **The updater signing keys**, copied to every build machine and backed up:
  - `~/.tauri/<app>-updater.key`
  - `~/.tauri/<app>-updater.key.password`

  Installed copies refuse updates that aren't signed with the key matching `plugins.updater.pubkey`. If the key is lost, every user has to reinstall.
- Push access to the `<app>-releases` repos, using your own git credentials. Without the `workflow` scope, a publish still works, but it leaves out the releases repo's workflow.

Windows PC, once, for the Linux build:
```bash
wsl --install -d Ubuntu-22.04
```
That asks you to create a user. Then, in one of the app repos:
```bash
make linux-setup
```
This installs the Tauri build packages, Rust and Node in that distro. The Linux build works in
its own clone, `~/release-builds/<app>` inside WSL. It is checked out at the Windows checkout's
commit, has its own `node_modules` and `target/`, and stays incremental between releases. The
signing key is read from the Windows `~/.tauri`.

Mac: the Rust targets for both architectures are added on the first build.

## Code signing (optional)

Without it, the installers work, but Windows SmartScreen and macOS Gatekeeper warn about them on
first launch. The download page explains what to click.

- **Windows:** set `bundle.windows.certificateThumbprint` or a `signCommand` in `tauri.conf.json`.
- **macOS:** with an Apple Developer certificate in the Mac's keychain, set `APPLE_SIGNING_IDENTITY` (plus `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID` to notarize) before `make release`.

## Adding an app

1. From the release-kit repo, copy the kit into the app:
   ```bash
   node sync.mjs ../MyApp
   ```
2. Write `release.config.mjs` next to the app's `package.json`. Mapmeister's is a full example; it has extras. Flowster's uses two optional settings:
   - `linuxSetup`: shell lines that `make linux-setup` runs in WSL after the base tools, e.g. extra Rust targets or CLIs.
   - `page.updateNote`: how installed copies find updates, for an app without *Help ▸ Check for Updates*.

   An app with a `pnpm-lock.yaml` is built with pnpm.
3. Copy the Makefile targets from another app.
4. In `tauri.conf.json`, set `bundle.createUpdaterArtifacts: true` and the updater `pubkey`, and use these endpoints:
   `["<site>/updates/{{target}}-{{arch}}.json", "<site>/<combinedFeed>"]`
5. Create the public `<app>-releases` repo with GitHub Pages serving its `gh-pages` branch. The first publish creates that branch.

## Changing the kit

This folder is a copy. Change it in the release-kit repo, then copy it into every app and commit there:
```bash
node sync.mjs <app>
```
`make version` prints each app's kit version.
