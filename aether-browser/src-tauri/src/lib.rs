//! Aether browser kernel.
//!
//! Everything in this crate except [`browser::manager`] and [`commands`] is
//! independent of the UI shell, so it builds and tests on any platform.

pub mod browser;
pub mod governor;
pub mod shield;
pub mod state;
pub mod storage;

#[cfg(feature = "shell")]
pub mod commands;
#[cfg(feature = "shell")]
pub mod overlay_window;


/// Errors surfaced by the kernel.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid data: {0}")]
    Invalid(String),
    #[error("vault error: {0}")]
    Vault(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("not supported on this platform: {0}")]
    Unsupported(&'static str),
    #[error("operating system error: {0}")]
    Os(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}
