//! Commands the shell UI can invoke. Web pages have no access to these.

use std::path::PathBuf;

use base64::Engine as _;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::browser::extensions::{self, ExtensionInfo};
use crate::browser::hibernation::{HibernationPolicy, Pressure};
use crate::browser::manager::{Layout, TabManager, TabsSnapshot, MAIN_WINDOW};
use crate::browser::omnibox::{self, Destination, SearchEngine};
use crate::governor::limiter::JobPlan;
use crate::governor::overlay::{parse_hotkey, OverlayConfig};
use crate::governor::{ResourceLimits, Telemetry};
use crate::shield::adblocker::{self, ShieldStats, UpdateReport};
use crate::shield::PrivacySettings;
use crate::state::*;
use crate::storage::db::{Bookmark, DownloadRecord, HistoryEntry, ImportReport};
use crate::storage::vault::{self, Login};
use crate::storage::Vault;
use crate::{Error, Result};

pub const SETTING_SEARCH_ENGINE: &str = "search_engine";
pub const OVERLAY_WINDOW: &str = "overlay";
pub const OVERLAY_PAGE: &str = "overlay-page";

fn search_engine(state: &AppState) -> SearchEngine {
    state.db.setting_or_default(SETTING_SEARCH_ENGINE)
}

// --------------------------------------------------------------------- tabs

#[tauri::command]
pub fn tabs_get(app: AppHandle, tabs: State<'_, TabManager>) -> TabsSnapshot {
    tabs.snapshot(&app)
}

#[tauri::command]
pub fn tab_create(
    app: AppHandle,
    state: State<'_, AppState>,
    tabs: State<'_, TabManager>,
    input: Option<String>,
    background: Option<bool>,
) -> Result<String> {
    let url = match input.as_deref().and_then(|text| omnibox::resolve(text, search_engine(&state))) {
        Some(Destination::Url(url) | Destination::Search(url)) => Some(url),
        _ => None,
    };
    let after = tabs.active_id().filter(|_| url.is_some());
    tabs.create(&app, url, background.unwrap_or(false), after.as_deref())
}

/// Navigates a tab to whatever was typed. Returns a command name instead when
/// the input was a `>command`.
#[tauri::command]
pub fn tab_navigate(
    app: AppHandle,
    state: State<'_, AppState>,
    tabs: State<'_, TabManager>,
    id: String,
    input: String,
) -> Result<Option<String>> {
    match omnibox::resolve(&input, search_engine(&state)) {
        Some(Destination::Url(url) | Destination::Search(url)) => {
            tabs.navigate(&app, &id, &url)?;
            Ok(None)
        }
        Some(Destination::Command(command)) => Ok(Some(command)),
        None => Ok(None),
    }
}

#[tauri::command]
pub fn tab_activate(app: AppHandle, tabs: State<'_, TabManager>, id: String) -> Result<()> {
    tabs.activate(&app, &id)
}

#[tauri::command]
pub fn tab_close(app: AppHandle, tabs: State<'_, TabManager>, id: String) -> Result<()> {
    tabs.close(&app, &id)
}

#[tauri::command]
pub fn tab_action(app: AppHandle, tabs: State<'_, TabManager>, id: String, action: String) -> Result<()> {
    tabs.with_webview(&app, &id, |webview| match action.as_str() {
        "back" => webview.go_back(),
        "forward" => webview.go_forward(),
        "reload" => webview.reload(),
        "print" => webview.print(),
        "devtools" => {
            if webview.is_devtools_open() {
                webview.close_devtools();
            } else {
                webview.open_devtools();
            }
            Ok(())
        }
        _ => Ok(()),
    })
}

#[tauri::command]
pub fn tab_zoom(app: AppHandle, tabs: State<'_, TabManager>, id: String, factor: f64) -> Result<()> {
    let factor = factor.clamp(0.25, 5.0);
    tabs.with_webview(&app, &id, |webview| webview.set_zoom(factor))
}

#[tauri::command]
pub fn tab_pin(app: AppHandle, tabs: State<'_, TabManager>, id: String, pinned: bool) {
    tabs.set_pinned(&app, &id, pinned);
}

#[tauri::command]
pub fn tab_group(app: AppHandle, tabs: State<'_, TabManager>, id: String, group: Option<String>) {
    tabs.set_group(&app, &id, group);
}

#[tauri::command]
pub fn tab_move(app: AppHandle, tabs: State<'_, TabManager>, id: String, index: usize) {
    tabs.move_tab(&app, &id, index);
}

#[tauri::command]
pub fn tab_hibernate(app: AppHandle, tabs: State<'_, TabManager>, id: String) -> bool {
    tabs.hibernate(&app, &id)
}

