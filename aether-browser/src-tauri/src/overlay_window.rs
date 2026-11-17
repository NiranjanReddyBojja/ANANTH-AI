//! The in-game overlay window: small, frameless, always on top.

use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Url, WebviewUrl};

use crate::commands::{OVERLAY_PAGE, OVERLAY_WINDOW};
use crate::state::AppState;

/// Height of the overlay's own toolbar, drawn by the shell UI.
const HEADER_HEIGHT: f64 = 40.0;

/// Creates the hidden overlay window. Called once at startup.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let config = app.state::<AppState>().overlay.read().unwrap_or_else(|p| p.into_inner()).clone();
    let (width, height) = (f64::from(config.width), f64::from(config.height));
    let window = tauri::window::WindowBuilder::new(app, OVERLAY_WINDOW)
        .title("Aether Overlay")
        .inner_size(width, height)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()?;
    window.add_child(
        WebviewBuilder::new("overlay-hud", WebviewUrl::App("index.html#overlay".into()))
            .position(LogicalPosition::new(0.0, 0.0))
            .size(LogicalSize::new(width, HEADER_HEIGHT)),
    )?;
    let start = Url::parse(&config.url).unwrap_or_else(|_| Url::parse("about:blank").expect("static URL"));
    window.add_child(
        WebviewBuilder::new(OVERLAY_PAGE, WebviewUrl::External(start))
            .position(LogicalPosition::new(0.0, HEADER_HEIGHT))
            .size(LogicalSize::new(width, (height - HEADER_HEIGHT).max(1.0))),
    )?;
    let handle = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Resized(_)) {
            layout(&handle);
        }
    });
    apply(app);
    Ok(())
}

fn layout(app: &AppHandle) {
    let Some(window) = app.get_window(OVERLAY_WINDOW) else { return };
    let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) else { return };
    let (width, height) = (f64::from(size.width) / scale, f64::from(size.height) / scale);
    if let Some(hud) = app.get_webview("overlay-hud") {
        let _ = hud.set_size(LogicalSize::new(width, HEADER_HEIGHT));
    }
    if let Some(page) = app.get_webview(OVERLAY_PAGE) {
        let _ = page.set_position(LogicalPosition::new(0.0, HEADER_HEIGHT));
        let _ = page.set_size(LogicalSize::new(width, (height - HEADER_HEIGHT).max(1.0)));
    }
}

/// Applies the saved size and opacity to the overlay window.
pub fn apply(app: &AppHandle) {
    let Some(window) = app.get_window(OVERLAY_WINDOW) else { return };
    let config = app.state::<AppState>().overlay.read().unwrap_or_else(|p| p.into_inner()).clone();
    let _ = window.set_size(LogicalSize::new(f64::from(config.width), f64::from(config.height)));
    layout(app);
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        set_window_opacity(hwnd.0 as isize, config.opacity);
    }
}

pub fn toggle(app: &AppHandle) {
    let Some(window) = app.get_window(OVERLAY_WINDOW) else { return };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.set_always_on_top(true);
        let _ = window.set_focus();
    }
}

pub fn navigate(app: &AppHandle, url: &str) {
    if let (Some(page), Ok(url)) = (app.get_webview(OVERLAY_PAGE), Url::parse(url)) {
        let _ = page.navigate(url);
    }
}

/// Makes the whole window translucent, page included.
#[cfg(windows)]
fn set_window_opacity(hwnd: isize, opacity: f64) {
    use windows::Win32::Foundation::{COLORREF, HWND};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA, WS_EX_LAYERED,
    };
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    let alpha = (opacity.clamp(0.2, 1.0) * 255.0).round() as u8;
    // SAFETY: `hwnd` is the live handle of our own overlay window.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED.0 as isize);
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
    }
}
