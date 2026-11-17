//! Fullscreen game detection.
//!
//! Every couple of seconds the foreground window is sampled. If it is a
//! fullscreen game, Auto Game Mode starts: the browser drops to idle
//! priority, tightens its limits and hibernates background tabs. Short
//! flickers (alt-tabbing, loading screens) are ignored.

use serde::Serialize;

/// What we know about the application in the foreground.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ForegroundApp {
    pub pid: u32,
    /// Full path of the executable.
    pub exe_path: String,
    /// The window covers its whole monitor (borderless or exclusive).
    pub fullscreen: bool,
    /// Windows reports an exclusive Direct3D fullscreen application.
    pub d3d_fullscreen: bool,
    /// The window belongs to the browser itself.
    pub is_self: bool,
}

/// Fullscreen programs that are not games.
const NOT_GAMES: &[&str] = &[
    "explorer.exe", "chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "opera.exe", "vivaldi.exe",
    "aether.exe", "vlc.exe", "mpc-hc64.exe", "mpv.exe", "potplayermini64.exe", "wmplayer.exe",
    "powerpnt.exe", "obs64.exe", "code.exe", "devenv.exe", "windowsterminal.exe", "searchhost.exe",
    "startmenuexperiencehost.exe", "shellexperiencehost.exe", "lockapp.exe", "applicationframehost.exe",
    "steam.exe", "steamwebhelper.exe", "epicgameslauncher.exe", "battle.net.exe", "galaxyclient.exe",
    "discord.exe", "spotify.exe", "zoom.exe", "teams.exe",
];

/// Folders games are installed in by the major launchers.
const GAME_LIBRARY_MARKERS: &[&str] = &[
    "\\steamapps\\common\\", "\\epic games\\", "\\riot games\\", "\\gog galaxy\\games\\", "\\gog games\\",
    "\\xboxgames\\", "\\ubisoft game launcher\\games\\", "\\origin games\\", "\\ea games\\",
    "\\battle.net\\", "\\rockstar games\\", "\\games\\",
];

fn exe_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_ascii_lowercase()
}

