//! Tab manager: owns every tab's Chromium webview inside the main window.
//!
//! The React shell is one lightweight webview covering the window. Each tab is
//! a separate child webview positioned over the shell's content area; only the
//! tabs shown in a pane are visible. Background tabs can be hibernated, which
//! destroys their webview (and with it the renderer process) until they are
//! selected again.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl};
use tauri_runtime_cef::{allocate_devtools_message_id, DevToolsProtocol, WebviewBuilderCefExt, WebviewCefExt};

use crate::browser::hibernation::{self, Pressure, TabActivity};
use crate::shield::adblocker::{cosmetics_injection_script, resource_type_from_cdp};
use crate::shield::privacy::strip_tracking_params;
use crate::shield::Decision;
use crate::state::AppState;
use crate::storage::db::{DownloadRecord, HibernatedTab, SessionTab};
use crate::{now_ms, Error, Result};

pub const MAIN_WINDOW: &str = "main";
pub const SHELL_WEBVIEW: &str = "shell";
pub const EVENT_TABS: &str = "tabs-changed";
pub const EVENT_DOWNLOADS: &str = "downloads-changed";
pub const EVENT_SHORTCUT: &str = "shortcut";

#[derive(Debug, Clone, Serialize)]
pub struct TabInfo {
    pub id: String,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub pinned: bool,
    pub group: Option<String>,
    pub hibernated: bool,
    pub active: bool,
    pub in_split: bool,
    pub blocked: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TabsSnapshot {
    pub tabs: Vec<TabInfo>,
    pub active: Option<String>,
    pub split: Option<String>,
}

/// The shell's content area in CSS pixels, relative to the window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct Layout {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

struct Tab {
    id: String,
    url: String,
    title: String,
    loading: bool,
    pinned: bool,
    group: Option<String>,
    hibernated: bool,
    last_active: Instant,
    /// Scroll position to restore once a woken tab has loaded.
    restore_scroll: f64,
}

#[derive(Default)]
struct Inner {
    tabs: Vec<Tab>,
    active: Option<String>,
    split: Option<String>,
    layout: Layout,
    next_id: u64,
    /// Tabs whose hibernation is in progress.
    hibernating: HashSet<String>,
    /// Screenshot requests in flight: DevTools message id -> tab id.
    screenshots: HashMap<i32, String>,
}

pub struct TabManager {
    inner: Mutex<Inner>,
    /// DevTools messages are sent from a worker thread, never from inside a
    /// DevTools callback.
    devtools: mpsc::Sender<(Webview, String)>,
}

fn label(id: &str) -> String {
    format!("tab-{id}")
}

fn id_from_label(label: &str) -> Option<&str> {
    label.strip_prefix("tab-")
}

impl Default for TabManager {
    fn default() -> Self {
        let (devtools, queue) = mpsc::channel::<(Webview, String)>();
        std::thread::Builder::new()
            .name("aether-devtools".into())
            .spawn(move || {
                for (webview, message) in queue {
                    if let Err(error) = webview.send_dev_tools_message(message.as_bytes()) {
                        log::debug!("devtools message dropped: {error}");
                    }
                }
            })
            .expect("spawn devtools thread");
        Self { inner: Mutex::new(Inner::default()), devtools }
    }
}

impl TabManager {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn send_devtools(&self, webview: &Webview, method: &str, params: serde_json::Value) -> Option<i32> {
        let id = allocate_devtools_message_id().ok()?;
        let message = serde_json::json!({ "id": id, "method": method, "params": params }).to_string();
        self.devtools.send((webview.clone(), message)).ok()?;
        Some(id)
    }

    // ------------------------------------------------------------ queries

    pub fn snapshot(&self, app: &AppHandle) -> TabsSnapshot {
        let blocked = app.state::<AppState>().shield.stats().blocked_by_tab;
        let inner = self.lock();
        TabsSnapshot {
            tabs: inner
                .tabs
                .iter()
                .map(|tab| TabInfo {
                    id: tab.id.clone(),
                    url: tab.url.clone(),
                    title: tab.title.clone(),
                    loading: tab.loading,
                    pinned: tab.pinned,
                    group: tab.group.clone(),
                    hibernated: tab.hibernated,
                    active: inner.active.as_deref() == Some(&tab.id),
                    in_split: inner.split.as_deref() == Some(&tab.id),
                    blocked: blocked.get(&tab.id).copied().unwrap_or(0),
                })
                .collect(),
            active: inner.active.clone(),
            split: inner.split.clone(),
        }
    }