/// The last frame of a hibernated tab, as a `data:` URL.
#[tauri::command]
pub fn tab_preview(state: State<'_, AppState>, id: String) -> Result<Option<String>> {
    Ok(state.db.hibernated_screenshot(&id)?.map(|bytes| {
        format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
    }))
}

#[tauri::command]
pub fn layout_set(app: AppHandle, tabs: State<'_, TabManager>, layout: Layout) {
    tabs.set_layout(&app, layout);
}

#[tauri::command]
pub fn split_set(app: AppHandle, tabs: State<'_, TabManager>, id: Option<String>) -> Result<()> {
    tabs.set_split(&app, id)
}

// ------------------------------------------------------------------ omnibox

#[tauri::command]
pub fn suggest(state: State<'_, AppState>, query: String) -> Result<Vec<HistoryEntry>> {
    state.db.suggest(&query, 6)
}

// ------------------------------------------------- history, bookmarks, files

#[tauri::command]
pub fn history_list(state: State<'_, AppState>, query: Option<String>) -> Result<Vec<HistoryEntry>> {
    state.db.history(query.as_deref().unwrap_or(""), 300)
}

#[tauri::command]
pub fn history_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.db.delete_history_entry(id)
}

#[tauri::command]
pub fn browsing_data_clear(
    app: AppHandle,
    state: State<'_, AppState>,
    history: bool,
    downloads: bool,
    site_data: bool,
) -> Result<()> {
    if history {
        state.db.clear_history()?;
    }
    if downloads {
        state.db.clear_downloads()?;
    }
    if site_data {
        // Cookies, cache and storage are shared by every webview in the profile.
        if let Some(webview) = app.webviews().values().next() {
            webview.clear_all_browsing_data().map_err(|error| Error::Os(error.to_string()))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn bookmarks_list(state: State<'_, AppState>) -> Result<Vec<Bookmark>> {
    state.db.bookmarks()
}

/// Adds or removes a bookmark; returns whether the page is bookmarked now.
#[tauri::command]
pub fn bookmark_toggle(state: State<'_, AppState>, url: String, title: String) -> Result<bool> {
    if state.db.is_bookmarked(&url)? {
        state.db.remove_bookmark_by_url(&url)?;
        Ok(false)
    } else {
        state.db.add_bookmark(&url, &title, "")?;
        Ok(true)
    }
}

#[tauri::command]
pub fn bookmark_remove(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.db.remove_bookmark(id)
}

#[tauri::command]
pub fn is_bookmarked(state: State<'_, AppState>, url: String) -> Result<bool> {
    state.db.is_bookmarked(&url)
}

#[tauri::command]
pub fn downloads_list(state: State<'_, AppState>) -> Result<Vec<DownloadRecord>> {
    state.db.downloads(100)
}

// ------------------------------------------------------------------- shield

#[tauri::command]
pub fn shield_stats(state: State<'_, AppState>) -> ShieldStats {
    state.shield.stats()
}

#[tauri::command]
pub fn shield_site(state: State<'_, AppState>, host: String, enabled: bool) -> Result<()> {
    state.shield.set_site_enabled(&host, enabled);
    state.db.set_setting(SETTING_SHIELD_ALLOWED, &state.shield.allowed_sites())
}

#[tauri::command]
pub fn shield_site_enabled(state: State<'_, AppState>, host: String) -> bool {
    state.shield.is_site_enabled(&host)
}

/// Downloads fresh filter lists and rebuilds the engine.
#[tauri::command]
pub async fn shield_update(app: AppHandle) -> Result<UpdateReport> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let report = adblocker::update_lists(&state.filter_dir())?;
        state.shield.reload_from_cache(&state.filter_dir());
        Ok(report)
    })
    .await
    .map_err(|error| Error::Os(error.to_string()))?
}

#[tauri::command]
pub fn privacy_get(state: State<'_, AppState>) -> PrivacySettings {
    state.privacy()
}

/// Saves privacy settings. Shield and tracking-parameter changes apply at
/// once; cookie, DNS, WebRTC and HTTPS-only changes apply after a restart.
#[tauri::command]
pub fn privacy_set(state: State<'_, AppState>, settings: PrivacySettings) -> Result<()> {
    state.shield.set_enabled(settings.shields_enabled);
    state.db.set_setting(SETTING_PRIVACY, &settings)?;
    *state.privacy.write().unwrap_or_else(|p| p.into_inner()) = settings;
    Ok(())
}

// ----------------------------------------------------------------- governor

#[derive(Serialize)]
pub struct GovernorStatus {
    pub telemetry: Telemetry,
    pub limits: ResourceLimits,
    pub game_active: bool,
    pub game: Option<String>,
    pub live_tabs: usize,
}

