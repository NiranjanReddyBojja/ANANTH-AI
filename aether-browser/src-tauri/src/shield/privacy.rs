//! Privacy protections: HTTPS-only upgrades, tracking-parameter stripping,
//! fingerprint noise, WebRTC leak policy, third-party cookie blocking and
//! DNS-over-HTTPS.

use serde::{Deserialize, Serialize};
use url::{Host, Url};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebRtcPolicy {
    /// Chromium default: may reveal local and public addresses.
    Default,
    /// Only the address used for normal traffic is exposed.
    #[default]
    PublicInterfaceOnly,
    /// WebRTC never bypasses a proxy/VPN; may break calls on some networks.
    DisableNonProxiedUdp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "provider", content = "template")]
pub enum DohProvider {
    Off,
    #[default]
    Cloudflare,
    Quad9,
    Google,
    Custom(String),
}

impl DohProvider {
    pub fn template(&self) -> Option<&str> {
        match self {
            DohProvider::Off => None,
            DohProvider::Cloudflare => Some("https://cloudflare-dns.com/dns-query"),
            DohProvider::Quad9 => Some("https://dns.quad9.net/dns-query"),
            DohProvider::Google => Some("https://dns.google/dns-query{?dns}"),
            DohProvider::Custom(template) => Some(template.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacySettings {
    pub shields_enabled: bool,
    pub https_only: bool,
    pub block_third_party_cookies: bool,
    pub strip_tracking_params: bool,
    pub fingerprint_guard: bool,
    pub do_not_track: bool,
    pub webrtc: WebRtcPolicy,
    pub doh: DohProvider,
    pub safe_browsing: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            shields_enabled: true,
            https_only: true,
            block_third_party_cookies: true,
            strip_tracking_params: true,
            fingerprint_guard: true,
            do_not_track: true,
            webrtc: WebRtcPolicy::default(),
            doh: DohProvider::default(),
            safe_browsing: true,
        }
    }
}

impl PrivacySettings {
    /// Chromium profile preferences implementing these settings. Applied
    /// when the browser starts.
    pub fn profile_preferences(&self) -> Vec<(&'static str, serde_json::Value)> {
        vec![
            // 1 = block third-party cookies, 0 = allow.
            ("profile.cookie_controls_mode", (self.block_third_party_cookies as i32).into()),
            ("profile.block_third_party_cookies", self.block_third_party_cookies.into()),
            ("enable_do_not_track", self.do_not_track.into()),
            ("https_only_mode_enabled", self.https_only.into()),
            ("safebrowsing.enabled", self.safe_browsing.into()),
            // Never offer to send page content or URLs to Google services.
            ("search.suggest_enabled", false.into()),
            ("url_keyed_anonymized_data_collection.enabled", false.into()),
        ]
    }

    /// Chromium browser-wide ("local state") preferences.
    pub fn global_preferences(&self) -> Vec<(&'static str, serde_json::Value)> {
        match self.doh.template() {
            Some(template) => vec![
                ("dns_over_https.mode", "secure".into()),
                ("dns_over_https.templates", template.into()),
            ],
            None => vec![("dns_over_https.mode", "off".into())],
        }
    }

    /// Rewrites a navigation URL according to the settings. Returns `None`
    /// when the URL is fine as it is.
    pub fn rewrite_navigation(&self, url: &Url) -> Option<Url> {
        let mut current = url.clone();
        let mut changed = false;
        if self.https_only {
            if let Some(upgraded) = upgrade_to_https(&current) {
                current = upgraded;
                changed = true;
            }
        }
        if self.strip_tracking_params {
            if let Some(stripped) = strip_tracking_params(&current) {
                current = stripped;
                changed = true;
            }
        }
        changed.then_some(current)
    }

    /// Script injected at document start into every frame.
    pub fn document_start_script(&self, session_seed: u64) -> String {
        let mut script = String::new();
        if self.do_not_track {
            script.push_str(GPC_SCRIPT);
        }
        if self.fingerprint_guard {
            script.push_str(&fingerprint_guard_script(session_seed));
        }
        script
    }
}

/// `http://` to `https://`, except for local and private destinations where
/// HTTPS usually is not available.
pub fn upgrade_to_https(url: &Url) -> Option<Url> {
    if url.scheme() != "http" {
        return None;
    }
    let local = match url.host()? {
        Host::Ipv4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified(),
        Host::Ipv6(ip) => ip.is_loopback() || ip.is_unspecified() || (ip.segments()[0] & 0xfe00) == 0xfc00,
        Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            domain == "localhost"
                || !domain.contains('.')
                || [".localhost", ".local", ".lan", ".internal", ".home.arpa", ".test"]
                    .iter()
                    .any(|suffix| domain.ends_with(suffix))
        }
    };
    if local {
        return None;
    }
    let mut upgraded = url.clone();
    upgraded.set_scheme("https").ok()?;
    if url.port() == Some(80) {
        upgraded.set_port(None).ok()?;
    }
    Some(upgraded)
}

const TRACKING_PARAMS: &[&str] = &[
    "fbclid", "gclid", "gclsrc", "dclid", "gbraid", "wbraid", "msclkid", "mc_eid", "mc_cid", "igshid",
    "yclid", "twclid", "ttclid", "li_fat_id", "_hsenc", "_hsmi", "mkt_tok", "oly_anon_id", "oly_enc_id",
    "vero_id", "s_cid", "_openstat",
];

/// Removes click-tracking query parameters (`utm_*`, `fbclid`, `gclid`, ...).
pub fn strip_tracking_params(url: &Url) -> Option<Url> {
    url.query()?;
    let is_tracking = |key: &str| {
        let key = key.to_ascii_lowercase();
        key.starts_with("utm_") || TRACKING_PARAMS.contains(&key.as_str())
    };
    if !url.query_pairs().any(|(key, _)| is_tracking(&key)) {
        return None;
    }
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| !is_tracking(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let mut stripped = url.clone();
    if kept.is_empty() {
        stripped.set_query(None);
    } else {
        stripped.query_pairs_mut().clear().extend_pairs(kept);
    }
    Some(stripped)
}

/// Global Privacy Control signal.
const GPC_SCRIPT: &str = "(function(){try{Object.defineProperty(Navigator.prototype,'globalPrivacyControl',\
{get:function(){return true},configurable:true});}catch(e){}})();";

/// Adds small, per-session noise to the APIs most used for fingerprinting
/// (canvas read-back, WebGL vendor strings, audio buffers) and coarsens
/// hardware details. The noise is stable within a session so sites do not
/// break, and differs between sessions so the fingerprint does not persist.
pub fn fingerprint_guard_script(session_seed: u64) -> String {
    format!(
        r#"(function(){{
'use strict';
if (window.__aetherGuard) return; 
try {{ Object.defineProperty(window, '__aetherGuard', {{ value: true }}); }} catch (e) {{ return; }}
var seed = {seed} >>> 0;
function rand(i) {{ var x = (seed ^ (i * 2654435761)) >>> 0; x ^= x << 13; x >>>= 0; x ^= x >>> 17; x ^= x << 5; return (x >>> 0) / 4294967296; }}
function wrap(proto, name, make) {{
  try {{ var original = proto[name]; if (typeof original !== 'function') return;
    var replacement = make(original);
    Object.defineProperty(replacement, 'name', {{ value: name }});
    replacement.toString = function () {{ return 'function ' + name + '() {{ [native code] }}'; }};
    Object.defineProperty(proto, name, {{ value: replacement, configurable: true, writable: true }});
  }} catch (e) {{}}
}}
function noisy(canvas) {{
  try {{
    var w = canvas.width, h = canvas.height; if (!w || !h || w * h > 4000000) return canvas;
    var copy = document.createElement('canvas'); copy.width = w; copy.height = h;
    var ctx = copy.getContext('2d'); ctx.drawImage(canvas, 0, 0);
    var img = ctx.getImageData(0, 0, w, h), d = img.data;
    for (var i = 0; i < 24; i++) {{ var p = Math.floor(rand(i) * (d.length / 4)) * 4; d[p] = d[p] ^ 1; }}
    ctx.putImageData(img, 0, 0); return copy;
  }} catch (e) {{ return canvas; }}
}}
if (window.HTMLCanvasElement) {{
  wrap(HTMLCanvasElement.prototype, 'toDataURL', function (o) {{ return function () {{ return o.apply(noisy(this), arguments); }}; }});
  wrap(HTMLCanvasElement.prototype, 'toBlob', function (o) {{ return function () {{ return o.apply(noisy(this), arguments); }}; }});
}}
if (window.CanvasRenderingContext2D) {{
  wrap(CanvasRenderingContext2D.prototype, 'getImageData', function (o) {{ return function () {{
    var img = o.apply(this, arguments);
    try {{ for (var i = 0; i < 8; i++) {{ var p = Math.floor(rand(i + 100) * (img.data.length / 4)) * 4; img.data[p] = img.data[p] ^ 1; }} }} catch (e) {{}}
    return img; }}; }});
}}
[window.WebGLRenderingContext, window.WebGL2RenderingContext].forEach(function (GL) {{
  if (!GL) return;
  wrap(GL.prototype, 'getParameter', function (o) {{ return function (p) {{
    if (p === 37445) return 'Google Inc.';
    if (p === 37446) return 'ANGLE (Generic GPU)';
    return o.apply(this, arguments); }}; }});
}});
if (window.AudioBuffer) {{
  wrap(AudioBuffer.prototype, 'getChannelData', function (o) {{ return function () {{
    var data = o.apply(this, arguments);
    try {{ if (!data.__aether) {{ for (var i = 0; i < data.length; i += 997) data[i] += (rand(i) - 0.5) * 1e-7;
      Object.defineProperty(data, '__aether', {{ value: true }}); }} }} catch (e) {{}}
    return data; }}; }});
}}
try {{
  Object.defineProperty(Navigator.prototype, 'hardwareConcurrency', {{ get: function () {{ return 8; }}, configurable: true }});
  if ('deviceMemory' in navigator) Object.defineProperty(Navigator.prototype, 'deviceMemory', {{ get: function () {{ return 8; }}, configurable: true }});
}} catch (e) {{}}
}})();"#,
        seed = session_seed as u32
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn upgrades_public_http_only() {
        assert_eq!(upgrade_to_https(&url("http://example.com/a?b=1")).unwrap().as_str(), "https://example.com/a?b=1");
        assert_eq!(upgrade_to_https(&url("http://example.com:80/")).unwrap().as_str(), "https://example.com/");
        assert_eq!(upgrade_to_https(&url("http://example.com:8080/")).unwrap().as_str(), "https://example.com:8080/");
        for local in [
            "http://localhost:3000/", "http://127.0.0.1/", "http://192.168.1.1/", "http://10.0.0.5/",
            "http://router/", "http://nas.local/", "http://[::1]/", "http://printer.lan/",
        ] {
            assert!(upgrade_to_https(&url(local)).is_none(), "{local} must stay http");
        }
        assert!(upgrade_to_https(&url("https://example.com/")).is_none());
    }

