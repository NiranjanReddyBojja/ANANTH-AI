//! Aether entry point: Chromium (CEF) configuration, window creation and the
//! background loops that drive the governor, game mode and hibernation.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl};
use tauri_runtime_cef::{Cef, WebRtcIpHandling};

use aether::browser::extensions;
use aether::browser::hibernation::Pressure;
use aether::browser::manager::{TabManager, MAIN_WINDOW, SHELL_WEBVIEW};
use aether::governor::game_detect::{sample_foreground, GameDetector, GameEvent};
use aether::governor::overlay::{parse_hotkey, GlobalHotkey};
use aether::governor::Governor;
use aether::shield::adblocker;
use aether::shield::privacy::WebRtcPolicy;
use aether::state::{data_dir, AppState};
use aether::{commands, overlay_window};

/// Keeps the global hotkey registered for the life of the app.
struct HotkeyHolder(#[allow(dead_code)] Mutex<Option<GlobalHotkey>>);

/// Chromium configured for a small footprint and the user's privacy settings.
fn chromium(state: &AppState) -> Cef {
    let privacy = state.privacy();
    let mut cef = Cef::default()
        .root_cache_path(state.data_dir.join("chromium"))
        // V8: favour memory over peak throughput.
        .javascript_flags("--optimize-for-size")
        .command_line_args([
            // One renderer per site instead of per tab, and a cap on renderers.
            ("process-per-site", None::<&str>),
            ("renderer-process-limit", Some("8")),
            // No background work once the last window is closed.
            ("disable-background-mode", None),
        ])
        .disable_features([
            // Background services a gaming browser does not need.
            "Translate",
            "MediaRouter",
            "OptimizationHints",
            "InterestFeedContentSuggestions",
            // Allow unpacked extensions from the extensions folder.
            "DisableLoadExtensionCommandLineSwitch",
        ])
        .webrtc_ip_handling(match privacy.webrtc {
            WebRtcPolicy::Default => WebRtcIpHandling::Default,
            WebRtcPolicy::PublicInterfaceOnly => WebRtcIpHandling::DefaultPublicInterfaceOnly,
            WebRtcPolicy::DisableNonProxiedUdp => WebRtcIpHandling::DisableNonProxiedUdp,
        })
        .safe_browsing(privacy.safe_browsing)
        .persist_session_cookies(true);

    for (name, value) in privacy.profile_preferences() {
        cef = cef.profile_preference_value(name, value);
    }
    for (name, value) in privacy.global_preferences() {
        cef = cef.global_preference(name, value);
    }
    // Development aids, compiled out of release builds.
    #[cfg(debug_assertions)]
    {
        if let Some(port) = std::env::var("AETHER_DEBUG_PORT").ok().and_then(|port| port.parse().ok()) {
            cef = cef.remote_debugging(tauri_runtime_cef::RemoteDebugging::Port { port, allowed_origins: Vec::new() });
        }
        if std::env::var_os("AETHER_NO_SANDBOX").is_some() {
            cef = cef.sandbox(tauri_runtime_cef::SandboxPolicy::Disabled);
        }
    }
    if let Some(paths) = extensions::load_extension_switch(&extensions::scan_extensions(&state.extensions_dir())) {
        cef = cef.command_line_arg("load-extension", Some(paths));
    }
    cef
}

fn create_windows(app: &AppHandle) -> tauri::Result<()> {
    let window = tauri::window::WindowBuilder::new(app, MAIN_WINDOW)
        .title("Aether")
        .inner_size(1360.0, 860.0)
        .min_inner_size(720.0, 480.0)
        .decorations(false)
        .build()?;
    let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
    window.add_child(
        WebviewBuilder::new(SHELL_WEBVIEW, WebviewUrl::App("index.html".into()))
            .position(LogicalPosition::new(0.0, 0.0))
            .size(LogicalSize::new(size.width, size.height))
            .auto_resize(),
    )?;

    // Closing the main window saves the session and quits (the overlay
    // window would otherwise keep the app alive).
    let handle = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
            let _ = handle.state::<AppState>().db.save_session(&handle.state::<TabManager>().session());
            handle.exit(0);
        }
    });

    if let Err(error) = overlay_window::create(app) {
        log::warn!("overlay unavailable: {error}");
    }
    Ok(())
}

fn restore_session(app: &AppHandle) {
    let state = app.state::<AppState>();
    let tabs = app.state::<TabManager>();
    let session = state.db.load_session().unwrap_or_default();
    let _ = state.db.clear_hibernated();
    let mut active = None;
    for saved in &session {
        let id = tabs.restore(saved);
        if saved.active {
            active = Some(id);
        }
    }
    let result = match active.or_else(|| tabs.snapshot(app).tabs.first().map(|tab| tab.id.clone())) {
        Some(id) => tabs.activate(app, &id),
        None => tabs.create(app, None, false, None).map(|_| ()),
    };
    if let Err(error) = result {
        log::warn!("could not restore session: {error}");
    }
}

