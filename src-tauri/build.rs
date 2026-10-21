fn main() {
    // Declaring the app's commands in a manifest makes each one permission-gated:
    // a window can call a command only if its capability grants `allow-<command>`
    // (see capabilities/main.json). Without this, every command is open to every window.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
        "pick_document",
        "read_picked_document",
        "save_document",
        "export_pdf",
        "set_document_state",
        "check_for_updates",
    ])))
    .expect("tauri build script");
}
