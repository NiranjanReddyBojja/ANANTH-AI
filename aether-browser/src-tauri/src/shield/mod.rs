//! Native shield engine: ad and tracker blocking plus privacy protections.

pub mod adblocker;
pub mod privacy;

pub use adblocker::{Decision, Shield};
pub use privacy::PrivacySettings;
