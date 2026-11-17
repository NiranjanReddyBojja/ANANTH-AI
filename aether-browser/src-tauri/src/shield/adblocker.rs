//! Ad and tracker blocker built on Brave's `adblock` engine.
//!
//! Requests are checked before they leave the browser; matching ones are
//! cancelled. Cosmetic filters become a stylesheet (and, when uBlock Origin's
//! resources are loaded, scriptlets) injected into each page.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;
use adblock::resources::Resource;
use adblock::Engine;
use serde::Serialize;

use crate::{Error, Result};

const BUILTIN_FILTERS: &str = include_str!("builtin_filters.txt");

/// Filter lists downloaded by [`update_lists`]: (file name, URL).
pub const FILTER_LISTS: &[(&str, &str)] = &[
    ("easylist.txt", "https://easylist.to/easylist/easylist.txt"),
    ("easyprivacy.txt", "https://easylist.to/easylist/easyprivacy.txt"),
    ("ublock-filters.txt", "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters.txt"),
    ("ublock-privacy.txt", "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/privacy.txt"),
    ("ublock-quick-fixes.txt", "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/quick-fixes.txt"),
    ("ublock-unbreak.txt", "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/unbreak.txt"),
];

/// Scriptlets and redirect resources (uBlock Origin's, as packaged by Brave).
pub const RESOURCES: (&str, &str) = (
    "resources.json",
    "https://raw.githubusercontent.com/brave/adblock-resources/master/dist/resources.json",
);

/// How long downloaded lists are considered fresh.
pub const LIST_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Block,
    /// Serve this `data:` URL instead of making the request.
    Redirect(String),
}

