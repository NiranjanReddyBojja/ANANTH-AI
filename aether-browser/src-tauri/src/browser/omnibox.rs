//! Turns omnibox input into a destination: a URL, a search, or a command.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SearchEngine {
    #[default]
    Google,
    DuckDuckGo,
    Bing,
    Brave,
}

impl SearchEngine {
    pub fn search_url(self, query: &str) -> String {
        let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        match self {
            SearchEngine::Google => format!("https://www.google.com/search?q={encoded}"),
            SearchEngine::DuckDuckGo => format!("https://duckduckgo.com/?q={encoded}"),
            SearchEngine::Bing => format!("https://www.bing.com/search?q={encoded}"),
            SearchEngine::Brave => format!("https://search.brave.com/search?q={encoded}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Destination {
    Url(String),
    Search(String),
    /// A browser command typed as `>name`, for example `>settings`.
    Command(String),
}

fn is_host_like(text: &str) -> bool {
    let host = text.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => (name, true),
        _ => (host, false),
    };
    if name.is_empty() {
        return false;
    }
    if name.eq_ignore_ascii_case("localhost") || name.parse::<std::net::Ipv4Addr>().is_ok() {
        return true;
    }
    if name.starts_with('[') && name.ends_with(']') {
        return name[1..name.len() - 1].parse::<std::net::Ipv6Addr>().is_ok();
    }
    let valid_labels = name
        .split('.')
        .all(|label| !label.is_empty() && label.chars().all(|c| c.is_alphanumeric() || c == '-'));
    let tld_ok = name.rsplit('.').next().is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_alphabetic()));
    valid_labels && ((name.contains('.') && tld_ok) || (port && !name.contains('.')))
}

fn is_local(text: &str) -> bool {
    let host = text.split(['/', '?', '#', ':']).next().unwrap_or("");
    host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::Ipv4Addr>().is_ok() || !host.contains('.')
}

/// Decides what the user meant.
pub fn resolve(input: &str, engine: SearchEngine) -> Option<Destination> {
    let text = input.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(command) = text.strip_prefix('>') {
        return Some(Destination::Command(command.trim().to_ascii_lowercase()));
    }
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("javascript:") {
        return None;
    }
    let has_scheme = ["http://", "https://", "file://", "about:", "data:", "blob:", "view-source:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme));
    if has_scheme {
        return Some(Destination::Url(text.to_owned()));
    }
    if !text.contains(char::is_whitespace) && is_host_like(text) {
        let scheme = if is_local(text) { "http" } else { "https" };
        return Some(Destination::Url(format!("{scheme}://{text}")));
    }
    Some(Destination::Search(engine.search_url(text)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn go(input: &str) -> Option<Destination> {
        resolve(input, SearchEngine::Google)
    }

    #[test]
    fn recognises_addresses() {
        assert_eq!(go("example.com"), Some(Destination::Url("https://example.com".into())));
        assert_eq!(go("  store.steampowered.com/app/730 "), Some(Destination::Url("https://store.steampowered.com/app/730".into())));
        assert_eq!(go("localhost:3000"), Some(Destination::Url("http://localhost:3000".into())));
        assert_eq!(go("192.168.1.1/admin"), Some(Destination::Url("http://192.168.1.1/admin".into())));
        assert_eq!(go("nas:8080"), Some(Destination::Url("http://nas:8080".into())));
        assert_eq!(go("https://a.example/?q=1"), Some(Destination::Url("https://a.example/?q=1".into())));
        assert_eq!(go("about:blank"), Some(Destination::Url("about:blank".into())));
    }

    #[test]
    fn everything_else_is_a_search() {
        assert_eq!(go("best cs2 settings"), Some(Destination::Search("https://www.google.com/search?q=best+cs2+settings".into())));
        assert_eq!(go("elden"), Some(Destination::Search("https://www.google.com/search?q=elden".into())));
        assert_eq!(go("what is 3.5"), Some(Destination::Search("https://www.google.com/search?q=what+is+3.5".into())));
        assert_eq!(go("node.js"), Some(Destination::Url("https://node.js".into())), "ambiguous: treated as a host like Chrome does");
        assert_eq!(go("c++ & rust?"), Some(Destination::Search("https://www.google.com/search?q=c%2B%2B+%26+rust%3F".into())));
        assert_eq!(
            resolve("fps drops", SearchEngine::DuckDuckGo),
            Some(Destination::Search("https://duckduckgo.com/?q=fps+drops".into()))
        );
    }

    #[test]
    fn commands_and_rejections() {
        assert_eq!(go("> Settings"), Some(Destination::Command("settings".into())));
        assert_eq!(go(""), None);
        assert_eq!(go("javascript:alert(1)"), None);
    }
}