    pub fn emit(&self, app: &AppHandle) {
        let _ = app.emit(EVENT_TABS, self.snapshot(app));
    }

    fn page_url(&self, id: &str) -> String {
        self.lock().tabs.iter().find(|tab| tab.id == id).map(|tab| tab.url.clone()).unwrap_or_default()
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut Tab)) -> bool {
        let mut inner = self.lock();
        match inner.tabs.iter_mut().find(|tab| tab.id == id) {
            Some(tab) => {
                change(tab);
                true
            }
            None => false,
        }
    }

    fn webview(&self, app: &AppHandle, id: &str) -> Option<Webview> {
        app.get_webview(&label(id))
    }

    // ---------------------------------------------------------- lifecycle

    /// Opens a tab. Without a URL it is a blank "new tab" drawn by the shell.
    pub fn create(&self, app: &AppHandle, url: Option<String>, background: bool, after: Option<&str>) -> Result<String> {
        let id = {
            let mut inner = self.lock();
            inner.next_id += 1;
            let id = format!("t{}", inner.next_id);
            let tab = Tab {
                id: id.clone(),
                url: url.clone().unwrap_or_default(),
                title: String::new(),
                loading: url.is_some(),
                pinned: false,
                group: None,
                hibernated: false,
                last_active: Instant::now(),
                restore_scroll: 0.0,
            };
            let position = after
                .and_then(|after| inner.tabs.iter().position(|tab| tab.id == after))
                .map_or(inner.tabs.len(), |index| index + 1);
            inner.tabs.insert(position, tab);
            id
        };
        if url.is_some() {
            self.ensure_webview(app, &id)?;
        }
        if background {
            self.apply_layout(app);
            self.emit(app);
        } else {
            self.activate(app, &id)?;
        }
        Ok(id)
    }

    /// Restores a tab from the previous session without loading it.
    pub fn restore(&self, session_tab: &SessionTab) -> String {
        let mut inner = self.lock();
        inner.next_id += 1;
        let id = format!("t{}", inner.next_id);
        inner.tabs.push(Tab {
            id: id.clone(),
            url: session_tab.url.clone(),
            title: session_tab.title.clone(),
            loading: false,
            pinned: session_tab.pinned,
            group: session_tab.group.clone(),
            // Loaded on first use, exactly like a hibernated tab.
            hibernated: !session_tab.url.is_empty(),
            last_active: Instant::now(),
            restore_scroll: 0.0,
        });
        id
    }

    pub fn activate(&self, app: &AppHandle, id: &str) -> Result<()> {
        let needs_wake = {
            let mut inner = self.lock();
            let Some(tab) = inner.tabs.iter_mut().find(|tab| tab.id == id) else {
                return Err(Error::Invalid(format!("no tab {id}")));
            };
            tab.last_active = Instant::now();
            let wake = tab.hibernated;
            if inner.split.as_deref() == Some(id) {
                // Selecting the split tab swaps the panes.
                inner.split = inner.active.clone();
            }
            inner.active = Some(id.to_owned());
            wake
        };
        if needs_wake {
            self.wake(app, id)?;
        }
        self.apply_layout(app);
        if let Some(webview) = self.webview(app, id) {
            let _ = webview.set_focus();
        }
        self.emit(app);
        Ok(())
    }

    pub fn close(&self, app: &AppHandle, id: &str) -> Result<()> {
        let next = {
            let mut inner = self.lock();
            let Some(index) = inner.tabs.iter().position(|tab| tab.id == id) else {
                return Ok(());
            };
            inner.tabs.remove(index);
            inner.hibernating.remove(id);
            if inner.split.as_deref() == Some(id) {
                inner.split = None;
            }
            if inner.active.as_deref() == Some(id) {
                inner.active = None;
                inner.tabs.get(index).or_else(|| index.checked_sub(1).and_then(|i| inner.tabs.get(i))).map(|tab| tab.id.clone())
            } else {
                None
            }
        };
        if let Some(webview) = self.webview(app, id) {
            let _ = webview.close();
        }
        let state = app.state::<AppState>();
        state.shield.reset_tab(id);
        let _ = state.db.take_hibernated(id);
        match next {
            Some(next) => self.activate(app, &next)?,
            None => {
                self.apply_layout(app);
                self.emit(app);
            }
        }
        Ok(())
    }

