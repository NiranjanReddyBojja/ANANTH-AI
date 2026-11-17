//! Tab management: lifecycle, hibernation and extensions.

pub mod extensions;
pub mod hibernation;
pub mod omnibox;

#[cfg(feature = "shell")]
pub mod manager;
