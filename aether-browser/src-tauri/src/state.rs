//! Shared application state.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::browser::hibernation::HibernationPolicy;
use crate::governor::overlay::OverlayConfig;
use crate::governor::{Governor, ResourceLimits};
use crate::shield::{PrivacySettings, Shield};
use crate::storage::{Database, Vault};

pub const SETTING_PRIVACY: &str = "privacy";
pub const SETTING_LIMITS: &str = "limits";
pub const SETTING_HIBERNATION: &str = "hibernation";
pub const SETTING_OVERLAY: &str = "overlay";
pub const SETTING_SHIELD_ALLOWED: &str = "shield.allowed_sites";

/// Where Aether keeps its profile. Override with `AETHER_DATA_DIR`.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("AETHER_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    if cfg!(windows) {
        env("APPDATA").unwrap_or_else(|| PathBuf::from(".")).join("Aether")
    } else if cfg!(target_os = "macos") {
        env("HOME").unwrap_or_else(|| PathBuf::from(".")).join("Library/Application Support/Aether")
    } else {
        env("XDG_DATA_HOME")
            .or_else(|| env("HOME").map(|home| home.join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("aether")
    }
}

pub struct AppState {
    pub data_dir: PathBuf,
    pub db: Arc<Database>,
    pub shield: Arc<Shield>,
    pub governor: Arc<Governor>,
    pub privacy: RwLock<PrivacySettings>,
    pub limits: RwLock<ResourceLimits>,
    pub hibernation: RwLock<HibernationPolicy>,
    pub overlay: RwLock<OverlayConfig>,
    pub vault: Mutex<Option<Vault>>,
    pub game_active: AtomicBool,
    pub game_name: Mutex<Option<String>>,
    /// Random per-launch seed for fingerprint noise.
    pub session_seed: u64,
}

impl AppState {
    /// Opens the profile and loads saved settings.
    pub fn load(data_dir: PathBuf, governor: Arc<Governor>) -> crate::Result<Self> {
        let db = Arc::new(Database::open(data_dir.join("aether.sqlite"))?);
        let privacy: PrivacySettings = db.setting_or_default(SETTING_PRIVACY);
        let shield = Arc::new(Shield::builtin());
        shield.set_enabled(privacy.shields_enabled);
        shield.set_allowed_sites(db.setting_or_default::<Vec<String>>(SETTING_SHIELD_ALLOWED));
        Ok(Self {
            limits: RwLock::new(db.setting_or_default(SETTING_LIMITS)),
            hibernation: RwLock::new(db.setting_or_default(SETTING_HIBERNATION)),
            overlay: RwLock::new(db.setting_or_default::<OverlayConfig>(SETTING_OVERLAY).clamped()),
            privacy: RwLock::new(privacy),
            vault: Mutex::new(None),
            game_active: AtomicBool::new(false),
            game_name: Mutex::new(None),
            session_seed: rand::random(),
            data_dir,
            db,
            shield,
            governor,
        })
    }

    pub fn filter_dir(&self) -> PathBuf {
        self.data_dir.join("filters")
    }

    pub fn extensions_dir(&self) -> PathBuf {
        self.data_dir.join("extensions")
    }

    pub fn is_game_active(&self) -> bool {
        self.game_active.load(Ordering::Relaxed)
    }

    pub fn privacy(&self) -> PrivacySettings {
        self.privacy.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn limits(&self) -> ResourceLimits {
        self.limits.read().unwrap_or_else(|p| p.into_inner()).clone()
    }
}
