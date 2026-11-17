//! Encrypted password manager.
//!
//! Every password is sealed with AES-256-GCM under a 256-bit vault key. The
//! key is either a random key kept in the operating system keyring (Windows
//! Credential Manager, macOS Keychain, Secret Service) or derived from a
//! master password with Argon2id. The site and user name are bound to the
//! ciphertext as associated data, so a row cannot be moved to another site.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;
use rand::RngCore;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use zeroize::Zeroizing;

use super::Database;
use crate::{now_ms, Error, Result};

const KEYRING_SERVICE: &str = "Aether Browser";
const KEYRING_ACCOUNT: &str = "vault-key";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
/// Setting holding the Argon2 salt when a master password is used.
const SALT_SETTING: &str = "vault.salt";
/// Setting holding an encrypted marker used to verify the master password.
const CHECK_SETTING: &str = "vault.check";
const CHECK_PLAINTEXT: &[u8] = b"aether-vault-v1";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Login {
    pub id: i64,
    pub origin: String,
    pub username: String,
    pub updated_at: i64,
}

/// An unlocked vault. The key is wiped from memory when dropped.
pub struct Vault {
    key: Zeroizing<[u8; KEY_LEN]>,
}

impl Vault {
    pub fn with_key(key: [u8; KEY_LEN]) -> Self {
        Self { key: Zeroizing::new(key) }
    }

