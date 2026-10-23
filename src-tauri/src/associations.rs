//! Settings > Open with Flowster: which Visio file types open in Flowster.
//!
//! The bundle declares the types (`fileAssociations` in tauri.conf.json, the
//! UTIs in Info.plist), so the system offers Flowster for them; making it the
//! default is up to the user. On macOS Flowster asks LaunchServices for each
//! type the user ticks, and macOS confirms each one. Windows does not let an
//! app make itself the default, so Settings opens the system's Default apps
//! page instead. Elsewhere the file manager's Open With does it.

use serde::Serialize;

/// The types Flowster opens, as listed in Settings.
pub const EXTENSIONS: &[&str] = &["vsdx", "vsdm", "vssx", "vssm", "vstx", "vstm"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Associations {
    /// `macos`, `windows` or `linux`: how the default app is chosen there.
    platform: &'static str,
    /// Why Flowster can't make itself the default here, if it can't.
    unavailable: Option<String>,
    kinds: Vec<Kind>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Kind {
    ext: &'static str,
    /// Whether files of this type open in Flowster now; `None` where that isn't known.
    is_default: Option<bool>,
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::mpsc;
    use std::time::Duration;

    use objc2::rc::Retained;
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSBundle, NSError, NSURL};

    /// An empty file with this extension, to ask LaunchServices about. Asking
    /// about a file rather than its content type gets the app Finder actually
    /// uses: an app that claims the extension (draw.io claims .vsdx) wins over
    /// the type's own handler unless the user has chosen one.
    fn probe(ext: &str) -> Result<Retained<NSURL>, String> {
        let dir = std::env::temp_dir().join("flowster-open-with");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("probe.{ext}"));
        if !path.exists() {
            std::fs::write(&path, b"").map_err(|e| e.to_string())?;
        }
        NSURL::from_file_path(&path).ok_or_else(|| format!("no file URL for {}", path.display()))
    }

    /// The running Flowster.app, or why there isn't one to register.
    pub fn app() -> Result<Retained<NSURL>, String> {
        if !objc2::available!(macos = 12.0) {
            return Err("Choosing the default app from Flowster needs macOS 12 or later. \
                        Use Get Info › Open with › Change All in Finder instead."
                .into());
        }
        let url = NSBundle::mainBundle().bundleURL();
        let is_app = url.path().is_some_and(|p| p.to_string().ends_with(".app"));
        if !is_app {
            return Err("Development builds can't be the default app. Install Flowster to choose it.".into());
        }
        Ok(url)
    }

    fn bundle_id(url: &NSURL) -> Option<String> {
        NSBundle::bundleWithURL(url)?.bundleIdentifier().map(|id| id.to_string())
    }

    /// Whether `ext` files open in Flowster (any copy of it: the bundle id is compared).
    pub fn is_default(ext: &str) -> Option<bool> {
        let ours = bundle_id(&NSBundle::mainBundle().bundleURL())?;
        let handler = NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&*probe(ext).ok()?);
        Some(handler.and_then(|u| bundle_id(&u)).is_some_and(|id| id == ours))
    }

    /// Asks LaunchServices to open `ext` files with Flowster; macOS asks the
    /// user to confirm, so this waits for their answer.
    pub fn make_default(app: &NSURL, ext: &str) -> Result<(), String> {
        let file = probe(ext)?;
        let (tx, rx) = mpsc::channel();
        let done = block2::RcBlock::new(move |error: *mut NSError| {
            let _ = tx.send(error.is_null());
        });
        NSWorkspace::sharedWorkspace().setDefaultApplicationAtURL_toOpenContentTypeOfFileAtURL_completionHandler(
            app,
            &file,
            Some(&done),
        );
        match rx.recv_timeout(Duration::from_secs(300)) {
            Ok(true) => Ok(()),
            Ok(false) => Err(format!(".{ext} files still open in another app")),
            Err(_) => Err(format!("macOS didn't answer about .{ext} files")),
        }
    }
}

/// The current state, for Settings.
pub fn current() -> Associations {
    #[cfg(target_os = "macos")]
    {
        Associations {
            platform: "macos",
            unavailable: mac::app().err(),
            kinds: EXTENSIONS.iter().map(|&ext| Kind { ext, is_default: mac::is_default(ext) }).collect(),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Associations {
            platform: if cfg!(windows) { "windows" } else { "linux" },
            unavailable: None,
            kinds: EXTENSIONS.iter().map(|&ext| Kind { ext, is_default: None }).collect(),
        }
    }
}

/// Makes Flowster the default app for `extensions` (macOS), or opens the
/// system settings where the user chooses it (Windows).
pub fn make_default(extensions: &[String]) -> Result<(), String> {
    if let Some(other) = extensions.iter().find(|e| !EXTENSIONS.contains(&e.as_str())) {
        return Err(format!("Flowster doesn't open .{other} files"));
    }
    #[cfg(target_os = "macos")]
    {
        let app = mac::app()?;
        let refused: Vec<String> = extensions.iter().filter_map(|ext| mac::make_default(&app, ext).err()).collect();
        if refused.is_empty() { Ok(()) } else { Err(refused.join("; ")) }
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg("ms-settings:defaultapps")
            .spawn()
            .map(drop)
            .map_err(|e| format!("couldn't open Default apps: {e}"))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err("Choose Flowster with your file manager's Open With.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_listed_types_can_be_claimed() {
        assert!(make_default(&["pdf".into()]).unwrap_err().contains(".pdf"));
        assert_eq!(current().kinds.len(), EXTENSIONS.len());
    }
}
