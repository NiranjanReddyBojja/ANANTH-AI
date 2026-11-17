# Aether Browser

A production-grade, low-RAM gaming web browser built with **Rust + Tauri (CEF Runtime) + TypeScript/React + SQLite**.

## Overview

Aether is designed to deliver Chrome-like browsing with:
- **40–60% lower RAM** than Chrome through tab hibernation and process limits
- **Native zero-extension ad, tracker, YouTube/Twitch blocking** via Chromium DevTools Protocol (Fetch interception)
- **Standard Chrome features:** history, bookmarks, downloads, session restore, split-view tabs
- **Privacy by default:** fingerprint guards, tracking-param stripping, secure password vault (Argon2id + AES-256-GCM)
- **Gaming-focused:** resource limiter (Windows Job Objects), game mode, overlay HUD (Alt+G)

## Building

### Prerequisites

- **Rust** (1.70+): `rustup update`
- **Node.js** (18+): for frontend build
- **System dependencies** (Linux):
  ```bash
  sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libgtk-4-dev \
    libsoup-3.0-dev librsvg2-dev libxdo-dev libayatana-appindicator3-dev \
    build-essential pkg-config libssl-dev
  ```

### Build Steps

```bash
# Install frontend dependencies
npm install

# Build in development mode (with features for testing)
npm run tauri dev

# Or build for release
npm run build
npm run tauri build
```

The release binary is in `src-tauri/target/release/aether`.

## Running

### Development
```bash
npm run tauri dev
```
Opens the browser in a dev window with hot reload of the frontend (Vite).

### Release
```bash
./target/release/aether
```
(or `aether.exe` on Windows)

**Environment variables:**
- `AETHER_DATA_DIR` — Override the data directory (default: `~/.local/share/aether.browser`)
- `RUST_LOG` — Set log level (e.g., `debug`, `info`)
- On Linux, `DISPLAY` can override the X11 display
- For headless testing: `AETHER_NO_SANDBOX=1 AETHER_DEBUG_PORT=9333` enables CDP on localhost:9333

## Testing

### Kernel Tests
```bash
cargo test --no-default-features --lib
```
Runs 42 unit tests covering storage, vault, shield, hibernation, and more.

### Harness Tests
Create a test harness with Xvfb and Playwright (see `scratchpad/aether/drive.js` for details):
```bash
# Start Xvfb and app
Xvfb :77 -screen 0 1400x900x24 &
DISPLAY=:77 AETHER_DEBUG_PORT=9333 AETHER_NO_SANDBOX=1 ./target/debug/aether &

# Start test server on localhost:8765
node server.js

# Run drive.js to navigate, block ads, test hibernation, etc.
node drive.js
```

## Architecture

### Layers

1. **Shell (Tauri Window Manager)**
   - Multi-webview support (tab tabs, overlay HUD)
   - Layout management (vertical/horizontal tabs, split panes, full-screen)
   - Event routing (page navigation, downloads, shortcuts)

