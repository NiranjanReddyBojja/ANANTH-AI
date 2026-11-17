//! Resource governor: RAM/CPU/network limits, game detection and the
//! in-game overlay hotkey.

pub mod game_detect;
pub mod limiter;
pub mod overlay;

pub use game_detect::{GameDetector, GameEvent};
pub use limiter::{Governor, ResourceLimits, Telemetry};