/// Loads cached filter lists, then refreshes stale ones in the background.
fn start_filter_updates(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("aether-filters".into())
        .spawn(move || {
            let state = app.state::<AppState>();
            let dir = state.filter_dir();
            state.shield.reload_from_cache(&dir);
            if adblocker::stale_lists(&dir).is_empty() {
                return;
            }
            match adblocker::update_lists(&dir) {
                Ok(report) if !report.updated.is_empty() => {
                    state.shield.reload_from_cache(&dir);
                    let _ = app.emit("shield-updated", report);
                }
                Ok(_) => {}
                Err(error) => log::warn!("filter update failed: {error}"),
            }
        })
        .ok();
}

/// Every two seconds: telemetry for the HUD, game detection, and memory
/// pressure handling. Every ten: the regular hibernation sweep and a session save.
fn start_governor_loop(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut detector = GameDetector::default();
        let mut tick = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            tick += 1;
            let state = app.state::<AppState>();
            let tabs = app.state::<TabManager>();

            if let Some(event) = detector.update(sample_foreground().as_ref()) {
                let active = matches!(event, GameEvent::Started { .. });
                state.game_active.store(active, Ordering::Relaxed);
                *state.game_name.lock().unwrap_or_else(|p| p.into_inner()) =
                    detector.active_game().map(str::to_owned);
                if let Err(error) = state.governor.apply(&state.limits(), active) {
                    log::warn!("could not apply limits: {error}");
                }
                if active && state.limits().game_mode {
                    // Give the game its memory straight away.
                    tabs.sweep(&app, Pressure { game_active: true, memory_critical: false });
                    state.governor.trim_memory();
                }
                let _ = app.emit("game-mode", event);
            }

            let telemetry = state.governor.telemetry();
            let critical = telemetry.memory_critical();
            let _ = app.emit("telemetry", &telemetry);

            if critical || tick % 5 == 0 {
                tabs.sweep(&app, Pressure { game_active: state.is_game_active(), memory_critical: critical });
            }
            if tick % 5 == 0 {
                let _ = state.db.save_session(&tabs.session());
            }
        }
    });
}

fn register_overlay_hotkey(app: &AppHandle) -> Option<GlobalHotkey> {
    let config = app.state::<AppState>().overlay.read().unwrap_or_else(|p| p.into_inner()).clone();
    if !config.enabled {
        return None;
    }
    let hotkey = parse_hotkey(&config.hotkey).ok()?;
    let handle = app.clone();
    match GlobalHotkey::register(hotkey, move || {
        let app = handle.clone();
        let _ = handle.run_on_main_thread(move || overlay_window::toggle(&app));
    }) {
        Ok(registered) => Some(registered),
        Err(error) => {
            log::info!("overlay hotkey not registered: {error}");
            None
        }
    }
}

#[tauri_runtime_cef::cef_entry_point]
fn main() {
    // Created first so that, on Windows, every Chromium child process is born
    // inside the browser's Job Object.
    let governor = Arc::new(Governor::new());
    let state = AppState::load(data_dir(), governor).expect("cannot open the Aether profile");
    if let Err(error) = state.governor.apply(&state.limits(), false) {
        eprintln!("aether: resource limits not applied: {error}");
    }
    let cef = chromium(&state);

    tauri::Builder::default()
        .runtime(cef)
        .manage(state)
        .manage(TabManager::default())
        .invoke_handler(tauri::generate_handler![
            commands::tabs_get,
            commands::tab_create,
            commands::tab_navigate,
            commands::tab_activate,
            commands::tab_close,
            commands::tab_action,
            commands::tab_zoom,
            commands::tab_pin,
            commands::tab_group,
            commands::tab_move,
            commands::tab_hibernate,
            commands::tab_preview,
            commands::layout_set,
            commands::split_set,
            commands::suggest,
            commands::history_list,
            commands::history_delete,
            commands::browsing_data_clear,
            commands::bookmarks_list,
            commands::bookmark_toggle,
            commands::bookmark_remove,
            commands::is_bookmarked,
            commands::downloads_list,
            commands::shield_stats,
            commands::shield_site,
            commands::shield_site_enabled,
            commands::shield_update,
            commands::privacy_get,
            commands::privacy_set,
            commands::governor_status,
            commands::limits_set,
            commands::memory_free,
            commands::hibernation_get,
            commands::hibernation_set,
            commands::vault_unlock,
            commands::vault_lock,
            commands::vault_is_unlocked,
            commands::vault_list,
            commands::vault_save,
            commands::vault_reveal,
            commands::vault_delete,
            commands::extensions_list,
            commands::extension_install,
            commands::extension_remove,
            commands::chrome_profiles,
            commands::chrome_import,
            commands::overlay_get,
            commands::overlay_set,
            commands::overlay_toggle,
            commands::overlay_navigate,
            commands::setting_get,
            commands::setting_set,
            commands::search_engine_get,
            commands::window_action,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            create_windows(&handle)?;
            restore_session(&handle);
            start_filter_updates(&handle);
            start_governor_loop(&handle);
            app.manage(HotkeyHolder(Mutex::new(register_overlay_hotkey(&handle))));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Aether");
}