    pub fn navigate(&self, app: &AppHandle, id: &str, url: &str) -> Result<()> {
        let parsed = Url::parse(url).map_err(|error| Error::Invalid(format!("bad URL: {error}")))?;
        if !matches!(parsed.scheme(), "http" | "https" | "about" | "data" | "blob" | "file" | "view-source") {
            return Err(Error::Invalid(format!("unsupported URL scheme '{}'", parsed.scheme())));
        }
        let existed = self.update(id, |tab| {
            tab.url = parsed.to_string();
            tab.title.clear();
            tab.loading = true;
            tab.hibernated = false;
            tab.last_active = Instant::now();
        });
        if !existed {
            return Err(Error::Invalid(format!("no tab {id}")));
        }
        let _ = app.state::<AppState>().db.take_hibernated(id);
        match self.webview(app, id) {
            Some(webview) => webview.navigate(parsed).map_err(|error| Error::Os(error.to_string()))?,
            None => self.ensure_webview(app, id)?,
        }
        self.apply_layout(app);
        if let Some(webview) = self.webview(app, id) {
            let _ = webview.set_focus();
        }
        self.emit(app);
        Ok(())
    }

    pub fn with_webview(&self, app: &AppHandle, id: &str, action: impl FnOnce(&Webview) -> tauri::Result<()>) -> Result<()> {
        match self.webview(app, id) {
            Some(webview) => action(&webview).map_err(|error| Error::Os(error.to_string())),
            None => Ok(()),
        }
    }

    pub fn set_pinned(&self, app: &AppHandle, id: &str, pinned: bool) {
        self.update(id, |tab| tab.pinned = pinned);
        // Pinned tabs sit first, in their existing order.
        self.lock().tabs.sort_by_key(|tab| !tab.pinned);
        self.emit(app);
    }

    pub fn set_group(&self, app: &AppHandle, id: &str, group: Option<String>) {
        self.update(id, |tab| tab.group = group.filter(|name| !name.trim().is_empty()));
        self.emit(app);
    }

    pub fn move_tab(&self, app: &AppHandle, id: &str, to_index: usize) {
        {
            let mut inner = self.lock();
            if let Some(from) = inner.tabs.iter().position(|tab| tab.id == id) {
                let tab = inner.tabs.remove(from);
                let to = to_index.min(inner.tabs.len());
                inner.tabs.insert(to, tab);
            }
        }
        self.emit(app);
    }

    // ------------------------------------------------------------- layout

    pub fn set_layout(&self, app: &AppHandle, layout: Layout) {
        let changed = {
            let mut inner = self.lock();
            let changed = inner.layout != layout;
            inner.layout = layout;
            changed
        };
        if changed {
            self.apply_layout(app);
        }
    }

    /// Shows a second tab beside the active one, or closes the split.
    pub fn set_split(&self, app: &AppHandle, id: Option<String>) -> Result<()> {
        let wake = {
            let mut inner = self.lock();
            let id = id.filter(|id| inner.active.as_deref() != Some(id) && inner.tabs.iter().any(|tab| &tab.id == id));
            inner.split = id.clone();
            id.filter(|id| inner.tabs.iter().any(|tab| &tab.id == id && tab.hibernated))
        };
        if let Some(id) = wake {
            self.wake(app, &id)?;
        }
        self.apply_layout(app);
        self.emit(app);
        Ok(())
    }