/// Whether the foreground application should trigger game mode.
pub fn looks_like_game(app: &ForegroundApp) -> bool {
    if app.is_self || app.exe_path.is_empty() {
        return false;
    }
    if NOT_GAMES.contains(&exe_name(&app.exe_path).as_str()) {
        return false;
    }
    if app.d3d_fullscreen {
        return true;
    }
    let path = app.exe_path.to_ascii_lowercase().replace('/', "\\");
    app.fullscreen && GAME_LIBRARY_MARKERS.iter().any(|marker| path.contains(marker))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum GameEvent {
    Started { game: String },
    Stopped,
}

/// Turns raw samples into start/stop events with hysteresis.
#[derive(Debug)]
pub struct GameDetector {
    active: Option<String>,
    hits: u32,
    misses: u32,
    start_after: u32,
    stop_after: u32,
}

impl Default for GameDetector {
    fn default() -> Self {
        // Sampled every 2 s: start after ~4 s of game, stop after ~10 s without.
        Self::new(2, 5)
    }
}

impl GameDetector {
    pub fn new(start_after: u32, stop_after: u32) -> Self {
        Self { active: None, hits: 0, misses: 0, start_after: start_after.max(1), stop_after: stop_after.max(1) }
    }

    pub fn active_game(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Feeds one foreground sample; returns an event when the state changes.
    pub fn update(&mut self, foreground: Option<&ForegroundApp>) -> Option<GameEvent> {
        let game = foreground.filter(|app| looks_like_game(app)).map(|app| exe_name(&app.exe_path));
        match (game, self.active.is_some()) {
            (Some(name), false) => {
                self.hits += 1;
                if self.hits >= self.start_after {
                    self.hits = 0;
                    self.misses = 0;
                    self.active = Some(name.clone());
                    return Some(GameEvent::Started { game: name });
                }
            }
            (Some(_), true) => self.misses = 0,
            (None, true) => {
                self.misses += 1;
                if self.misses >= self.stop_after {
                    self.misses = 0;
                    self.hits = 0;
                    self.active = None;
                    return Some(GameEvent::Stopped);
                }
            }
            (None, false) => self.hits = 0,
        }
        None
    }
}

/// Samples the foreground window. `None` when there is none, or on platforms
/// without an implementation.
pub fn sample_foreground() -> Option<ForegroundApp> {
    #[cfg(windows)]
    {
        windows_impl::sample_foreground()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
mod windows_impl {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, RECT};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_RUNNING_D3D_FULL_SCREEN};
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId};

    use super::ForegroundApp;

    pub fn sample_foreground() -> Option<ForegroundApp> {
        // SAFETY: all calls take valid out-pointers to locals; the process
        // handle is closed before returning.
        unsafe {
            let window = GetForegroundWindow();
            if window.is_invalid() {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, Some(&mut pid));
            if pid == 0 {
                return None;
            }

            let mut rect = RECT::default();
            let mut fullscreen = false;
            if GetWindowRect(window, &mut rect).is_ok() {
                let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
                let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                if GetMonitorInfoW(monitor, &mut info).as_bool() {
                    let screen = info.rcMonitor;
                    fullscreen = rect.left <= screen.left
                        && rect.top <= screen.top
                        && rect.right >= screen.right
                        && rect.bottom >= screen.bottom;
                }
            }

            let d3d_fullscreen =
                SHQueryUserNotificationState().is_ok_and(|state| state == QUNS_RUNNING_D3D_FULL_SCREEN);

            let mut exe_path = String::new();
            if let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                let mut buffer = [0u16; 1024];
                let mut length = buffer.len() as u32;
                if QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut length)
                    .is_ok()
                {
                    exe_path = String::from_utf16_lossy(&buffer[..length as usize]);
                }
                let _ = CloseHandle(process);
            }

            Some(ForegroundApp { pid, exe_path, fullscreen, d3d_fullscreen, is_self: pid == std::process::id() })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(path: &str, fullscreen: bool, d3d: bool) -> ForegroundApp {
        ForegroundApp { pid: 4242, exe_path: path.into(), fullscreen, d3d_fullscreen: d3d, is_self: false }
    }

    const CS2: &str = r"D:\SteamLibrary\steamapps\common\Counter-Strike Global Offensive\game\bin\win64\cs2.exe";

    #[test]
    fn classifies_foreground_apps() {
        assert!(looks_like_game(&app(CS2, true, false)), "borderless fullscreen game in a Steam library");
        assert!(!looks_like_game(&app(CS2, false, false)), "windowed game does not trigger game mode");
        assert!(looks_like_game(&app(r"C:\Emulators\rpcs3\rpcs3.exe", true, true)), "exclusive D3D fullscreen");
        assert!(!looks_like_game(&app(r"C:\Program Files\Google\Chrome\Application\chrome.exe", true, true)));
        assert!(!looks_like_game(&app(r"C:\Program Files\VideoLAN\VLC\vlc.exe", true, false)));
        assert!(!looks_like_game(&app(r"C:\Program Files (x86)\Steam\steam.exe", true, false)), "launcher, not a game");
        assert!(!looks_like_game(&app(r"C:\Tools\notepad++.exe", true, false)));
        assert!(!looks_like_game(&ForegroundApp { is_self: true, ..app(CS2, true, true) }));
    }

    #[test]
    fn hysteresis_ignores_flicker() {
        let game = app(CS2, true, false);
        let desktop = app(r"C:\Windows\explorer.exe", false, false);
        let mut detector = GameDetector::new(2, 3);

        assert_eq!(detector.update(Some(&game)), None);
        assert_eq!(detector.update(Some(&desktop)), None, "a single sample is not enough");
        assert_eq!(detector.update(Some(&game)), None);
        assert_eq!(detector.update(Some(&game)), Some(GameEvent::Started { game: "cs2.exe".into() }));
        assert_eq!(detector.active_game(), Some("cs2.exe"));

        // A quick alt-tab does not end game mode.
        assert_eq!(detector.update(Some(&desktop)), None);
        assert_eq!(detector.update(None), None);
        assert_eq!(detector.update(Some(&game)), None);
        assert_eq!(detector.active_game(), Some("cs2.exe"));

        assert_eq!(detector.update(Some(&desktop)), None);
        assert_eq!(detector.update(Some(&desktop)), None);
        assert_eq!(detector.update(Some(&desktop)), Some(GameEvent::Stopped));
        assert_eq!(detector.active_game(), None);
    }
}
