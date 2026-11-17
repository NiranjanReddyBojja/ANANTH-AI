//! SQLite storage for history, bookmarks, tab sessions, hibernated tabs,
//! downloads and settings, plus an importer for Chrome profiles.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{now_ms, Error, Result};

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS history (
    id          INTEGER PRIMARY KEY,
    url         TEXT NOT NULL UNIQUE,
    title       TEXT NOT NULL DEFAULT '',
    visit_count INTEGER NOT NULL DEFAULT 1,
    last_visit  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS history_last_visit ON history(last_visit DESC);

CREATE TABLE IF NOT EXISTS bookmarks (
    id         INTEGER PRIMARY KEY,
    url        TEXT NOT NULL,
    title      TEXT NOT NULL DEFAULT '',
    folder     TEXT NOT NULL DEFAULT '',
    position   INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    UNIQUE(url, folder)
);

CREATE TABLE IF NOT EXISTS session_tabs (
    position   INTEGER PRIMARY KEY,
    url        TEXT NOT NULL,
    title      TEXT NOT NULL DEFAULT '',
    pinned     INTEGER NOT NULL DEFAULT 0,
    group_name TEXT,
    active     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS hibernated_tabs (
    tab_id        TEXT PRIMARY KEY,
    url           TEXT NOT NULL,
    title         TEXT NOT NULL DEFAULT '',
    scroll_y      REAL NOT NULL DEFAULT 0,
    screenshot    BLOB,
    hibernated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS downloads (
    id         TEXT PRIMARY KEY,
    url        TEXT NOT NULL,
    path       TEXT NOT NULL,
    state      TEXT NOT NULL,
    started_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS passwords (
    id         INTEGER PRIMARY KEY,
    origin     TEXT NOT NULL,
    username   TEXT NOT NULL,
    nonce      BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(origin, username)
);
"#;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub visit_count: i64,
    pub last_visit: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub folder: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTab {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub group: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HibernatedTab {
    pub tab_id: String,
    pub url: String,
    pub title: String,
    pub scroll_y: f64,
    /// PNG/JPEG bytes of the last frame, shown while the tab reloads.
    #[serde(skip)]
    pub screenshot: Option<Vec<u8>>,
    pub hibernated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub id: String,
    pub url: String,
    pub path: String,
    pub state: String,
    pub started_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportReport {
    pub history: usize,
    pub bookmarks: usize,
}

/// Thread-safe handle to the browser database.
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub(crate) fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut conn = self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut conn)
    }

    // ------------------------------------------------------------ history

    pub fn record_visit(&self, url: &str, title: &str) -> Result<()> {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Ok(());
        }
        self.with(|conn| {
            conn.execute(
                "INSERT INTO history (url, title, last_visit) VALUES (?1, ?2, ?3)
                 ON CONFLICT(url) DO UPDATE SET
                     visit_count = visit_count + 1,
                     last_visit = excluded.last_visit,
                     title = CASE WHEN excluded.title = '' THEN title ELSE excluded.title END",
                params![url, title, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn set_history_title(&self, url: &str, title: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute("UPDATE history SET title = ?2 WHERE url = ?1", params![url, title])?;
            Ok(())
        })
    }

    /// Most recent entries, optionally filtered by a substring of URL or title.
    pub fn history(&self, query: &str, limit: usize) -> Result<Vec<HistoryEntry>> {
        let pattern = format!("%{}%", escape_like(query));
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, url, title, visit_count, last_visit FROM history
                 WHERE ?1 = '%%' OR url LIKE ?1 ESCAPE '\\' OR title LIKE ?1 ESCAPE '\\'
                 ORDER BY last_visit DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![pattern, limit as i64], history_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Omnibox suggestions: prefix matches first, then by frequency and recency.
    pub fn suggest(&self, typed: &str, limit: usize) -> Result<Vec<HistoryEntry>> {
        let typed = typed.trim();
        if typed.is_empty() {
            return Ok(Vec::new());
        }
        let needle = escape_like(typed);
        let contains = format!("%{needle}%");
        let prefixes = [
            format!("https://{needle}%"),
            format!("https://www.{needle}%"),
            format!("http://{needle}%"),
        ];
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, url, title, visit_count, last_visit,
                        (url LIKE ?2 ESCAPE '\\' OR url LIKE ?3 ESCAPE '\\' OR url LIKE ?4 ESCAPE '\\') AS is_prefix
                 FROM history
                 WHERE url LIKE ?1 ESCAPE '\\' OR title LIKE ?1 ESCAPE '\\'
                 ORDER BY is_prefix DESC, visit_count DESC, last_visit DESC LIMIT ?5",
            )?;
            let rows = stmt.query_map(
                params![contains, prefixes[0], prefixes[1], prefixes[2], limit as i64],
                history_row,
            )?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn delete_history_entry(&self, id: i64) -> Result<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM history WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    pub fn clear_history(&self) -> Result<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM history", [])?;
            Ok(())
        })
    }

    // ---------------------------------------------------------- bookmarks

    pub fn add_bookmark(&self, url: &str, title: &str, folder: &str) -> Result<i64> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO bookmarks (url, title, folder, position, created_at)
                 VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(position), 0) + 1 FROM bookmarks), ?4)
                 ON CONFLICT(url, folder) DO UPDATE SET title = excluded.title",
                params![url, title, folder, now_ms()],
            )?;
            Ok(conn.query_row(
                "SELECT id FROM bookmarks WHERE url = ?1 AND folder = ?2",
                params![url, folder],
                |row| row.get(0),
            )?)
        })
    }

    pub fn remove_bookmark_by_url(&self, url: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM bookmarks WHERE url = ?1", params![url])?;
            Ok(())
        })
    }

    pub fn remove_bookmark(&self, id: i64) -> Result<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM bookmarks WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    pub fn bookmarks(&self) -> Result<Vec<Bookmark>> {
        self.with(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, url, title, folder FROM bookmarks ORDER BY folder, position")?;
            let rows = stmt.query_map([], |row| {
                Ok(Bookmark { id: row.get(0)?, url: row.get(1)?, title: row.get(2)?, folder: row.get(3)? })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn is_bookmarked(&self, url: &str) -> Result<bool> {
        self.with(|conn| {
            Ok(conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM bookmarks WHERE url = ?1)",
                params![url],
                |row| row.get(0),
            )?)
        })
    }

    // ------------------------------------------------------------ session

    pub fn save_session(&self, tabs: &[SessionTab]) -> Result<()> {
        self.with(|conn| {
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM session_tabs", [])?;
            for (position, tab) in tabs.iter().enumerate() {
                tx.execute(
                    "INSERT INTO session_tabs (position, url, title, pinned, group_name, active)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![position as i64, tab.url, tab.title, tab.pinned, tab.group, tab.active],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn load_session(&self) -> Result<Vec<SessionTab>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT url, title, pinned, group_name, active FROM session_tabs ORDER BY position",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(SessionTab {
                    url: row.get(0)?,
                    title: row.get(1)?,
                    pinned: row.get(2)?,
                    group: row.get(3)?,
                    active: row.get(4)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    // -------------------------------------------------------- hibernation

    pub fn save_hibernated(&self, tab: &HibernatedTab) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO hibernated_tabs
                     (tab_id, url, title, scroll_y, screenshot, hibernated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![tab.tab_id, tab.url, tab.title, tab.scroll_y, tab.screenshot, tab.hibernated_at],
            )?;
            Ok(())
        })
    }

    /// Removes and returns the stored state of a hibernated tab.
    pub fn take_hibernated(&self, tab_id: &str) -> Result<Option<HibernatedTab>> {
        self.with(|conn| {
            let tab = conn
                .query_row(
                    "SELECT tab_id, url, title, scroll_y, screenshot, hibernated_at
                     FROM hibernated_tabs WHERE tab_id = ?1",
                    params![tab_id],
                    |row| {
                        Ok(HibernatedTab {
                            tab_id: row.get(0)?,
                            url: row.get(1)?,
                            title: row.get(2)?,
                            scroll_y: row.get(3)?,
                            screenshot: row.get(4)?,
                            hibernated_at: row.get(5)?,
                        })
                    },
                )
                .optional()?;
            conn.execute("DELETE FROM hibernated_tabs WHERE tab_id = ?1", params![tab_id])?;
            Ok(tab)
        })
    }

    pub fn hibernated_screenshot(&self, tab_id: &str) -> Result<Option<Vec<u8>>> {
        self.with(|conn| {
            Ok(conn
                .query_row(
                    "SELECT screenshot FROM hibernated_tabs WHERE tab_id = ?1",
                    params![tab_id],
                    |row| row.get::<_, Option<Vec<u8>>>(0),
                )
                .optional()?
                .flatten())
        })
    }

    pub fn clear_hibernated(&self) -> Result<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM hibernated_tabs", [])?;
            Ok(())
        })
    }

    // ---------------------------------------------------------- downloads

    pub fn upsert_download(&self, record: &DownloadRecord) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO downloads (id, url, path, state, started_at) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET state = excluded.state, path = excluded.path",
                params![record.id, record.url, record.path, record.state, record.started_at],
            )?;
            Ok(())
        })
    }

    pub fn downloads(&self, limit: usize) -> Result<Vec<DownloadRecord>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, url, path, state, started_at FROM downloads
                 ORDER BY started_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], |row| {
                Ok(DownloadRecord {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    path: row.get(2)?,
                    state: row.get(3)?,
                    started_at: row.get(4)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn clear_downloads(&self) -> Result<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM downloads", [])?;
            Ok(())
        })
    }

    // ----------------------------------------------------------- settings

    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let raw: Option<String> = self.with(|conn| {
            Ok(conn
                .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |row| row.get(0))
                .optional()?)
        })?;
        Ok(raw.and_then(|value| serde_json::from_str(&value).ok()))
    }

    pub fn setting_or_default<T: DeserializeOwned + Default>(&self, key: &str) -> T {
        self.setting(key).ok().flatten().unwrap_or_default()
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let raw = serde_json::to_string(value).map_err(|error| Error::Invalid(error.to_string()))?;
        self.with(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, raw],
            )?;
            Ok(())
        })
    }

    // ------------------------------------------------------ Chrome import

    /// Imports history and bookmarks from a Chrome/Chromium/Edge/Brave
    /// profile directory (the folder containing `History` and `Bookmarks`).
    pub fn import_chrome_profile(&self, profile_dir: impl AsRef<Path>) -> Result<ImportReport> {
        let profile_dir = profile_dir.as_ref();
        let mut report = ImportReport::default();

        let history_file = profile_dir.join("History");
        if history_file.is_file() {
            // Chrome keeps the file locked while running, so read a copy.
            let copy = std::env::temp_dir().join(format!("aether-import-{}.sqlite", std::process::id()));
            std::fs::copy(&history_file, &copy)?;
            let imported = self.import_chrome_history(&copy);
            let _ = std::fs::remove_file(&copy);
            report.history = imported?;
        }

        let bookmarks_file = profile_dir.join("Bookmarks");
        if bookmarks_file.is_file() {
            let json: serde_json::Value = serde_json::from_slice(&std::fs::read(bookmarks_file)?)
                .map_err(|error| Error::Invalid(format!("Bookmarks file: {error}")))?;
            let mut found = Vec::new();
            if let Some(roots) = json.get("roots").and_then(|roots| roots.as_object()) {
                for root in roots.values() {
                    collect_chrome_bookmarks(root, "", &mut found);
                }
            }
            for (url, title, folder) in &found {
                self.add_bookmark(url, title, folder)?;
            }
            report.bookmarks = found.len();
        }
        Ok(report)
    }

    fn import_chrome_history(&self, chrome_db: &Path) -> Result<usize> {
        let source = Connection::open_with_flags(chrome_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut stmt = source.prepare(
            "SELECT url, title, visit_count, last_visit_time FROM urls
             WHERE hidden = 0 AND (url LIKE 'http://%' OR url LIKE 'https://%')",
        )?;
        let rows: Vec<(String, String, i64, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        self.with(|conn| {
            let tx = conn.transaction()?;
            for (url, title, visits, chrome_time) in &rows {
                tx.execute(
                    "INSERT INTO history (url, title, visit_count, last_visit) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(url) DO UPDATE SET
                         visit_count = visit_count + excluded.visit_count,
                         last_visit = MAX(last_visit, excluded.last_visit)",
                    params![url, title, (*visits).max(1), chrome_time_to_unix_ms(*chrome_time)],
                )?;
            }
            tx.commit()?;
            Ok(rows.len())
        })
    }
}

fn history_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: row.get(0)?,
        url: row.get(1)?,
        title: row.get(2)?,
        visit_count: row.get(3)?,
        last_visit: row.get(4)?,
    })
}

fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Chrome stores times as microseconds since 1601-01-01 UTC.
pub fn chrome_time_to_unix_ms(chrome_time: i64) -> i64 {
    const EPOCH_DELTA_MICROS: i64 = 11_644_473_600_000_000;
    (chrome_time - EPOCH_DELTA_MICROS) / 1000
}

fn collect_chrome_bookmarks(node: &serde_json::Value, folder: &str, out: &mut Vec<(String, String, String)>) {
    let name = node.get("name").and_then(|name| name.as_str()).unwrap_or("");
    match node.get("type").and_then(|kind| kind.as_str()) {
        Some("url") => {
            if let Some(url) = node.get("url").and_then(|url| url.as_str()) {
                if url.starts_with("http://") || url.starts_with("https://") {
                    out.push((url.to_string(), name.to_string(), folder.to_string()));
                }
            }
        }
        Some("folder") => {
            let path = if folder.is_empty() { name.to_string() } else { format!("{folder}/{name}") };
            for child in node.get("children").and_then(|c| c.as_array()).into_iter().flatten() {
                collect_chrome_bookmarks(child, &path, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_counts_visits_and_suggests_prefix_first() {
        let db = Database::open_in_memory().unwrap();
        db.record_visit("https://store.steampowered.com/", "Steam").unwrap();
        db.record_visit("https://store.steampowered.com/", "").unwrap();
        db.record_visit("https://example.com/about-steam", "About").unwrap();
        db.record_visit("about:blank", "ignored").unwrap();

        let all = db.history("", 10).unwrap();
        assert_eq!(all.len(), 2);
        let steam = all.iter().find(|entry| entry.url.contains("steampowered")).unwrap();
        assert_eq!(steam.visit_count, 2);
        assert_eq!(steam.title, "Steam", "an empty title must not overwrite a known one");

        let suggestions = db.suggest("store", 5).unwrap();
        assert_eq!(suggestions[0].url, "https://store.steampowered.com/");
        assert_eq!(db.suggest("steam", 5).unwrap().len(), 2);
        assert!(db.suggest("100%", 5).unwrap().is_empty(), "LIKE wildcards are escaped");
    }

    #[test]
    fn bookmarks_sessions_settings_roundtrip() {
        let db = Database::open_in_memory().unwrap();
        db.add_bookmark("https://a.example/", "A", "").unwrap();
        db.add_bookmark("https://a.example/", "A renamed", "").unwrap();
        assert_eq!(db.bookmarks().unwrap().len(), 1);
        assert_eq!(db.bookmarks().unwrap()[0].title, "A renamed");
        assert!(db.is_bookmarked("https://a.example/").unwrap());
        db.remove_bookmark_by_url("https://a.example/").unwrap();
        assert!(!db.is_bookmarked("https://a.example/").unwrap());

        let tabs = vec![
            SessionTab { url: "https://a/".into(), title: "A".into(), pinned: true, group: Some("Work".into()), active: false },
            SessionTab { url: "https://b/".into(), title: "B".into(), pinned: false, group: None, active: true },
        ];
        db.save_session(&tabs).unwrap();
        db.save_session(&tabs).unwrap();
        assert_eq!(db.load_session().unwrap(), tabs);

        db.set_setting("limits", &vec![1, 2, 3]).unwrap();
        assert_eq!(db.setting::<Vec<i32>>("limits").unwrap(), Some(vec![1, 2, 3]));
        assert_eq!(db.setting::<Vec<i32>>("missing").unwrap(), None);
    }

    #[test]
    fn hibernated_tab_is_taken_once() {
        let db = Database::open_in_memory().unwrap();
        let tab = HibernatedTab {
            tab_id: "tab-7".into(),
            url: "https://wiki.example/boss".into(),
            title: "Boss guide".into(),
            scroll_y: 1234.5,
            screenshot: Some(vec![1, 2, 3]),
            hibernated_at: 42,
        };
        db.save_hibernated(&tab).unwrap();
        assert_eq!(db.hibernated_screenshot("tab-7").unwrap(), Some(vec![1, 2, 3]));
        assert_eq!(db.take_hibernated("tab-7").unwrap(), Some(tab));
        assert_eq!(db.take_hibernated("tab-7").unwrap(), None);
    }

    #[test]
    fn imports_chrome_profile() {
        let dir = tempfile::tempdir().unwrap();
        let chrome = Connection::open(dir.path().join("History")).unwrap();
        chrome
            .execute_batch(
                "CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT, title TEXT,
                     visit_count INTEGER, typed_count INTEGER, last_visit_time INTEGER, hidden INTEGER);
                 INSERT INTO urls VALUES (1, 'https://www.twitch.tv/', 'Twitch', 9, 1, 13350000000000000, 0);
                 INSERT INTO urls VALUES (2, 'chrome://settings/', 'Settings', 3, 0, 13350000000000000, 0);
                 INSERT INTO urls VALUES (3, 'https://hidden.example/', 'Hidden', 1, 0, 13350000000000000, 1);",
            )
            .unwrap();
        drop(chrome);
        std::fs::write(
            dir.path().join("Bookmarks"),
            r#"{"roots":{"bookmark_bar":{"type":"folder","name":"Bookmarks bar","children":[
                {"type":"url","name":"SteamDB","url":"https://steamdb.info/"},
                {"type":"folder","name":"Guides","children":[
                    {"type":"url","name":"Wiki","url":"https://wiki.example/"},
                    {"type":"url","name":"Script","url":"javascript:alert(1)"}]}]},
              "other":{"type":"folder","name":"Other","children":[]}}}"#,
        )
        .unwrap();

        let db = Database::open_in_memory().unwrap();
        let report = db.import_chrome_profile(dir.path()).unwrap();
        assert_eq!(report, ImportReport { history: 1, bookmarks: 2 });

        let history = db.history("", 10).unwrap();
        assert_eq!(history[0].visit_count, 9);
        // 13350000000000000 µs after 1601 is in January 2024.
        assert_eq!(history[0].last_visit, 1_705_526_400_000);
        let bookmarks = db.bookmarks().unwrap();
        assert!(bookmarks.iter().any(|b| b.url == "https://wiki.example/" && b.folder == "Bookmarks bar/Guides"));
    }
}