    /// Positions the visible tabs over the content area and hides the rest.
    pub fn apply_layout(&self, app: &AppHandle) {
        let (layout, active, split, ids) = {
            let inner = self.lock();
            (
                inner.layout,
                inner.active.clone(),
                inner.split.clone(),
                inner.tabs.iter().map(|tab| tab.id.clone()).collect::<Vec<_>>(),
            )
        };
        let visible_area = layout.width >= 1.0 && layout.height >= 1.0;
        const GAP: f64 = 2.0;
        for id in ids {
            let Some(webview) = self.webview(app, &id) else { continue };
            let is_active = active.as_deref() == Some(&id);
            let is_split = split.as_deref() == Some(&id);
            if !visible_area || !(is_active || is_split) {
                let _ = webview.hide();
                continue;
            }
            let (x, width) = match (&split, is_split) {
                (None, _) => (layout.x, layout.width),
                (Some(_), false) => (layout.x, (layout.width - GAP) / 2.0),
                (Some(_), true) => (layout.x + (layout.width + GAP) / 2.0, (layout.width - GAP) / 2.0),
            };
            let _ = webview.set_position(LogicalPosition::new(x, layout.y));
            let _ = webview.set_size(LogicalSize::new(width.max(1.0), layout.height));
            let _ = webview.show();
        }
    }

    // ------------------------------------------------------------ webview

    fn ensure_webview(&self, app: &AppHandle, id: &str) -> Result<()> {
        if self.webview(app, id).is_some() {
            return Ok(());
        }
        let url = self.page_url(id);
        let parsed = Url::parse(&url).map_err(|error| Error::Invalid(format!("bad URL: {error}")))?;
        let window = app.get_window(MAIN_WINDOW).ok_or_else(|| Error::Os("main window is gone".into()))?;
        let state = app.state::<AppState>();
        let privacy = state.privacy();
        let layout = self.lock().layout;

        let mut builder = WebviewBuilder::new(label(id), WebviewUrl::External(parsed))
            .position(LogicalPosition::new(layout.x, layout.y))
            .size(LogicalSize::new(layout.width.max(1.0), layout.height.max(1.0)));

        let mut document_start = privacy.document_start_script(state.session_seed);
        document_start.push_str(&shortcut_script(state.session_seed));
        builder = builder.initialization_script_for_all_frames(document_start);

        // Browser shortcuts pressed inside a page arrive as tagged console messages.
        let (key_app, key_id, key_tag) = (app.clone(), id.to_owned(), shortcut_tag(state.session_seed));
        builder = builder.on_console_message(move |message| {
            if let Some(command) = message.message.strip_prefix(key_tag.as_str()) {
                let (app, id, command) = (key_app.clone(), key_id.clone(), command.to_owned());
                tauri::async_runtime::spawn(async move { on_page_shortcut(&app, &id, &command) });
            }
        });

        let (nav_app, nav_id) = (app.clone(), id.to_owned());
        builder = builder.on_navigation(move |target| on_navigation(&nav_app, &nav_id, target));

        let (load_app, load_id) = (app.clone(), id.to_owned());
        builder = builder.on_page_load(move |webview, payload| {
            on_page_load(&load_app, &load_id, &webview, payload.url(), payload.event());
        });

        let (title_app, title_id) = (app.clone(), id.to_owned());
        builder = builder.on_document_title_changed(move |_webview, title| {
            let tabs = title_app.state::<TabManager>();
            let mut url = String::new();
            tabs.update(&title_id, |tab| {
                tab.title = title.clone();
                url = tab.url.clone();
            });
            let _ = title_app.state::<AppState>().db.set_history_title(&url, &title);
            tabs.emit(&title_app);
        });

        let (window_app, window_id) = (app.clone(), id.to_owned());
        builder = builder.on_new_window(move |target, _features| {
            // Popups and target=_blank links become tabs next to their opener.
            let (app, opener, target) = (window_app.clone(), window_id.clone(), target.to_string());
            tauri::async_runtime::spawn(async move {
                let tabs = app.state::<TabManager>();
                if let Err(error) = tabs.create(&app, Some(target), false, Some(&opener)) {
                    log::warn!("could not open tab: {error}");
                }
            });
            NewWindowResponse::Deny
        });

        let download_app = app.clone();
        builder = builder.on_download(move |_webview, event| on_download(&download_app, event));

        let webview = window.add_child(builder).map_err(|error| Error::Os(error.to_string()))?;
        self.attach_devtools(app, &webview, id);
        Ok(())
    }

