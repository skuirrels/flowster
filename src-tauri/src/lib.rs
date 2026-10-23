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
//! Saving works the same way round. The token of a `.vsdx` the user opened, or
//! the handle `save_document` returns after a Save As, lets File > Save write
//! that same file again without a dialog; the page never learns the path, and
//! a handle only ever names a file the user chose. Writes go to a temporary
//! file that then replaces the target, so a crash mid-save leaves the old file.
//!
//! Export: SVG and PNG arrive ready-made and are written like a save. PDF is
//! made here (`export_pdf`) from the document's `.vsdx` bytes, because laying
//! out its text needs the system fonts, which the WASM engine cannot reach.
//!
//! Files opened from outside (Finder, Explorer's Open With, a double-click)
//! reach the page the same way as a picked file: the host turns each path into
//! a one-time token and sends it down the channel the page registered with
//! `watch_opened_documents`. Files that arrive before the page is ready wait.
//! Settings can make Flowster the default app for its file types
//! (`file_associations`, `make_default_app`; see `associations`).
//!
//! The page reports whether there are unsaved changes (`set_document_state`),
//! so closing the window can warn before they are lost.
//!
//! Updates (§21, signed Tauri updater) are the host's own business: release builds check the
//! signed update feed at start-up and ask in a native dialog before installing.
//! The webview gets no updater permission.

#![forbid(unsafe_code)]

mod associations;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::Serialize;
use tauri::ipc::{Channel, Response};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::UpdaterExt;

/// Matches the engine's own cap on a package (flowster_vsdx::Limits).
const MAX_DOCUMENT_BYTES: u64 = 2 << 30;

const DOCUMENT_EXTENSIONS: &[&str] = &["vsdx", "vssx", "vstx", "vsdm", "vssm", "vstm", "vsd", "vss", "vst"];

#[derive(Default)]
struct PickedFiles {
    next: AtomicU64,
    paths: Mutex<HashMap<u64, PathBuf>>,
    /// `.vsdx` files the user opened or saved, by handle: File > Save writes back to them.
    documents: Mutex<HashMap<u64, PathBuf>>,
}

impl PickedFiles {
    /// A one-time token for reading `path` (`read_picked_document`).
    fn offer(&self, path: PathBuf) -> Result<PickedDocument, String> {
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.paths.lock().map_err(|e| e.to_string())?.insert(token, path);
        Ok(PickedDocument { token, file_name })
    }
}

/// A file the system asked Flowster to open, and where to send it.
#[derive(Default)]
struct OpenedFiles {
    /// Arrived before the page was listening.
    waiting: Mutex<Option<PathBuf>>,
    page: Mutex<Option<Channel<PickedDocument>>>,
}

/// Hands a file opened from outside to the page, or keeps it until the page
/// listens. Flowster shows one drawing at a time, so of several it takes the first.
fn deliver_opened(app: &AppHandle, paths: Vec<PathBuf>) {
    let Some(path) = paths.into_iter().find(|p| p.is_file()) else { return };
    let opened = app.state::<OpenedFiles>();
    let Ok(page) = opened.page.lock() else { return };
    match page.as_ref() {
        Some(channel) => {
            if let Ok(doc) = app.state::<PickedFiles>().offer(path) {
                let _ = channel.send(doc);
            }
        }
        None => {
            if let Ok(mut waiting) = opened.waiting.lock() {
                *waiting = Some(path);
            }
        }
    }
}

/// The page listens for files opened from outside; one that came first is sent now.
#[tauri::command]
fn watch_opened_documents(
    app: AppHandle,
    on_open: Channel<PickedDocument>,
    opened: State<'_, OpenedFiles>,
) -> Result<(), String> {
    *opened.page.lock().map_err(|e| e.to_string())? = Some(on_open);
    let waiting = opened.waiting.lock().map_err(|e| e.to_string())?.take();
    deliver_opened(&app, waiting.into_iter().collect());
    Ok(())
}

/// Settings: the file types Flowster opens and whether it is their default app.
#[tauri::command]
async fn file_associations() -> Result<associations::Associations, String> {
    tauri::async_runtime::spawn_blocking(associations::current).await.map_err(|e| e.to_string())
}