#[tauri::command]
pub fn governor_status(state: State<'_, AppState>, tabs: State<'_, TabManager>) -> GovernorStatus {
    GovernorStatus {
        telemetry: state.governor.telemetry(),
        limits: state.limits(),
        game_active: state.is_game_active(),
        game: state.game_name.lock().unwrap_or_else(|p| p.into_inner()).clone(),
        live_tabs: tabs.live_tab_count(),
    }
}

#[tauri::command]
pub fn limits_set(state: State<'_, AppState>, limits: ResourceLimits) -> Result<JobPlan> {
    let plan = state.governor.apply(&limits, state.is_game_active())?;
    state.db.set_setting(SETTING_LIMITS, &limits)?;
    *state.limits.write().unwrap_or_else(|p| p.into_inner()) = limits;
    Ok(plan)
}

/// "Boost": hibernate every background tab and trim working sets.
#[tauri::command]
pub fn memory_free(app: AppHandle, state: State<'_, AppState>, tabs: State<'_, TabManager>) -> usize {
    let frozen = tabs.sweep(&app, Pressure { game_active: state.is_game_active(), memory_critical: true });
    state.governor.trim_memory();
    frozen
}

#[tauri::command]
pub fn hibernation_get(state: State<'_, AppState>) -> HibernationPolicy {
    state.hibernation.read().unwrap_or_else(|p| p.into_inner()).clone()
}

#[tauri::command]
pub fn hibernation_set(state: State<'_, AppState>, policy: HibernationPolicy) -> Result<()> {
    state.db.set_setting(SETTING_HIBERNATION, &policy)?;
    *state.hibernation.write().unwrap_or_else(|p| p.into_inner()) = policy;
    Ok(())
}

// -------------------------------------------------------------------- vault

#[tauri::command]
pub fn vault_unlock(state: State<'_, AppState>, master_password: Option<String>) -> Result<()> {
    let vault = match master_password.filter(|password| !password.is_empty()) {
        Some(password) => Vault::from_master_password(&state.db, &password)?,
        None => Vault::from_os_keyring()?,
    };
    *state.vault.lock().unwrap_or_else(|p| p.into_inner()) = Some(vault);
    Ok(())
}

#[tauri::command]
pub fn vault_lock(state: State<'_, AppState>) {
    *state.vault.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

#[tauri::command]
pub fn vault_is_unlocked(state: State<'_, AppState>) -> bool {
    state.vault.lock().unwrap_or_else(|p| p.into_inner()).is_some()
}

#[tauri::command]
pub fn vault_list(state: State<'_, AppState>) -> Result<Vec<Login>> {
    vault::list_logins(&state.db, None)
}

fn with_vault<T>(state: &AppState, action: impl FnOnce(&Vault) -> Result<T>) -> Result<T> {
    let guard = state.vault.lock().unwrap_or_else(|p| p.into_inner());
    action(guard.as_ref().ok_or_else(|| Error::Vault("the vault is locked".into()))?)
}

#[tauri::command]
pub fn vault_save(state: State<'_, AppState>, origin: String, username: String, password: String) -> Result<()> {
    with_vault(&state, |vault| vault.save(&state.db, &origin, &username, &password))
}

#[tauri::command]
pub fn vault_reveal(state: State<'_, AppState>, id: i64) -> Result<String> {
    with_vault(&state, |vault| Ok(vault.reveal(&state.db, id)?.to_string()))
}

#[tauri::command]
pub fn vault_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    vault::delete_login(&state.db, id)
}

// --------------------------------------------------------------- extensions

#[tauri::command]
pub fn extensions_list(state: State<'_, AppState>) -> Vec<ExtensionInfo> {
    extensions::scan_extensions(&state.extensions_dir())
}

/// Installs an extension from a `.crx`/`.zip` file or an unpacked folder.
/// It is loaded the next time the browser starts.
#[tauri::command]
pub fn extension_install(state: State<'_, AppState>, path: String) -> Result<ExtensionInfo> {
    let source = PathBuf::from(&path);
    let dir = state.extensions_dir();
    std::fs::create_dir_all(&dir)?;
    let name: String = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().chars().filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c)).collect())
        .filter(|name: &String| !name.is_empty())
        .unwrap_or_else(|| "extension".into());
    let dest = dir.join(format!("{name}-{}", crate::now_ms()));
    if source.is_dir() {
        let info = extensions::read_extension(&source)?;
        copy_dir(&source, &dest)?;
        Ok(ExtensionInfo { path: dest, ..info })
    } else {
        let installed = extensions::unpack_crx(&std::fs::read(&source)?, &dest);
        if installed.is_err() {
            let _ = std::fs::remove_dir_all(&dest);
        }
        installed
    }
}