/// What to inject into a page for cosmetic filtering.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Cosmetics {
    pub css: String,
    pub script: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ShieldStats {
    pub enabled: bool,
    pub blocked_total: u64,
    pub blocked_by_tab: HashMap<String, u64>,
    pub filter_lists: usize,
    pub using_full_lists: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct UpdateReport {
    pub updated: Vec<String>,
    pub failed: Vec<String>,
}

pub struct Shield {
    engine: RwLock<Engine>,
    enabled: AtomicBool,
    /// Hosts where the user turned the shield off.
    allowed_sites: RwLock<HashSet<String>>,
    blocked_total: AtomicU64,
    blocked_by_tab: Mutex<HashMap<String, u64>>,
    list_count: AtomicU64,
    full_lists: AtomicBool,
}

impl Default for Shield {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Shield {
    /// A shield using only the small built-in list.
    pub fn builtin() -> Self {
        Self::from_engine(build_engine(&[BUILTIN_FILTERS], None), 1, false)
    }

    /// A shield built from the given filter list texts (plus the built-in list).
    pub fn from_lists(lists: &[&str]) -> Self {
        let mut all = vec![BUILTIN_FILTERS];
        all.extend_from_slice(lists);
        Self::from_engine(build_engine(&all, None), all.len(), !lists.is_empty())
    }

    fn from_engine(engine: Engine, lists: usize, full: bool) -> Self {
        Self {
            engine: RwLock::new(engine),
            enabled: AtomicBool::new(true),
            allowed_sites: RwLock::new(HashSet::new()),
            blocked_total: AtomicU64::new(0),
            blocked_by_tab: Mutex::new(HashMap::new()),
            list_count: AtomicU64::new(lists as u64),
            full_lists: AtomicBool::new(full),
        }
    }

    /// Rebuilds the engine from the lists cached in `dir`. Returns how many
    /// lists were loaded; zero leaves the current engine untouched.
    pub fn reload_from_cache(&self, dir: &Path) -> usize {
        let texts: Vec<String> = FILTER_LISTS
            .iter()
            .filter_map(|(name, _)| std::fs::read_to_string(dir.join(name)).ok())
            .collect();
        if texts.is_empty() {
            return 0;
        }
        let resources = std::fs::read_to_string(dir.join(RESOURCES.0)).ok();
        let mut all: Vec<&str> = vec![BUILTIN_FILTERS];
        all.extend(texts.iter().map(String::as_str));
        let engine = build_engine(&all, resources.as_deref());
        *self.engine.write().unwrap_or_else(|p| p.into_inner()) = engine;
        self.list_count.store(all.len() as u64, Ordering::Relaxed);
        self.full_lists.store(true, Ordering::Relaxed);
        texts.len()
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Turns the shield on or off for one site (by host name).
    pub fn set_site_enabled(&self, host: &str, enabled: bool) {
        let mut sites = self.allowed_sites.write().unwrap_or_else(|p| p.into_inner());
        if enabled {
            sites.remove(host);
        } else {
            sites.insert(host.to_ascii_lowercase());
        }
    }

    pub fn is_site_enabled(&self, host: &str) -> bool {
        !self.allowed_sites.read().unwrap_or_else(|p| p.into_inner()).contains(&host.to_ascii_lowercase())
    }

    pub fn allowed_sites(&self) -> Vec<String> {
        let mut sites: Vec<String> =
            self.allowed_sites.read().unwrap_or_else(|p| p.into_inner()).iter().cloned().collect();
        sites.sort();
        sites
    }

    pub fn set_allowed_sites(&self, hosts: impl IntoIterator<Item = String>) {
        *self.allowed_sites.write().unwrap_or_else(|p| p.into_inner()) =
            hosts.into_iter().map(|host| host.to_ascii_lowercase()).collect();
    }

    fn active_for(&self, page_url: &str) -> bool {
        if !self.is_enabled() {
            return false;
        }
        match url::Url::parse(page_url).ok().and_then(|url| url.host_str().map(str::to_owned)) {
            Some(host) => self.is_site_enabled(&host),
            None => true,
        }
    }

    /// Decides whether a request may go out.
    ///
    /// `resource_type` uses the engine's names (`script`, `image`, `xhr`, ...);
    /// see [`resource_type_from_cdp`]. Top-level documents are never blocked.
    pub fn check(&self, url: &str, page_url: &str, resource_type: &str, tab: Option<&str>) -> Decision {
        if resource_type == "document" || !self.active_for(page_url) {
            return Decision::Allow;
        }
        let Ok(request) = Request::new(url, page_url, resource_type, "GET") else {
            return Decision::Allow;
        };
        let result = self.engine.read().unwrap_or_else(|p| p.into_inner()).check_network_request(&request);
        if !result.should_block() {
            return Decision::Allow;
        }
        self.blocked_total.fetch_add(1, Ordering::Relaxed);
        if let Some(tab) = tab {
            *self.blocked_by_tab.lock().unwrap_or_else(|p| p.into_inner()).entry(tab.to_owned()).or_default() += 1;
        }
        match result.redirect {
            Some(resource) => Decision::Redirect(resource),
            None => Decision::Block,
        }
    }

    /// Stylesheet and scriptlets to inject into `page_url`.
    pub fn cosmetics(&self, page_url: &str) -> Cosmetics {
        if !self.active_for(page_url) {
            return Cosmetics::default();
        }
        let resources = self.engine.read().unwrap_or_else(|p| p.into_inner()).url_cosmetic_resources(page_url);
        let mut selectors: Vec<&str> = resources
            .hide_selectors
            .iter()
            .filter(|selector| !resources.exceptions.contains(*selector))
            .map(String::as_str)
            .collect();
        selectors.sort_unstable();
        // Chunked so one invalid selector cannot disable the whole sheet.
        let css = selectors
            .chunks(100)
            .map(|chunk| format!("{}{{display:none!important}}", chunk.join(",")))
            .collect::<Vec<_>>()
            .join("\n");
        let mut script = resources.injected_script;
        if let Some(builtin) = builtin_scriptlet(page_url) {
            script.push_str(builtin);
        }
        Cosmetics { css, script }
    }

    /// CSS hide rules for generic cosmetic filters matching the classes/ids found
    /// on `page_url`. Call this after the page has loaded with the classes and ids
    /// collected from the DOM. Returns an empty string when generichide is active.
    pub fn generic_cosmetics(&self, page_url: &str, classes: &[String], ids: &[String]) -> String {
        if !self.active_for(page_url) {
            return String::new();
        }
        let engine = self.engine.read().unwrap_or_else(|p| p.into_inner());
        let resources = engine.url_cosmetic_resources(page_url);
        if resources.generichide {
            return String::new();
        }
        let selectors = engine.hidden_class_id_selectors(
            classes.iter().map(String::as_str),
            ids.iter().map(String::as_str),
            &resources.exceptions,
        );
        if selectors.is_empty() {
            return String::new();
        }
        selectors
            .chunks(100)
            .map(|chunk| format!("{}{{display:none!important}}", chunk.join(",")))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn reset_tab(&self, tab: &str) {
        self.blocked_by_tab.lock().unwrap_or_else(|p| p.into_inner()).remove(tab);
    }

    pub fn stats(&self) -> ShieldStats {
        ShieldStats {
            enabled: self.is_enabled(),
            blocked_total: self.blocked_total.load(Ordering::Relaxed),
            blocked_by_tab: self.blocked_by_tab.lock().unwrap_or_else(|p| p.into_inner()).clone(),
            filter_lists: self.list_count.load(Ordering::Relaxed) as usize,
            using_full_lists: self.full_lists.load(Ordering::Relaxed),
        }
    }
}

/// Strips ad payloads from YouTube's player responses, in the style of
/// uBlock Origin's `json-prune`. YouTube changes often; treat this as
/// best-effort. Loading uBlock Origin's own scriptlets via `resources.json`
/// supersedes it.
const YOUTUBE_SCRIPTLET: &str = r#"(function(){
if (window.__aetherYt) return; window.__aetherYt = true;
var KEYS = ['adPlacements', 'adSlots', 'playerAds', 'adBreakHeartbeatParams'];
function prune(o) { try { if (o && typeof o === 'object') { for (var i = 0; i < KEYS.length; i++) { if (KEYS[i] in o) delete o[KEYS[i]]; }
  if (o.playerResponse) prune(o.playerResponse); } } catch (e) {} return o; }
var parse = JSON.parse; JSON.parse = function () { return prune(parse.apply(this, arguments)); };
var json = Response.prototype.json; Response.prototype.json = function () { return json.apply(this, arguments).then(prune); };
var initial; try { Object.defineProperty(window, 'ytInitialPlayerResponse', { configurable: true,
  get: function () { return initial; }, set: function (v) { initial = prune(v); } }); } catch (e) {}
})();"#;

fn builtin_scriptlet(page_url: &str) -> Option<&'static str> {
    let host = url::Url::parse(page_url).ok()?.host_str()?.to_ascii_lowercase();
    (host == "youtube.com" || host.ends_with(".youtube.com")).then_some(YOUTUBE_SCRIPTLET)
}

fn build_engine(lists: &[&str], resources_json: Option<&str>) -> Engine {
    let mut filter_set = FilterSet::new(false);
    for list in lists {
        filter_set.add_filter_list((*list).to_owned(), ParseOptions::default());
    }
    let mut engine = Engine::new_with_filter_set(filter_set);
    if let Some(resources) = resources_json.and_then(|json| serde_json::from_str::<Vec<Resource>>(json).ok()) {
        engine.use_resources(resources);
    }
    engine
}

/// Maps a Chrome DevTools Protocol `Network.ResourceType` to the engine's name.
pub fn resource_type_from_cdp(cdp_type: &str) -> &'static str {
    match cdp_type {
        "Document" => "document",
        "Stylesheet" => "stylesheet",
        "Image" => "image",
        "Media" => "media",
        "Font" => "font",
        "Script" => "script",
        "XHR" | "Fetch" | "EventSource" => "xhr",
        "WebSocket" => "websocket",
        "Ping" | "CSPViolationReport" => "ping",
        _ => "other",
    }
}