/// Settings: makes Flowster the default app for these extensions (macOS asks to
/// confirm each), or opens the system's Default apps (Windows).
#[tauri::command]
async fn make_default_app(extensions: Vec<String>) -> Result<associations::Associations, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let made = associations::make_default(&extensions);
        made.map(|()| associations::current())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What the page last said about its document, for the close warning.
#[derive(Default)]
struct DocumentState {
    dirty: AtomicBool,
    title: Mutex<String>,
}

fn is_vsdx(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("vsdx"))
}

/// The last path component of a name the page suggests, for a dialog's default.
fn suggested_name(request: &tauri::ipc::Request<'_>, fallback: &str) -> String {
    request
        .headers()
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .and_then(|n| n.rsplit(['/', '\\']).next())
        .filter(|n| !n.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

/// Writes beside the target, then renames over it: a crash or a full disk
/// mid-write leaves the previous file intact.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.saving"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Shows the save dialog for one file type; `None` if cancelled.
async fn ask_where(
    app: &AppHandle,
    title: &'static str,
    suggested: String,
    filter: (&'static str, &'static str),
) -> Result<Option<PathBuf>, String> {
    let dialog = app.dialog().clone();
    let choice = tauri::async_runtime::spawn_blocking(move || {
        dialog.file().set_title(title).set_file_name(&suggested).add_filter(filter.0, &[filter.1]).blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    choice.map(|c| c.into_path().map_err(|e| e.to_string())).transpose()
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
    picked.offer(choice.into_path().map_err(|e| e.to_string())?).map(Some)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    file_name: String,
    /// For a `.vsdx`: pass it back as `x-handle` to save the same file again.
    handle: Option<u64>,
}

/// Writes what the page sends (a raw binary IPC body) where the user says.
/// The page suggests a file name but never sees or chooses the path.
///
/// Headers: `x-file-name` suggests a name for the dialog; `x-format` is `vsdx`
/// (the default), `svg` or `png`; `x-handle`, for a `.vsdx`, names a file the
/// user opened or saved before, which is then written again without asking.
#[tauri::command]
async fn save_document(
    app: AppHandle,
    files: State<'_, PickedFiles>,
    request: tauri::ipc::Request<'_>,
) -> Result<Option<Saved>, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected the document bytes".into());
    };
    let bytes = bytes.clone();
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let format = header("x-format").unwrap_or_else(|| "vsdx".into());
    let (title, filter) = match format.as_str() {
        "vsdx" => ("Save", ("Visio drawing", "vsdx")),
        "svg" => ("Export", ("SVG image", "svg")),
        "png" => ("Export", ("PNG image", "png")),
        other => return Err(format!("cannot save {other} files")),
    };
    let known = header("x-handle")
        .and_then(|h| h.parse::<u64>().ok())
        .filter(|_| format == "vsdx")
        .and_then(|h| Some((h, files.documents.lock().ok()?.get(&h)?.clone())));
    let (handle, path) = match known {
        Some((h, path)) => (Some(h), path),
        None => {
            let suggested = suggested_name(&request, &format!("Drawing.{}", filter.1));
            let Some(path) = ask_where(&app, title, suggested, filter).await? else { return Ok(None) };
            (None, path)
        }
    };
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let target = path.clone();
    tauri::async_runtime::spawn_blocking(move || write_atomically(&target, &bytes))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("could not write {file_name}: {e}"))?;
    let handle = match handle {
        Some(h) => Some(h),
        None if format == "vsdx" && is_vsdx(&path) => {
            let h = files.next.fetch_add(1, Ordering::Relaxed) + 1;
            files.documents.lock().map_err(|e| e.to_string())?.insert(h, path);
            Some(h)
        }
        None => None,
    };
    Ok(Some(Saved { file_name, handle }))
}

/// Exports the document (its `.vsdx` bytes as the raw body) as a PDF where the
/// user says: one PDF page per page index in `x-pages` (comma-separated).
#[tauri::command]
async fn export_pdf(app: AppHandle, request: tauri::ipc::Request<'_>) -> Result<Option<String>, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected the document bytes".into());
    };
    let bytes = bytes.clone();
    let pages: Vec<usize> = request
        .headers()
        .get("x-pages")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').filter_map(|p| p.trim().parse().ok()).collect())
        .unwrap_or_default();
    let suggested = suggested_name(&request, "Drawing.pdf");
    let Some(path) = ask_where(&app, "Export", suggested, ("PDF document", "pdf")).await? else { return Ok(None) };
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let pdf = pdf_from_vsdx(bytes, &pages)?;
        write_atomically(&path, &pdf).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(Some(file_name))
}

