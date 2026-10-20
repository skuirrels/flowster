//! Flowster desktop host (SPEC §5.1, §21).
//!
//! The live document runs in the WebView's Engine Worker. The host only
//! provides OS services, and each one is a permission-gated command (see
//! `build.rs` and `capabilities/main.json`). M0 has exactly one service:
//! opening a file the user picks.
//!
//! The webview never names a path. `pick_document` shows the native dialog on
//! the host side and returns a one-time token; `read_picked_document` returns
//! that file's bytes as a raw binary IPC payload (no base64, §21) and forgets
//! the token. A compromised page can therefore read only files the user just picked.
//!
//! Updates (§21, signed Tauri updater) are the host's own business: release builds check the
//! signed update feed at start-up and ask in a native dialog before installing.
//! The webview gets no updater permission.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::UpdaterExt;

/// Matches the engine's own cap on a package (flowster_vsdx::Limits).
const MAX_DOCUMENT_BYTES: u64 = 2 << 30;

const DOCUMENT_EXTENSIONS: &[&str] = &["vsdx", "vssx", "vstx", "vsdm", "vssm", "vstm", "vsd", "vss", "vst"];

#[derive(Default)]
struct PickedFiles {
    next: AtomicU64,
    paths: Mutex<HashMap<u64, PathBuf>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PickedDocument {
    token: u64,
    file_name: String,
}

#[tauri::command]
async fn pick_document(app: AppHandle, picked: State<'_, PickedFiles>) -> Result<Option<PickedDocument>, String> {
    let dialog = app.dialog().clone();
    let choice = tauri::async_runtime::spawn_blocking(move || {
        dialog.file().set_title("Open").add_filter("Visio documents", DOCUMENT_EXTENSIONS).blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(choice) = choice else { return Ok(None) };
    let path = choice.into_path().map_err(|e| e.to_string())?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let token = picked.next.fetch_add(1, Ordering::Relaxed) + 1;
    picked.paths.lock().map_err(|e| e.to_string())?.insert(token, path);
    Ok(Some(PickedDocument { token, file_name }))
}

/// Shows the platform's save dialog and writes the document the page sends
/// (a raw binary IPC body). The page suggests a file name but never sees or
/// chooses the path, so it can only write where the user just said to.
#[tauri::command]
async fn save_document(app: AppHandle, request: tauri::ipc::Request<'_>) -> Result<Option<String>, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected the document bytes".into());
    };
    let bytes = bytes.clone();
    // Only a default for the dialog: keep the last path component.
    let suggested = request
        .headers()
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .and_then(|n| n.rsplit(['/', '\\']).next())
        .filter(|n| !n.is_empty())
        .unwrap_or("Drawing.vsdx")
        .to_owned();
    let dialog = app.dialog().clone();
    let choice = tauri::async_runtime::spawn_blocking(move || {
        dialog
            .file()
            .set_title("Save")
            .set_file_name(&suggested)
            .add_filter("Visio drawing", &["vsdx"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(choice) = choice else { return Ok(None) };
    let path = choice.into_path().map_err(|e| e.to_string())?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || std::fs::write(&path, &bytes))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(Some(file_name))
}

#[tauri::command]
async fn read_picked_document(token: u64, picked: State<'_, PickedFiles>) -> Result<Response, String> {
    let path = picked
        .paths
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&token)
        .ok_or("unknown or already used document token")?;
    let len = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    if len > MAX_DOCUMENT_BYTES {
        return Err(format!("{} is larger than {} bytes", path.display(), MAX_DOCUMENT_BYTES));
    }
    let bytes = tauri::async_runtime::spawn_blocking(move || std::fs::read(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(Response::new(bytes))
}

/// Offers a newer signed release, if the feed has one. Failures (offline, no feed
/// yet) are silent: the next start-up tries again.
async fn check_for_update(app: AppHandle) {
    let Ok(updater) = app.updater() else { return };
    let Ok(Some(update)) = updater.check().await else { return };
    let ask = app
        .dialog()
        .message(format!(
            "Flowster {} is available (you have {}). Install it now? Flowster restarts when it's done.",
            update.version, update.current_version
        ))
        .title("Update available")
        .buttons(MessageDialogButtons::OkCancelCustom("Install and restart".into(), "Later".into()));
    let Ok(install) = tauri::async_runtime::spawn_blocking(move || ask.blocking_show()).await else { return };
    if install && update.download_and_install(|_, _| {}, || {}).await.is_ok() {
        app.restart();
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            app.manage(PickedFiles::default());
            if !cfg!(debug_assertions) {
                tauri::async_runtime::spawn(check_for_update(app.handle().clone()));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![pick_document, read_picked_document, save_document])
        .run(tauri::generate_context!())
        .expect("error while running Flowster");
}