2. **Rust Kernel** (`src-tauri/src/`)
   - `browser/` — Tab manager, webview lifecycle, navigation interception
   - `shield/` — Ad/tracker blocker (Brave's `adblock` crate + cosmetic filters)
   - `governor/` — Resource limiter, game detection, global hotkey overlay
   - `storage/` — SQLite database, encrypted password vault, session restore
   - `main.rs` — Chromium (CEF) configuration, initialization

3. **Frontend (React + TypeScript)**
   - `src/` — UI components (title bar, omnibox, settings, sidebar)
   - `lib/ipc.ts` — Typed IPC bridge to Rust commands
   - `store/browserStore.ts` — Lightweight sync state store
   - Tailwind CSS v4 for styling

4. **CEF Chromium Runtime**
   - Tauri's `tauri-runtime-cef` with Chromium 154+
   - Sandbox and site isolation enabled
   - DevTools Protocol (CDP) for ad blocking and content control

### Key Files

- `src-tauri/Cargo.toml` — Rust dependencies (tauri 3.0.0-alpha.6, adblock 0.13, rusqlite, windows Job Objects)
- `src-tauri/tauri.conf.json` — App config, capabilities, permissions
- `src-tauri/src/main.rs` — Browser initialization, Chromium flags
- `src-tauri/src/shield/adblocker.rs` — Fetch interception + cosmetic filters
- `src/App.tsx` — Main UI layout and event handlers

## Features & Limitations

### ✅ Implemented & Verified

- [x] Tab creation, navigation, history
- [x] Network ad/tracker blocking (EasyList, EasyPrivacy, uBlock Origin lists)
- [x] Cosmetic CSS filters (site-specific + generic class/id-based)
- [x] Fingerprint guard + Global Privacy Control injection
- [x] Tracking param stripping (utm_*, fbclid, etc.)
- [x] Session restore on startup
- [x] Tab hibernation (scroll position, screenshot, process cleanup)
- [x] Split-view tab layout
- [x] Bookmarks, history, downloads
- [x] Encrypted password vault (Argon2id, AES-256-GCM)
- [x] Per-tab shield stats
- [x] Telemetry (memory, CPU, process count)
- [x] Keyboard shortcuts (Ctrl+T, Ctrl+W, etc.)
- [x] Settings modal (11 sections)
- [x] Gamer sidebar + resource limiter display

### ⚠️ Known Limitations

1. **Sandbox** — Tauri's CEF runtime runs **without the Chromium sandbox on Windows** (per its docs); Linux/Mac sandboxing untested
2. **RAM/CPU limits** — Hard limits (Windows Job Objects) are Windows-only; on other platforms they are live gauges only, not enforced
3. **Game detection** — Windows-only; uses foreground window detection
4. **Global hotkey (Alt+G)** — Windows-only; overlay opacity also Windows-only
5. **Performance claims** — "40–60% lower RAM" and "zero FPS impact" are not measured; hibernation trades latency for memory
6. **YouTube/Twitch ad blocking** — Best-effort via JSON-prune scriptlets; unverified on live sites (Twitch ads are server-side)
7. **Password autofill** — Not implemented; vault is manual lookup only
8. **Extension signing** — Extensions load unpacked at startup via `--load-extension`; no signature check or Web Store integration
9. **Privacy settings** — Some (e.g., donottrack, webrtc policy) require a browser restart to apply
10. **Overlay fullscreen** — Does not show over exclusive-fullscreen games
11. **Audible tab detection** — Not implemented
12. **Hibernation screenshots** — Often unavailable for hidden/offscreen tabs
13. **Tauri 3 + CEF runtime** — Alpha; expect bugs and API changes

### YouTube/Twitch Pre-Roll

Video ads on YouTube are blocked via a JSON-prune scriptlet that removes ad keys from player responses. Twitch ads are served server-side and cannot be blocked from the client; the browser does not differentiate ad vs. content streams.

## Monorepo Structure

```
aether-browser/
├── Cargo.toml (workspace)
├── Cargo.lock
├── package.json
├── vite.config.ts
├── tsconfig.json
├── index.html (Vite entry)
├── src/                       # Frontend (React)
│   ├── main.tsx
│   ├── App.tsx
│   ├── index.css (Tailwind)
│   ├── lib/ipc.ts (IPC bridge)
│   ├── store/browserStore.ts
│   └── components/
│       ├── TitleBar.tsx
│       ├── Omnibox.tsx
│       ├── GamerSidebar.tsx
│       ├── SettingsModal.tsx
│       ├── OverlayHud.tsx
│       └── Icon.tsx
├── dist/                      # Built frontend (Vite output)
├── src-tauri/                 # Rust backend
│   ├── Cargo.toml
│   ├── build.rs
│   ├── tauri.conf.json
│   ├── capabilities/
│   ├── icons/
│   ├── src/
│   │   ├── lib.rs
│   │   ├── main.rs
│   │   ├── state.rs
│   │   ├── commands.rs        # ~54 Tauri commands
│   │   ├── overlay_window.rs
│   │   ├── browser/
│   │   │   ├── mod.rs
│   │   │   ├── manager.rs     # Tab webview lifecycle, CDP routing
│   │   │   ├── hibernation.rs
│   │   │   ├── extensions.rs
│   │   │   └── omnibox.rs
│   │   ├── shield/
│   │   │   ├── mod.rs
│   │   │   ├── adblocker.rs   # Fetch interception, cosmetics
│   │   │   ├── privacy.rs     # Fingerprint guard, tracking params
│   │   │   └── builtin_filters.txt
│   │   ├── governor/
│   │   │   ├── mod.rs
│   │   │   ├── limiter.rs     # Resource limits, telemetry
│   │   │   ├── game_detect.rs
│   │   │   └── overlay.rs
│   │   └── storage/
│   │       ├── mod.rs
│   │       ├── db.rs          # History, bookmarks, session
│   │       └── vault.rs       # Password encryption/decryption
│   └── gen/                   # Tauri-generated (auto)
├── target/                    # Rust build output (not committed)
├── target-kernel/             # Kernel-only builds
├── node_modules/              # NPM deps (not committed)
└── README.md
```

## Contributing

This is a proof-of-concept desktop browser. Contributions are welcome for:
- Performance tuning
- Security hardening
- Extending ad/tracker lists
- Adding platform-specific features (macOS, Linux)
- Fixing the known limitations above

## License

(Project license TBD)

## References

- [Tauri 3.0 Docs](https://tauri.app)
- [Brave's adblock](https://github.com/brave/adblock-rust)
- [Chromium DevTools Protocol](https://chromedevtools.github.io/devtools-protocol/)
- [EasyList Filters](https://easylist.to/)