#[tauri::command]
pub fn extension_remove(state: State<'_, AppState>, id: String) -> Result<()> {
    // `id` is the folder name; never let it point outside the extensions dir.
    if id.is_empty() || id.contains(['/', '\\']) || id.contains("..") {
        return Err(Error::Invalid("bad extension id".into()));
    }
    std::fs::remove_dir_all(state.extensions_dir().join(id))?;
    Ok(())
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------ Chrome import

/// Chrome-family profile folders found on this computer.
#[tauri::command]
pub fn chrome_profiles() -> Vec<String> {
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(local) = env("LOCALAPPDATA") {
        for vendor in ["Google/Chrome", "Microsoft/Edge", "BraveSoftware/Brave-Browser", "Chromium"] {
            roots.push(local.join(vendor).join("User Data"));
        }
    }
    if let Some(home) = env("HOME") {
        for dir in [".config/google-chrome", ".config/chromium", ".config/microsoft-edge",
            "Library/Application Support/Google/Chrome", "Library/Application Support/Microsoft Edge"] {
            roots.push(home.join(dir));
        }
    }
    let mut found = Vec::new();
    for root in roots {
        for profile in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            if profile.path().join("History").is_file() || profile.path().join("Bookmarks").is_file() {
                found.push(profile.path().to_string_lossy().into_owned());
            }
        }
    }
    found.sort();
    found
}

#[tauri::command]
pub fn chrome_import(state: State<'_, AppState>, profile: String) -> Result<ImportReport> {
    state.db.import_chrome_profile(profile)
}

// ------------------------------------------------------------------ overlay

#[tauri::command]
pub fn overlay_get(state: State<'_, AppState>) -> OverlayConfig {
    state.overlay.read().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Saves overlay settings. A changed hotkey applies after a restart.
#[tauri::command]
pub fn overlay_set(app: AppHandle, state: State<'_, AppState>, config: OverlayConfig) -> Result<()> {
    parse_hotkey(&config.hotkey)?;
    let config = config.clamped();
    state.db.set_setting(SETTING_OVERLAY, &config)?;
    *state.overlay.write().unwrap_or_else(|p| p.into_inner()) = config;
    crate::overlay_window::apply(&app);
    Ok(())
}

#[tauri::command]
pub fn overlay_toggle(app: AppHandle) {
    crate::overlay_window::toggle(&app);
}

#[tauri::command]
pub fn overlay_navigate(app: AppHandle, state: State<'_, AppState>, input: String) -> Result<()> {
    let Some(Destination::Url(url) | Destination::Search(url)) = omnibox::resolve(&input, search_engine(&state)) else {
        return Ok(());
    };
    state.overlay.write().unwrap_or_else(|p| p.into_inner()).url = url.clone();
    state.db.set_setting(SETTING_OVERLAY, &overlay_get(state.clone()))?;
    crate::overlay_window::navigate(&app, &url);
    Ok(())
}

// ----------------------------------------------------------------- settings

#[tauri::command]
pub fn setting_get(state: State<'_, AppState>, key: String) -> Result<Option<serde_json::Value>> {
    state.db.setting(&format!("ui.{key}"))
}

#[tauri::command]
pub fn setting_set(app: AppHandle, state: State<'_, AppState>, key: String, value: serde_json::Value) -> Result<()> {
    if key == SETTING_SEARCH_ENGINE {
        let engine: SearchEngine = serde_json::from_value(value).map_err(|error| Error::Invalid(error.to_string()))?;
        return state.db.set_setting(SETTING_SEARCH_ENGINE, &engine);
    }
    state.db.set_setting(&format!("ui.{key}"), &value)?;
    let _ = app.emit("setting-changed", (key, value));
    Ok(())
}

#[tauri::command]
pub fn search_engine_get(state: State<'_, AppState>) -> SearchEngine {
    search_engine(&state)
}

// ------------------------------------------------------------------- window

#[tauri::command]
pub fn window_action(app: AppHandle, tabs: State<'_, TabManager>, state: State<'_, AppState>, action: String) -> Result<()> {
    let window = app.get_window(MAIN_WINDOW).ok_or_else(|| Error::Os("main window is gone".into()))?;
    let result = match action.as_str() {
        "minimize" => window.minimize(),
        "maximize" => {
            if window.is_maximized().unwrap_or(false) { window.unmaximize() } else { window.maximize() }
        }
        "drag" => window.start_dragging(),
        "close" => {
            let _ = state.db.save_session(&tabs.session());
            window.close()
        }
        _ => Ok(()),
    };
    result.map_err(|error| Error::Os(error.to_string()))
}