/// JavaScript that applies [`Cosmetics`] to the current document.
pub fn cosmetics_injection_script(cosmetics: &Cosmetics) -> String {
    if cosmetics.css.is_empty() && cosmetics.script.is_empty() {
        return String::new();
    }
    let css = serde_json::to_string(&cosmetics.css).unwrap_or_else(|_| "\"\"".into());
    format!(
        "(function(){{try{{var css={css};if(css){{var s=document.getElementById('__aether_shield');\
if(!s){{s=document.createElement('style');s.id='__aether_shield';\
(document.head||document.documentElement).appendChild(s);}}s.textContent=css;}}}}catch(e){{}}\
try{{{script}}}catch(e){{}}}})();",
        script = cosmetics.script
    )
}

/// Paths of cached lists that are missing or older than [`LIST_MAX_AGE`].
pub fn stale_lists(dir: &Path) -> Vec<(PathBuf, &'static str)> {
    FILTER_LISTS
        .iter()
        .chain(std::iter::once(&RESOURCES))
        .filter_map(|(name, url)| {
            let path = dir.join(name);
            let fresh = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age < LIST_MAX_AGE);
            (!fresh).then_some((path, *url))
        })
        .collect()
}

/// Downloads stale filter lists into `dir`. Blocking; call it off the UI thread.
pub fn update_lists(dir: &Path) -> Result<UpdateReport> {
    std::fs::create_dir_all(dir)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|error| Error::Network(error.to_string()))?;
    let mut report = UpdateReport::default();
    for (path, url) in stale_lists(dir) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let body = client
            .get(url)
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.bytes());
        match body {
            // Guard against captive portals and error pages.
            Ok(bytes) if bytes.len() > 1024 => {
                let temp = path.with_extension("tmp");
                std::fs::write(&temp, &bytes)?;
                std::fs::rename(&temp, &path)?;
                report.updated.push(name);
            }
            _ => report.failed.push(name),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "https://news.example/article";

    #[test]
    fn blocks_ads_and_trackers_but_not_content() {
        let shield = Shield::builtin();
        assert_eq!(
            shield.check("https://securepubads.g.doubleclick.net/tag/js/gpt.js", PAGE, "script", Some("t1")),
            Decision::Block
        );
        assert_eq!(
            shield.check("https://www.google-analytics.com/collect?v=1", PAGE, "xhr", Some("t1")),
            Decision::Block
        );
        assert_eq!(shield.check("https://news.example/app.js", PAGE, "script", Some("t1")), Decision::Allow);
        assert_eq!(shield.check("https://cdn.example/photo.jpg", PAGE, "image", None), Decision::Allow);

        let stats = shield.stats();
        assert_eq!(stats.blocked_total, 2);
        assert_eq!(stats.blocked_by_tab["t1"], 2);
    }

    #[test]
    fn never_blocks_top_level_navigation() {
        let shield = Shield::builtin();
        assert_eq!(
            shield.check("https://doubleclick.net/", "https://doubleclick.net/", "document", None),
            Decision::Allow
        );
    }

    #[test]
    fn third_party_option_is_respected() {
        let shield = Shield::builtin();
        assert_eq!(shield.check("https://cdn.taboola.com/loader.js", PAGE, "script", None), Decision::Block);
        assert_eq!(
            shield.check("https://cdn.taboola.com/loader.js", "https://www.taboola.com/", "script", None),
            Decision::Allow
        );
    }

    #[test]
    fn site_and_global_switches() {
        let shield = Shield::builtin();
        let ad = "https://ad.doubleclick.net/x.js";
        shield.set_site_enabled("news.example", false);
        assert_eq!(shield.check(ad, PAGE, "script", None), Decision::Allow);
        assert_eq!(shield.check(ad, "https://other.example/", "script", None), Decision::Block);
        shield.set_site_enabled("news.example", true);
        assert_eq!(shield.check(ad, PAGE, "script", None), Decision::Block);
        shield.set_enabled(false);
        assert_eq!(shield.check(ad, PAGE, "script", None), Decision::Allow);
    }

    #[test]
    fn exception_rules_win() {
        let shield = Shield::from_lists(&["||tracker.example^\n@@||tracker.example/allowed.js"]);
        assert_eq!(shield.check("https://tracker.example/t.js", PAGE, "script", None), Decision::Block);
        assert_eq!(shield.check("https://tracker.example/allowed.js", PAGE, "script", None), Decision::Allow);
    }

    #[test]
    fn cosmetic_filters_become_css() {
        let shield = Shield::builtin();
        let youtube = shield.cosmetics("https://www.youtube.com/watch?v=abc");
        assert!(youtube.css.contains("ytd-ad-slot-renderer"));
        assert!(youtube.css.contains("display:none!important"));

        let other = shield.cosmetics(PAGE);
        assert!(!other.css.contains("ytd-ad-slot-renderer"), "site-specific rules stay on their site");

        assert!(youtube.script.contains("adPlacements"));
        assert!(other.script.is_empty());
        let script = cosmetics_injection_script(&youtube);
        assert!(script.contains("__aether_shield"));
        assert_eq!(cosmetics_injection_script(&Cosmetics::default()), "");
    }

    #[test]
    fn reloads_from_cached_lists() {
        let dir = tempfile::tempdir().unwrap();
        let shield = Shield::builtin();
        assert_eq!(shield.reload_from_cache(dir.path()), 0);
        std::fs::write(dir.path().join("easylist.txt"), "||cached-ads.example^\n").unwrap();
        assert_eq!(shield.reload_from_cache(dir.path()), 1);
        assert_eq!(shield.check("https://cached-ads.example/a.js", PAGE, "script", None), Decision::Block);
        assert!(shield.stats().using_full_lists);
        // Everything except the list we just wrote still needs downloading.
        assert_eq!(stale_lists(dir.path()).len(), FILTER_LISTS.len());
    }

    /// Downloads the real lists; run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "needs network access"]
    fn real_lists_download_and_block() {
        let dir = tempfile::tempdir().unwrap();
        let report = update_lists(dir.path()).unwrap();
        assert!(report.failed.is_empty(), "failed downloads: {:?}", report.failed);
        let shield = Shield::builtin();
        let started = std::time::Instant::now();
        assert_eq!(shield.reload_from_cache(dir.path()), FILTER_LISTS.len());
        eprintln!("engine built from full lists in {:?}", started.elapsed());

        let page = "https://www.example-news.com/story";
        for (url, kind) in [
            ("https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js", "script"),
            ("https://www.googletagmanager.com/gtm.js?id=GTM-XXXX", "script"),
            ("https://static.ads-twitter.com/uwt.js", "script"),
            ("https://c.amazon-adsystem.com/aax2/apstag.js", "script"),
            ("https://sb.scorecardresearch.com/beacon.js", "script"),
        ] {
            assert_ne!(shield.check(url, page, kind, None), Decision::Allow, "{url} should be blocked");
        }
        for url in ["https://www.example-news.com/main.css", "https://upload.wikimedia.org/wikipedia/commons/a/a9/Example.jpg"] {
            assert_eq!(shield.check(url, page, "image", None), Decision::Allow, "{url} should load");
        }
        let youtube = shield.cosmetics("https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        eprintln!("youtube: {} bytes of css, {} bytes of scriptlets", youtube.css.len(), youtube.script.len());
        assert!(youtube.css.len() > 500, "YouTube cosmetic rules from the full lists");
        assert!(youtube.script.contains("adPlacements"), "built-in YouTube scriptlet");
        assert!(stale_lists(dir.path()).is_empty());
    }

    #[test]
    fn maps_cdp_resource_types() {
        assert_eq!(resource_type_from_cdp("Script"), "script");
        assert_eq!(resource_type_from_cdp("Fetch"), "xhr");
        assert_eq!(resource_type_from_cdp("Manifest"), "other");
    }
}