    #[test]
    fn strips_tracking_parameters() {
        assert_eq!(
            strip_tracking_params(&url("https://shop.example/item?id=7&utm_source=x&fbclid=abc&UTM_Medium=y")).unwrap().as_str(),
            "https://shop.example/item?id=7"
        );
        assert_eq!(
            strip_tracking_params(&url("https://shop.example/?gclid=1#top")).unwrap().as_str(),
            "https://shop.example/#top"
        );
        assert!(strip_tracking_params(&url("https://shop.example/item?id=7")).is_none());
        assert!(strip_tracking_params(&url("https://shop.example/")).is_none());
    }

    #[test]
    fn rewrite_combines_and_respects_switches() {
        let settings = PrivacySettings::default();
        assert_eq!(
            settings.rewrite_navigation(&url("http://example.com/?utm_campaign=a&q=1")).unwrap().as_str(),
            "https://example.com/?q=1"
        );
        assert!(settings.rewrite_navigation(&url("https://example.com/?q=1")).is_none());

        let off = PrivacySettings { https_only: false, strip_tracking_params: false, ..Default::default() };
        assert!(off.rewrite_navigation(&url("http://example.com/?utm_campaign=a")).is_none());
    }

    #[test]
    fn preferences_follow_settings() {
        let on = PrivacySettings::default();
        assert!(on.profile_preferences().contains(&("profile.cookie_controls_mode", 1.into())));
        assert!(on.global_preferences().contains(&("dns_over_https.mode", "secure".into())));

        let off = PrivacySettings { block_third_party_cookies: false, doh: DohProvider::Off, ..Default::default() };
        assert!(off.profile_preferences().contains(&("profile.cookie_controls_mode", 0.into())));
        assert_eq!(off.global_preferences(), vec![("dns_over_https.mode", "off".into())]);
    }

    #[test]
    fn document_start_script_follows_settings() {
        let all = PrivacySettings::default().document_start_script(42);
        assert!(all.contains("globalPrivacyControl") && all.contains("__aetherGuard"));
        let none = PrivacySettings { do_not_track: false, fingerprint_guard: false, ..Default::default() };
        assert!(none.document_start_script(42).is_empty());
        // Settings survive a JSON round trip, including missing new fields.
        let parsed: PrivacySettings = serde_json::from_str(r#"{"https_only":false}"#).unwrap();
        assert!(!parsed.https_only && parsed.fingerprint_guard);
    }
}