    /// Subscribes to the tab's DevTools events: request interception for the
    /// shield, and screenshot results for hibernation.
    fn attach_devtools(&self, app: &AppHandle, webview: &Webview, id: &str) {
        let (app_handle, tab_id, target) = (app.clone(), id.to_owned(), webview.clone());
        let subscribed = webview.on_dev_tools_protocol(move |protocol| match protocol {
            DevToolsProtocol::Event { method, params } if method == "Fetch.requestPaused" => {
                on_request_paused(&app_handle, &tab_id, &target, &params);
            }
            DevToolsProtocol::MethodResult { message_id, success, result } => {
                let tabs = app_handle.state::<TabManager>();
                let pending = tabs.lock().screenshots.remove(&message_id);
                if let Some(tab) = pending {
                    let screenshot = success.then(|| screenshot_from_result(&result)).flatten();
                    let app = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        app.state::<TabManager>().finish_hibernation(&app, &tab, screenshot);
                    });
                }
            }
            _ => {}
        });
        if let Err(error) = subscribed {
            log::warn!("shield unavailable for tab {id}: {error}");
            return;
        }
        self.send_devtools(
            webview,
            "Fetch.enable",
            serde_json::json!({ "patterns": [{ "urlPattern": "*", "requestStage": "Request" }] }),
        );
    }

    // -------------------------------------------------------- hibernation

    /// Starts hibernating a background tab. Returns false if it is not eligible.
    pub fn hibernate(&self, app: &AppHandle, id: &str) -> bool {
        let Some(webview) = self.webview(app, id) else { return false };
        {
            let mut inner = self.lock();
            let shown = inner.active.as_deref() == Some(id) || inner.split.as_deref() == Some(id);
            let exists = inner.tabs.iter().any(|tab| tab.id == id && !tab.hibernated);
            if shown || !exists || !inner.hibernating.insert(id.to_owned()) {
                return false;
            }
        }
        // Capture scroll position first; parse handles both raw-JSON-object and
        // JSON-encoded-string forms that different CEF versions may produce.
        let (scroll_app, scroll_id) = (app.clone(), id.to_owned());
        let _ = webview.eval_with_callback(hibernation::CAPTURE_STATE_SCRIPT, move |raw| {
            // Try JSON-encoded string first ("…"), then bare object ({…}).
            let parsed = serde_json::from_str::<String>(&raw)
                .ok()
                .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
                .or_else(|| serde_json::from_str::<serde_json::Value>(&raw).ok());
            let scroll = parsed
                .as_ref()
                .and_then(|state| state.get("scrollY"))
                .and_then(|y| y.as_f64())
                .unwrap_or(0.0);
            scroll_app.state::<TabManager>().update(&scroll_id, |tab| tab.restore_scroll = scroll);
        });
        if let Some(message_id) = self.send_devtools(
            &webview,
            "Page.captureScreenshot",
            serde_json::json!({ "format": "jpeg", "quality": 40, "optimizeForSpeed": true }),
        ) {
            self.lock().screenshots.insert(message_id, id.to_owned());
        }
        // Hidden tabs often cannot produce a frame; do not wait for long.
        let (app, id) = (app.clone(), id.to_owned());
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1200)).await;
            app.state::<TabManager>().finish_hibernation(&app, &id, None);
        });
        true
    }

    fn finish_hibernation(&self, app: &AppHandle, id: &str, screenshot: Option<Vec<u8>>) {
        let saved = {
            let mut inner = self.lock();
            if !inner.hibernating.remove(id) {
                return; // already finished, cancelled or closed
            }
            inner.screenshots.retain(|_, tab| tab != id);
            let shown = inner.active.as_deref() == Some(id) || inner.split.as_deref() == Some(id);
            if shown {
                return; // the user came back to it meanwhile
            }
            let Some(tab) = inner.tabs.iter_mut().find(|tab| tab.id == id) else { return };
            tab.hibernated = true;
            tab.loading = false;
            HibernatedTab {
                tab_id: id.to_owned(),
                url: tab.url.clone(),
                title: tab.title.clone(),
                scroll_y: tab.restore_scroll,
                screenshot,
                hibernated_at: now_ms(),
            }
        };
        if let Err(error) = app.state::<AppState>().db.save_hibernated(&saved) {
            log::warn!("could not save hibernated tab: {error}");
        }
        if let Some(webview) = self.webview(app, id) {
            let _ = webview.close();
        }
        self.emit(app);
    }

    fn wake(&self, app: &AppHandle, id: &str) -> Result<()> {
        let stored = app.state::<AppState>().db.take_hibernated(id)?;
        self.update(id, |tab| {
            tab.hibernated = false;
            tab.loading = true;
            tab.restore_scroll = stored.as_ref().map_or(0.0, |saved| saved.scroll_y);
        });
        self.lock().hibernating.remove(id);
        self.ensure_webview(app, id)
    }

    /// Hibernates whatever the policy selects. Returns how many tabs were frozen.
    pub fn sweep(&self, app: &AppHandle, pressure: Pressure) -> usize {
        let state = app.state::<AppState>();
        let policy = state.hibernation.read().unwrap_or_else(|p| p.into_inner()).clone();
        let activity: Vec<TabActivity> = {
            let inner = self.lock();
            inner
                .tabs
                .iter()
                .filter(|tab| !tab.hibernated && !tab.url.is_empty() && !inner.hibernating.contains(&tab.id))
                .map(|tab| TabActivity {
                    id: tab.id.clone(),
                    visible: inner.active.as_deref() == Some(&tab.id) || inner.split.as_deref() == Some(&tab.id),
                    pinned: tab.pinned,
                    audible: false,
                    last_active: tab.last_active,
                })
                .collect()
        };
        policy
            .select(Instant::now(), &activity, pressure)
            .iter()
            .filter(|id| self.hibernate(app, id))
            .count()
    }

    // ------------------------------------------------------------ session

    pub fn session(&self) -> Vec<SessionTab> {
        let inner = self.lock();
        inner
            .tabs
            .iter()
            .map(|tab| SessionTab {
                url: tab.url.clone(),
                title: tab.title.clone(),
                pinned: tab.pinned,
                group: tab.group.clone(),
                active: inner.active.as_deref() == Some(&tab.id),
            })
            .collect()
    }

    pub fn active_id(&self) -> Option<String> {
        self.lock().active.clone()
    }

    pub fn live_tab_count(&self) -> usize {
        self.lock().tabs.iter().filter(|tab| !tab.hibernated && !tab.url.is_empty()).count()
    }
}

