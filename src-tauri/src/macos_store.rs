use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{
    NSData, NSString, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions, NSURL,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static ACTIVE_CODEX_SCOPE: OnceLock<Mutex<Option<ActiveSecurityScope>>> = OnceLock::new();

struct ActiveSecurityScope {
    url: Retained<NSURL>,
    path: PathBuf,
}

impl Drop for ActiveSecurityScope {
    fn drop(&mut self) {
        unsafe {
            self.url.stopAccessingSecurityScopedResource();
        }
    }
}

pub fn active_codex_home() -> Option<PathBuf> {
    active_scope()
        .lock()
        .ok()
        .and_then(|scope| scope.as_ref().map(|scope| scope.path.clone()))
}

pub fn activate_saved_codex_home(bookmark_path: &Path) -> Result<PathBuf, String> {
    if let Some(path) = active_codex_home() {
        return Ok(path);
    }

    let bookmark_file = bookmark_path.to_path_buf();
    let data = NSData::dataWithContentsOfFile(&ns_string(&bookmark_file))
        .ok_or_else(|| "Codex folder access has not been granted yet.".to_string())?;
    let mut is_stale = Bool::NO;
    let url = unsafe {
        NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
            &data,
            NSURLBookmarkResolutionOptions::WithSecurityScope,
            None,
            &mut is_stale,
        )
    }
    .map_err(|error| error.localizedDescription().to_string())?;

    let path = path_from_url(&url)?;
    validate_codex_home(&path)?;
    activate_scope(url, path.clone())?;

    if is_stale.as_bool() {
        save_bookmark_for_active_scope(&bookmark_file)?;
    }

    Ok(path)
}

pub fn choose_codex_home(
    bookmark_path: &Path,
    suggested_directory: &Path,
) -> Result<PathBuf, String> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "The Codex folder picker must run on the main thread.".to_string())?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(false);
    panel.setResolvesAliases(true);
    panel.setShowsHiddenFiles(true);
    panel.setTitle(Some(&NSString::from_str("Connect Token Meter to Codex")));
    panel.setMessage(Some(&NSString::from_str(
        "Select the .codex folder that contains auth.json and sessions.",
    )));
    panel.setPrompt(Some(&NSString::from_str("Connect")));

    if suggested_directory.exists() {
        let suggested_url = NSURL::fileURLWithPath_isDirectory(
            &NSString::from_str(&suggested_directory.to_string_lossy()),
            true,
        );
        panel.setDirectoryURL(Some(&suggested_url));
    }

    if panel.runModal() != NSModalResponseOK {
        return Err("Codex folder selection was cancelled.".to_string());
    }

    let url = panel
        .URLs()
        .firstObject()
        .ok_or_else(|| "No Codex folder was selected.".to_string())?;
    let path = path_from_url(&url)?;
    validate_codex_home(&path)?;
    save_bookmark(&url, bookmark_path)?;
    activate_scope(url, path.clone())?;
    Ok(path)
}

fn active_scope() -> &'static Mutex<Option<ActiveSecurityScope>> {
    ACTIVE_CODEX_SCOPE.get_or_init(|| Mutex::new(None))
}

fn activate_scope(url: Retained<NSURL>, path: PathBuf) -> Result<(), String> {
    if !unsafe { url.startAccessingSecurityScopedResource() } {
        return Err("macOS did not grant access to the selected Codex folder.".to_string());
    }

    let mut active = active_scope()
        .lock()
        .map_err(|_| "Codex folder access state is unavailable.".to_string())?;
    *active = Some(ActiveSecurityScope { url, path });
    Ok(())
}

fn save_bookmark_for_active_scope(bookmark_path: &Path) -> Result<(), String> {
    let active = active_scope()
        .lock()
        .map_err(|_| "Codex folder access state is unavailable.".to_string())?;
    let scope = active
        .as_ref()
        .ok_or_else(|| "No active Codex folder access was found.".to_string())?;
    save_bookmark(&scope.url, bookmark_path)
}

fn save_bookmark(url: &NSURL, bookmark_path: &Path) -> Result<(), String> {
    if let Some(parent) = bookmark_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let options = NSURLBookmarkCreationOptions::WithSecurityScope
        | NSURLBookmarkCreationOptions::SecurityScopeAllowOnlyReadAccess;
    let data = url
        .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            options, None, None,
        )
        .map_err(|error| error.localizedDescription().to_string())?;
    if data.writeToFile_atomically(&ns_string(bookmark_path), true) {
        Ok(())
    } else {
        Err("Could not save Codex folder access for the next launch.".to_string())
    }
}

fn validate_codex_home(path: &Path) -> Result<(), String> {
    if !path.join("auth.json").is_file() {
        return Err("Select the .codex folder that contains auth.json.".to_string());
    }
    Ok(())
}

fn path_from_url(url: &NSURL) -> Result<PathBuf, String> {
    url.path()
        .map(|path| PathBuf::from(path.to_string()))
        .ok_or_else(|| "The selected Codex folder does not have a local file path.".to_string())
}

fn ns_string(path: &Path) -> Retained<NSString> {
    NSString::from_str(&path.to_string_lossy())
}