/// A PDF of the given pages of a `.vsdx`, its text laid out with the system fonts.
fn pdf_from_vsdx(bytes: Vec<u8>, pages: &[usize]) -> Result<Vec<u8>, String> {
    let pkg = flowster_vsdx::OpcPackage::open(bytes).map_err(|e| e.to_string())?;
    let doc = flowster_vsdx::visio::read_document(&pkg).map_err(|e| e.to_string())?;
    let lists: Vec<_> = pages.iter().filter_map(|&i| flowster_render::build_page(&doc, i)).collect();
    if lists.is_empty() {
        return Err("there are no pages to export".into());
    }
    let mut text = flowster_text::TextEngine::system();
    let layouts: Vec<_> = lists.iter().map(|dl| text.layout_display_list(dl)).collect();
    let pages: Vec<_> = lists.iter().zip(&layouts).map(|(dl, l)| (dl, l.as_slice())).collect();
    flowster_export::pdf::pdf(&pages)
}

/// The page's document: whether it has unsaved changes, and its title.
#[tauri::command]
fn set_document_state(dirty: bool, title: String, state: State<'_, DocumentState>) -> Result<(), String> {
    state.dirty.store(dirty, Ordering::Relaxed);
    *state.title.lock().map_err(|e| e.to_string())? = title;
    Ok(())
}

#[tauri::command]
async fn read_picked_document(token: u64, picked: State<'_, PickedFiles>) -> Result<Response, String> {
    let path = picked
        .paths
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&token)
        .ok_or("unknown or already used document token")?;
    // A .vsdx can be saved back in place: its token becomes its save handle.
    if is_vsdx(&path) {
        picked.documents.lock().map_err(|e| e.to_string())?.insert(token, path.clone());
    }
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

/// Shows a message and waits for OK (or a choice between two labels; true for the first).
async fn ask(app: &AppHandle, title: &str, text: String, buttons: Option<(&str, &str)>) -> bool {
    let mut dialog = app.dialog().message(text).title(title);
    if let Some((yes, no)) = buttons {
        dialog = dialog.buttons(MessageDialogButtons::OkCancelCustom(yes.into(), no.into()));
    }
    tauri::async_runtime::spawn_blocking(move || dialog.blocking_show()).await.unwrap_or(false)
}

static CHECKING: AtomicBool = AtomicBool::new(false);

/// Offers a newer signed release, if the feed has one. The start-up check is
/// quiet unless there is an update (offline or no feed yet: the next start-up
/// tries again); Help > Check for Updates (`interactive`) also says "up to
/// date" and reports errors.
async fn check_for_update(app: AppHandle, interactive: bool) {
    if CHECKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let result = offer_update(&app, interactive).await;
    CHECKING.store(false, Ordering::SeqCst);
    if let Err(e) = result
        && interactive
    {
        ask(&app, "Check for Updates", format!("Couldn't check for updates: {e}"), None).await;
    }
}