// ----------------------------------------------------------------- handlers

/// Prefix of the console messages that carry a shortcut. It contains a
/// per-launch random value so pages cannot fake one.
fn shortcut_tag(seed: u64) -> String {
    format!("__aether:{:016x}:", seed.rotate_left(17) ^ 0x9e37_79b9_7f4a_7c15)
}

/// Captures browser-level shortcuts before the page sees them. Web pages have
/// no channel to the kernel, so the key is reported through the console.
fn shortcut_script(seed: u64) -> String {
    format!(
        r#"(function(){{var say=console.debug.bind(console),tag='{tag}';
window.addEventListener('keydown',function(e){{var m=e.ctrlKey||e.metaKey,k=(e.key||'').toLowerCase(),c=null;
if(m&&!e.altKey){{if(k==='t'&&!e.shiftKey)c='new-tab';else if(k==='w')c='close-tab';else if(k==='l')c='focus-url';
else if(k==='tab')c=e.shiftKey?'prev-tab':'next-tab';else if(k==='d')c='bookmark';else if(k===',')c='settings';
else if(k.length===1&&k>='1'&&k<='9')c='tab-'+k;}}
else if(e.altKey&&!m){{if(k==='arrowleft')c='back';else if(k==='arrowright')c='forward';}}
else if(k==='f12')c='devtools';
if(c){{e.preventDefault();e.stopImmediatePropagation();say(tag+c);}}}},true);}})();"#,
        tag = shortcut_tag(seed)
    )
}

