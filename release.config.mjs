// How Flowster is released (scripts/release-kit, see its README.md).
export default {
  // ~/.tauri/flowster-updater.key, the WSL build clone and temporary folders.
  slug: "flowster",
  siteRepo: "https://github.com/skuirrels/flowster-releases.git",
  siteUrl: "https://skuirrels.github.io/flowster-releases/",
  combinedFeed: "latest.json",
  // Files whose version follows src-tauri/tauri.conf.json (`make bump` sets them all, and Cargo.lock).
  versionFiles: ["Cargo.toml", "ui/package.json"],
  icon: "src-tauri/icon-source.svg",
  // The WASM engine build (scripts/build-wasm.mjs) in the WSL Linux build: the wasm32 target comes
  // from rust-toolchain.toml; wasm-bindgen-cli must match the wasm-bindgen pin in Cargo.toml.
  linuxSetup: `rustup toolchain install
command -v wasm-bindgen >/dev/null && [ "$(wasm-bindgen --version)" = "wasm-bindgen 0.2.129" ] || cargo install wasm-bindgen-cli --version 0.2.129 --locked
echo "provision-wsl: Flowster's WASM tools ready"`,
  page: {
    description: "A cross-platform diagramming application that reads and writes Visio files.",
    tagline: "A cross-platform diagramming app that opens Visio files. Early preview: viewing .vsdx documents, with editing and real-time co-editing to come.",
    license: "early preview",
    accent: ["#0f766e", "#2dd4bf"],
    features: [
      ["Opens Visio files", ".vsdx, .vssx and .vstx, including macro-enabled ones, through a package reader hardened against malformed files."],
      ["One engine everywhere", "The document engine is Rust compiled to WebAssembly, with the same results on Windows, macOS and Linux."],
      ["Locked-down host", "The app can only read files you pick; the window gets no other access to your system."],
    ],
    // No Help menu yet: the check runs at start-up only (src-tauri/src/lib.rs).
    updateNote: "Flowster checks for a newer version when it starts and offers to install it.",
    footer: "Visio is a trademark of Microsoft Corporation. Flowster is an independent project and is not affiliated with Microsoft.",
  },
};