async fn offer_update(app: &AppHandle, interactive: bool) -> Result<(), String> {
    let version = app.package_info().version.to_string();
    let up_to_date = |platform: &str| format!("You're up to date: Flowster {version} is the latest version{platform}.");
    if cfg!(debug_assertions) {
        if interactive {
            ask(
                app,
                "Check for Updates",
                "Development builds don't update themselves. Install Flowster to get updates.".into(),
                None,
            )
            .await;
        }
        return Ok(());
    }
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = match updater.check().await {
        Ok(Some(update)) => update,
        Ok(None) => {
            if interactive {
                ask(app, "Check for Updates", up_to_date(""), None).await;
            }
            return Ok(());
        }
        // Each platform is built and published on its own: a newer version that
        // isn't out for this one yet means this computer is up to date.
        Err(e) if e.to_string().contains("platform") && e.to_string().contains("not found") => {
            if interactive {
                ask(app, "Check for Updates", up_to_date(" for this computer"), None).await;
            }
            return Ok(());
        }
        Err(e) => return Err(e.to_string()),
    };
    let unsaved = if app.state::<DocumentState>().dirty.load(Ordering::Relaxed) {
        "\n\nYour drawing has unsaved changes: save it first, as Flowster closes to install."
    } else {
        ""
    };
    let text = format!(
        "Flowster {} is available (you have {}). Install it now? Flowster restarts when it's done.{unsaved}",
        update.version, update.current_version
    );
    if !ask(app, "Update available", text, Some(("Install and restart", "Later"))).await {
        return Ok(());
    }
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    app.restart();
}

/// Help > Check for Updates.
#[tauri::command]
fn check_for_updates(app: AppHandle) {
    tauri::async_runtime::spawn(check_for_update(app, true));
}

/// Documents named on the command line: how Windows and Linux open a file with Flowster.
fn documents_in_args() -> Vec<PathBuf> {
    std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .filter(|p| p.extension().is_some_and(|e| DOCUMENT_EXTENSIONS.iter().any(|d| e.eq_ignore_ascii_case(d))))
        .collect()
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Managed before start-up: macOS can hand over a file before `setup` runs.
        .manage(PickedFiles::default())
        .manage(OpenedFiles::default())
        .setup(|app| {
            app.manage(DocumentState::default());
            if !cfg!(target_os = "macos") {
                deliver_opened(app.handle(), documents_in_args());
            }
            if !cfg!(debug_assertions) {
                tauri::async_runtime::spawn(check_for_update(app.handle().clone(), false));
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<DocumentState>();
                if !state.dirty.load(Ordering::Relaxed) {
                    return;
                }
                api.prevent_close();
                let title = state.title.lock().map(|t| t.clone()).unwrap_or_default();
                let window = window.clone();
                window
                    .dialog()
                    .message(format!(
                        "“{title}” has changes that aren't saved. Close Flowster anyway?\n\n\
                         Flowster keeps a recovery copy and offers it the next time it starts."
                    ))
                    .title("Unsaved changes")
                    .kind(MessageDialogKind::Warning)
                    .buttons(MessageDialogButtons::OkCancelCustom("Close without saving".into(), "Cancel".into()))
                    .show(move |close| {
                        if close {
                            window.state::<DocumentState>().dirty.store(false, Ordering::Relaxed);
                            let _ = window.destroy();
                        }
                    });
            }
        })
        .invoke_handler(tauri::generate_handler![
            pick_document,
            read_picked_document,
            save_document,
            export_pdf,
            set_document_state,
            check_for_updates,
            watch_opened_documents,
            file_associations,
            make_default_app
        ])
        .build(tauri::generate_context!())
        .expect("error while starting Flowster");
    app.run(|_app, _event| {
        // macOS opens files with an Apple event rather than arguments, at start-up or later.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Opened { urls } = _event {
            deliver_opened(_app, urls.into_iter().filter_map(|u| u.to_file_path().ok()).collect());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drawing_exports_as_a_pdf() {
        let pdf = pdf_from_vsdx(flowster_vsdx::fixture::minimal_vsdx(), &[0]).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf_from_vsdx(flowster_vsdx::fixture::minimal_vsdx(), &[9]).is_err(), "no such page");
        assert!(pdf_from_vsdx(b"not a package".to_vec(), &[0]).is_err());
    }

    #[test]
    fn saving_replaces_the_file_whole() {
        let dir = std::env::temp_dir().join(format!("flowster-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Plan.vsdx");
        std::fs::write(&path, b"old contents, longer than the new").unwrap();
        write_atomically(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary file left behind");
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(is_vsdx(Path::new("a/B.VSDX")) && !is_vsdx(Path::new("b.vsdm")));
    }
}