fn on_page_shortcut(app: &AppHandle, id: &str, command: &str) {
    let tabs = app.state::<TabManager>();
    let focus_shell = || {
        if let Some(shell) = app.get_webview(SHELL_WEBVIEW) {
            let _ = shell.set_focus();
        }
    };
    let ids: Vec<String> = tabs.lock().tabs.iter().map(|tab| tab.id.clone()).collect();
    let position = ids.iter().position(|tab| tab == id).unwrap_or(0);
    let result = match command {
        "new-tab" => {
            focus_shell();
            tabs.create(app, None, false, None).map(|_| ())
        }
        "close-tab" => tabs.close(app, id),
        "next-tab" => tabs.activate(app, &ids[(position + 1) % ids.len().max(1)]),
        "prev-tab" => tabs.activate(app, &ids[(position + ids.len() - 1) % ids.len().max(1)]),
        "back" => tabs.with_webview(app, id, |webview| webview.go_back()),
        "forward" => tabs.with_webview(app, id, |webview| webview.go_forward()),
        "devtools" => tabs.with_webview(app, id, |webview| {
            if webview.is_devtools_open() {
                webview.close_devtools();
            } else {
                webview.open_devtools();
            }
            Ok(())
        }),
        "focus-url" | "bookmark" | "settings" => {
            focus_shell();
            app.emit(EVENT_SHORTCUT, command).map_err(|error| Error::Os(error.to_string()))
        }
        other => match other.strip_prefix("tab-").and_then(|n| n.parse::<usize>().ok()) {
            // Ctrl+9 selects the last tab, as in Chrome.
            Some(9) => ids.last().map_or(Ok(()), |last| tabs.activate(app, last)),
            Some(n) if n >= 1 && n <= ids.len() => tabs.activate(app, &ids[n - 1]),
            _ => Ok(()),
        },
    };
    if let Err(error) = result {
        log::debug!("shortcut {command} failed: {error}");
    }
}

fn on_navigation(app: &AppHandle, id: &str, target: &Url) -> bool {
    let state = app.state::<AppState>();
    if state.privacy().strip_tracking_params && matches!(target.scheme(), "http" | "https") {
        if let Some(clean) = strip_tracking_params(target) {
            // Cancel this navigation and go to the clean URL instead.
            let (app, id) = (app.clone(), id.to_owned());
            tauri::async_runtime::spawn(async move {
                if let Some(webview) = app.get_webview(&label(&id)) {
                    let _ = webview.navigate(clean);
                }
            });
            return false;
        }
    }
    true
}

fn on_page_load(app: &AppHandle, id: &str, webview: &Webview, url: &Url, event: PageLoadEvent) {
    let state = app.state::<AppState>();
    let tabs = app.state::<TabManager>();
    let url_text = url.to_string();
    let inject_cosmetics = || {
        let script = cosmetics_injection_script(&state.shield.cosmetics(&url_text));
        if !script.is_empty() {
            let _ = webview.eval(script);
        }
    };
    match event {
        PageLoadEvent::Started => {
            state.shield.reset_tab(id);
            tabs.update(id, |tab| {
                if tab.url != url_text {
                    tab.title.clear();
                }
                tab.url = url_text.clone();
                tab.loading = true;
            });
            inject_cosmetics();
        }
        PageLoadEvent::Finished => {
            let mut scroll = 0.0;
            let mut title = String::new();
            tabs.update(id, |tab| {
                tab.url = url_text.clone();
                tab.loading = false;
                scroll = std::mem::take(&mut tab.restore_scroll);
                title = tab.title.clone();
            });
            inject_cosmetics();
            let restore = hibernation::restore_scroll_script(scroll);
            if !restore.is_empty() {
                let _ = webview.eval(restore);
            }
            let _ = state.db.record_visit(&url_text, &title);
            // Collect CSS classes/ids from the page and inject generic cosmetic
            // hide rules (e.g. ##.adsbygoogle). Done after DOMContentLoaded so
            // the initial element set is available.
            let (gc_app, gc_id) = (app.clone(), id.to_owned());
            let gc_url = url_text.clone();
            let _ = webview.eval_with_callback(
                r#"(function(){try{var c=Array.from(new Set(Array.from(document.querySelectorAll('[class]')).flatMap(function(el){return Array.from(el.classList)}))).slice(0,2000);var i=Array.from(document.querySelectorAll('[id]')).map(function(el){return el.id;}).filter(Boolean).slice(0,500);return JSON.stringify({classes:c,ids:i});}catch(e){return JSON.stringify({classes:[],ids:[]});}})();"#,
                move |raw| {
                    let parsed = serde_json::from_str::<String>(&raw)
                        .ok()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                        .or_else(|| serde_json::from_str::<serde_json::Value>(&raw).ok());
                    let Some(parsed) = parsed else { return };
                    let classes: Vec<String> = parsed.get("classes")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default();
                    let ids: Vec<String> = parsed.get("ids")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default();
                    let state = gc_app.state::<AppState>();
                    let tabs = gc_app.state::<TabManager>();
                    let css = state.shield.generic_cosmetics(&gc_url, &classes, &ids);
                    if css.is_empty() { return; }
                    if let Some(wv) = tabs.webview(&gc_app, &gc_id) {
                        let encoded = serde_json::to_string(&css).unwrap_or_default();
                        let script = format!(
                            "(function(){{var s=document.createElement('style');s.dataset.aether='gc';s.textContent={encoded};(document.head||document.documentElement).appendChild(s);}})();"
                        );
                        let _ = wv.eval(script);
                    }
                },
            );
        }
    }
    tabs.emit(app);
}