    /// Unlocks with a random key stored in the OS keyring, creating it on first use.
    pub fn from_os_keyring() -> Result<Self> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
            .map_err(|error| Error::Vault(format!("keyring unavailable: {error}")))?;
        let encoder = base64::engine::general_purpose::STANDARD;
        match entry.get_password() {
            Ok(stored) => {
                let bytes = Zeroizing::new(
                    encoder.decode(stored.trim()).map_err(|_| Error::Vault("corrupt keyring entry".into()))?,
                );
                let key: [u8; KEY_LEN] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| Error::Vault("corrupt keyring entry".into()))?;
                Ok(Self::with_key(key))
            }
            Err(keyring::Error::NoEntry) => {
                let mut key = [0u8; KEY_LEN];
                rand::rngs::OsRng.fill_bytes(&mut key);
                entry
                    .set_password(&encoder.encode(key))
                    .map_err(|error| Error::Vault(format!("cannot store vault key: {error}")))?;
                Ok(Self::with_key(key))
            }
            Err(error) => Err(Error::Vault(format!("keyring unavailable: {error}"))),
        }
    }

    /// Unlocks with a master password (Argon2id, 64 MiB, 3 passes). The first
    /// call sets the password; later calls fail if it does not match.
    pub fn from_master_password(db: &Database, password: &str) -> Result<Self> {
        let encoder = base64::engine::general_purpose::STANDARD;
        let salt = match db.setting::<String>(SALT_SETTING)? {
            Some(stored) => encoder.decode(stored).map_err(|_| Error::Vault("corrupt salt".into()))?,
            None => {
                let mut salt = vec![0u8; 16];
                rand::rngs::OsRng.fill_bytes(&mut salt);
                db.set_setting(SALT_SETTING, &encoder.encode(&salt))?;
                salt
            }
        };
        let vault = Self::with_key(derive_key(password, &salt)?);

        match db.setting::<String>(CHECK_SETTING)? {
            Some(check) => {
                let sealed = encoder.decode(check).map_err(|_| Error::Vault("corrupt check".into()))?;
                let (nonce, ciphertext) = sealed.split_at(NONCE_LEN.min(sealed.len()));
                vault
                    .open(nonce, ciphertext, b"check")
                    .map_err(|_| Error::Vault("wrong master password".into()))?;
            }
            None => {
                let (nonce, ciphertext) = vault.seal(CHECK_PLAINTEXT, b"check")?;
                db.set_setting(CHECK_SETTING, &encoder.encode([nonce.as_slice(), &ciphertext].concat()))?;
            }
        }
        Ok(vault)
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(self.key.as_slice()).expect("key is 32 bytes")
    }

    fn seal(&self, plaintext: &[u8], aad: &[u8]) -> Result<([u8; NONCE_LEN], Vec<u8>)> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher()
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad })
            .map_err(|_| Error::Vault("encryption failed".into()))?;
        Ok((nonce, ciphertext))
    }

    fn open(&self, nonce: &[u8], ciphertext: &[u8], aad: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        if nonce.len() != NONCE_LEN {
            return Err(Error::Vault("corrupt entry".into()));
        }
        self.cipher()
            .decrypt(Nonce::from_slice(nonce), Payload { msg: ciphertext, aad })
            .map(Zeroizing::new)
            .map_err(|_| Error::Vault("decryption failed (wrong key or tampered data)".into()))
    }

    /// Saves or replaces the password for a site and user name.
    pub fn save(&self, db: &Database, origin: &str, username: &str, password: &str) -> Result<()> {
        let (nonce, ciphertext) = self.seal(password.as_bytes(), &associated_data(origin, username))?;
        db.with(|conn| {
            conn.execute(
                "INSERT INTO passwords (origin, username, nonce, ciphertext, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(origin, username) DO UPDATE SET
                     nonce = excluded.nonce, ciphertext = excluded.ciphertext, updated_at = excluded.updated_at",
                params![origin, username, nonce.as_slice(), ciphertext, now_ms()],
            )?;
            Ok(())
        })
    }

    /// Decrypts one saved password.
    pub fn reveal(&self, db: &Database, id: i64) -> Result<Zeroizing<String>> {
        let row: Option<(String, String, Vec<u8>, Vec<u8>)> = db.with(|conn| {
            Ok(conn
                .query_row(
                    "SELECT origin, username, nonce, ciphertext FROM passwords WHERE id = ?1",
                    params![id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?)
        })?;
        let (origin, username, nonce, ciphertext) = row.ok_or_else(|| Error::Vault("no such login".into()))?;
        let plaintext = self.open(&nonce, &ciphertext, &associated_data(&origin, &username))?;
        String::from_utf8(plaintext.to_vec())
            .map(Zeroizing::new)
            .map_err(|_| Error::Vault("corrupt entry".into()))
    }
}

/// Lists saved logins without decrypting anything.
pub fn list_logins(db: &Database, origin: Option<&str>) -> Result<Vec<Login>> {
    db.with(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, origin, username, updated_at FROM passwords
             WHERE ?1 IS NULL OR origin = ?1 ORDER BY origin, username",
        )?;
        let rows = stmt.query_map(params![origin], |row| {
            Ok(Login { id: row.get(0)?, origin: row.get(1)?, username: row.get(2)?, updated_at: row.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
}

pub fn delete_login(db: &Database, id: i64) -> Result<()> {
    db.with(|conn| {
        conn.execute("DELETE FROM passwords WHERE id = ?1", params![id])?;
        Ok(())
    })
}

fn associated_data(origin: &str, username: &str) -> Vec<u8> {
    // Length-prefixed so ("ab", "c") and ("a", "bc") never collide.
    let mut data = Vec::with_capacity(origin.len() + username.len() + 8);
    data.extend_from_slice(&(origin.len() as u32).to_le_bytes());
    data.extend_from_slice(origin.as_bytes());
    data.extend_from_slice(&(username.len() as u32).to_le_bytes());
    data.extend_from_slice(username.as_bytes());
    data
}

fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; KEY_LEN]> {
    let params = Params::new(64 * 1024, 3, 1, Some(KEY_LEN)).map_err(|error| Error::Vault(error.to_string()))?;
    let mut key = [0u8; KEY_LEN];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|error| Error::Vault(error.to_string()))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_reveals_and_replaces() {
        let db = Database::open_in_memory().unwrap();
        let vault = Vault::with_key([7; KEY_LEN]);
        vault.save(&db, "https://store.steampowered.com", "gamer", "hunter2").unwrap();
        vault.save(&db, "https://store.steampowered.com", "gamer", "correct horse").unwrap();

        let logins = list_logins(&db, None).unwrap();
        assert_eq!(logins.len(), 1);
        assert_eq!(vault.reveal(&db, logins[0].id).unwrap().as_str(), "correct horse");

        // Nothing readable is stored.
        let stored: Vec<u8> = db
            .with(|conn| Ok(conn.query_row("SELECT ciphertext FROM passwords", [], |row| row.get(0))?))
            .unwrap();
        assert!(!stored.windows(7).any(|window| window == b"correct"));

        delete_login(&db, logins[0].id).unwrap();
        assert!(list_logins(&db, None).unwrap().is_empty());
    }

    #[test]
    fn wrong_key_and_moved_rows_fail() {
        let db = Database::open_in_memory().unwrap();
        let vault = Vault::with_key([1; KEY_LEN]);
        vault.save(&db, "https://bank.example", "me", "secret").unwrap();
        let id = list_logins(&db, None).unwrap()[0].id;

        assert!(Vault::with_key([2; KEY_LEN]).reveal(&db, id).is_err());

        // Re-pointing the row at another site breaks authentication.
        db.with(|conn| {
            conn.execute("UPDATE passwords SET origin = 'https://evil.example'", [])?;
            Ok(())
        })
        .unwrap();
        assert!(vault.reveal(&db, id).is_err());
    }

    #[test]
    fn master_password_is_verified() {
        let db = Database::open_in_memory().unwrap();
        let vault = Vault::from_master_password(&db, "open sesame").unwrap();
        vault.save(&db, "https://a.example", "u", "p").unwrap();
        let id = list_logins(&db, None).unwrap()[0].id;

        assert!(Vault::from_master_password(&db, "wrong").is_err());
        let again = Vault::from_master_password(&db, "open sesame").unwrap();
        assert_eq!(again.reveal(&db, id).unwrap().as_str(), "p");
    }
}
