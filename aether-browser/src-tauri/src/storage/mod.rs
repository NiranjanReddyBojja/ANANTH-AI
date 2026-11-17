//! Persistent storage: SQLite database and the encrypted password vault.

pub mod db;
pub mod vault;

pub use db::Database;
pub use vault::Vault;