fn on_download(app: &AppHandle, event: DownloadEvent<'_>) -> bool {
    let state = app.state::<AppState>();
    let record = match event {
        DownloadEvent::Requested { url, destination } => DownloadRecord {
            id: destination.to_string_lossy().into_owned(),
            url: url.to_string(),
            path: destination.to_string_lossy().into_owned(),
            state: "in_progress".into(),
            started_at: now_ms(),
        },
        DownloadEvent::Finished { url, path, success } => {
            let path = path.map(|path| path.to_string_lossy().into_owned()).unwrap_or_default();
            DownloadRecord {
                id: path.clone(),
                url: url.to_string(),
                path,
                state: if success { "completed" } else { "failed" }.into(),
                started_at: now_ms(),
            }
        }
        _ => return true,
    };
    if !record.id.is_empty() {
        let _ = state.db.upsert_download(&record);
        let _ = app.emit(EVENT_DOWNLOADS, ());
    }
    true
}

/// Decides the fate of one intercepted request.
fn on_request_paused(app: &AppHandle, tab_id: &str, webview: &Webview, params: &[u8]) {
    let tabs = app.state::<TabManager>();
    let Ok(event) = serde_json::from_slice::<serde_json::Value>(params) else { return };
    let Some(request_id) = event.get("requestId").and_then(|id| id.as_str()) else { return };
    let url = event.pointer("/request/url").and_then(|url| url.as_str()).unwrap_or_default();
    let resource_type = resource_type_from_cdp(event.get("resourceType").and_then(|t| t.as_str()).unwrap_or(""));
    let page_url = tabs.page_url(tab_id);

    let decision = if url.starts_with("http") {
        app.state::<AppState>().shield.check(url, &page_url, resource_type, Some(tab_id))
    } else {
        Decision::Allow
    };
    match decision {
        Decision::Allow => {
            tabs.send_devtools(webview, "Fetch.continueRequest", serde_json::json!({ "requestId": request_id }));
        }
        Decision::Block => {
            tabs.send_devtools(
                webview,
                "Fetch.failRequest",
                serde_json::json!({ "requestId": request_id, "errorReason": "BlockedByClient" }),
            );
        }
        Decision::Redirect(resource) => {
            // `resource` is a data: URL holding a harmless stand-in.
            let (mime, body) = split_data_url(&resource);
            tabs.send_devtools(
                webview,
                "Fetch.fulfillRequest",
                serde_json::json!({
                    "requestId": request_id,
                    "responseCode": 200,
                    "responseHeaders": [
                        { "name": "Content-Type", "value": mime },
                        { "name": "Access-Control-Allow-Origin", "value": "*" }
                    ],
                    "body": body,
                }),
            );
        }
    }
}

/// Splits `data:<mime>;base64,<body>` into the MIME type and base64 body.
fn split_data_url(data_url: &str) -> (String, String) {
    let rest = data_url.strip_prefix("data:").unwrap_or(data_url);
    let (meta, body) = rest.split_once(',').unwrap_or(("text/plain", ""));
    let mime = meta.split(';').next().filter(|mime| !mime.is_empty()).unwrap_or("text/plain").to_owned();
    if meta.ends_with(";base64") {
        (mime, body.to_owned())
    } else {
        (mime, base64::engine::general_purpose::STANDARD.encode(body))
    }
}

fn screenshot_from_result(result: &[u8]) -> Option<Vec<u8>> {
    let value: serde_json::Value = serde_json::from_slice(result).ok()?;
    let data = value.get("data")?.as_str()?;
    base64::engine::general_purpose::STANDARD.decode(data).ok().filter(|bytes| !bytes.is_empty())
}

/// Tab id for a webview, if it is a tab.
pub fn tab_id_of(webview: &Webview) -> Option<String> {
    id_from_label(webview.label()).map(str::to_owned)
}
