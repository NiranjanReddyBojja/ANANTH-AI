//! In-game overlay: a global hotkey (Alt+G by default) toggles a small,
//! translucent, always-on-top browser window for guides, maps or a stream.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayConfig {
    pub enabled: bool,
    /// For example `Alt+G` or `Ctrl+Shift+F10`.
    pub hotkey: String,
    /// 0.2 (very faint) to 1.0 (solid).
    pub opacity: f64,
    pub width: u32,
    pub height: u32,
    /// Page shown in the overlay.
    pub url: String,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hotkey: "Alt+G".into(),
            opacity: 0.9,
            width: 480,
            height: 640,
            url: String::new(),
        }
    }
}

impl OverlayConfig {
    pub fn clamped(mut self) -> Self {
        self.opacity = if self.opacity.is_finite() { self.opacity.clamp(0.2, 1.0) } else { 0.9 };
        self.width = self.width.clamp(240, 1920);
        self.height = self.height.clamp(180, 1440);
        self
    }
}

/// A parsed hotkey: modifier flags and a Windows virtual-key code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub win: bool,
    pub virtual_key: u32,
}

/// Parses strings such as `Alt+G`, `Ctrl+Shift+F10` or `Alt+Space`.
/// At least one modifier is required so plain typing is never captured.
pub fn parse_hotkey(text: &str) -> Result<Hotkey> {
    let mut hotkey = Hotkey { alt: false, ctrl: false, shift: false, win: false, virtual_key: 0 };
    for part in text.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "alt" | "option" => hotkey.alt = true,
            "ctrl" | "control" => hotkey.ctrl = true,
            "shift" => hotkey.shift = true,
            "win" | "super" | "meta" | "cmd" => hotkey.win = true,
            key => {
                if hotkey.virtual_key != 0 {
                    return Err(Error::Invalid(format!("hotkey '{text}' has more than one key")));
                }
                hotkey.virtual_key = virtual_key(key)
                    .ok_or_else(|| Error::Invalid(format!("unknown key '{part}' in hotkey '{text}'")))?;
            }
        }
    }
    if hotkey.virtual_key == 0 {
        return Err(Error::Invalid(format!("hotkey '{text}' has no key")));
    }
    if !(hotkey.alt || hotkey.ctrl || hotkey.win) {
        return Err(Error::Invalid(format!("hotkey '{text}' needs Alt, Ctrl or Win")));
    }
    Ok(hotkey)
}

fn virtual_key(key: &str) -> Option<u32> {
    let bytes = key.as_bytes();
    match key {
        _ if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() => Some(u32::from(bytes[0].to_ascii_uppercase())),
        "space" => Some(0x20),
        "tab" => Some(0x09),
        "esc" | "escape" => Some(0x1B),
        "insert" => Some(0x2D),
        "home" => Some(0x24),
        "end" => Some(0x23),
        "pageup" => Some(0x21),
        "pagedown" => Some(0x22),
        "`" | "backquote" | "tilde" => Some(0xC0),
        _ => key.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=24).contains(n)).map(|n| 0x6F + n),
    }
}

/// A system-wide hotkey. The callback runs on a background thread each time
/// the key is pressed, even while a game has focus. Unregistered on drop.
pub struct GlobalHotkey {
    #[cfg(windows)]
    thread_id: u32,
    #[cfg(windows)]
    thread: Option<std::thread::JoinHandle<()>>,
}

impl GlobalHotkey {
    pub fn register(hotkey: Hotkey, callback: impl Fn() + Send + 'static) -> Result<Self> {
        #[cfg(windows)]
        {
            windows_impl::register(hotkey, callback)
        }
        #[cfg(not(windows))]
        {
            let _ = (hotkey, callback);
            Err(Error::Unsupported("global hotkeys are implemented for Windows only"))
        }
    }
}

#[cfg(windows)]
impl Drop for GlobalHotkey {
    fn drop(&mut self) {
        windows_impl::stop(self.thread_id);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::sync::mpsc;

    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, PostThreadMessageW, MSG, WM_HOTKEY, WM_QUIT};

    use super::{GlobalHotkey, Hotkey};
    use crate::{Error, Result};

    const HOTKEY_ID: i32 = 0xAE7;

    pub fn register(hotkey: Hotkey, callback: impl Fn() + Send + 'static) -> Result<GlobalHotkey> {
        let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<u32, String>>();
        // Hotkey messages are delivered to the registering thread, so that
        // thread owns the registration and runs the message loop.
        let thread = std::thread::Builder::new()
            .name("aether-hotkey".into())
            .spawn(move || unsafe {
                let mut modifiers = MOD_NOREPEAT;
                for (on, flag) in [(hotkey.alt, MOD_ALT), (hotkey.ctrl, MOD_CONTROL), (hotkey.shift, MOD_SHIFT), (hotkey.win, MOD_WIN)] {
                    if on {
                        modifiers |= flag;
                    }
                }
                let modifiers: HOT_KEY_MODIFIERS = modifiers;
                if let Err(error) = RegisterHotKey(None, HOTKEY_ID, modifiers, hotkey.virtual_key) {
                    let _ = ready_tx.send(Err(error.to_string()));
                    return;
                }
                let _ = ready_tx.send(Ok(GetCurrentThreadId()));
                let mut message = MSG::default();
                while GetMessageW(&mut message, None, 0, 0).as_bool() {
                    if message.message == WM_HOTKEY {
                        callback();
                    }
                }
                let _ = UnregisterHotKey(None, HOTKEY_ID);
            })
            .map_err(|error| Error::Os(error.to_string()))?;

        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(GlobalHotkey { thread_id, thread: Some(thread) }),
            Ok(Err(error)) => Err(Error::Os(format!("cannot register hotkey (already in use?): {error}"))),
            Err(_) => Err(Error::Os("hotkey thread exited".into())),
        }
    }

    pub fn stop(thread_id: u32) {
        // SAFETY: posting WM_QUIT to our own hotkey thread ends its message loop.
        unsafe {
            let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hotkeys() {
        assert_eq!(
            parse_hotkey("Alt+G").unwrap(),
            Hotkey { alt: true, ctrl: false, shift: false, win: false, virtual_key: 0x47 }
        );
        let combo = parse_hotkey("ctrl + shift + F10").unwrap();
        assert!(combo.ctrl && combo.shift && !combo.alt);
        assert_eq!(combo.virtual_key, 0x79);
        assert_eq!(parse_hotkey("Alt+Space").unwrap().virtual_key, 0x20);
        assert_eq!(parse_hotkey("Alt+1").unwrap().virtual_key, 0x31);
    }

    #[test]
    fn rejects_bad_hotkeys() {
        for bad in ["G", "Shift+G", "Alt+", "Alt+G+H", "Alt+F99", "Alt+Banana", ""] {
            assert!(parse_hotkey(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn config_is_clamped() {
        let config = OverlayConfig { opacity: 7.0, width: 10, height: 99_999, ..Default::default() }.clamped();
        assert_eq!((config.opacity, config.width, config.height), (1.0, 240, 1440));
        assert_eq!(OverlayConfig { opacity: f64::NAN, ..Default::default() }.clamped().opacity, 0.9);
    }
}
