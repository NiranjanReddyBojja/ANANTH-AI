# Ananth Browser

A fast, minimal desktop web browser built on Electron (Chromium). It behaves the way
you expect a browser to behave: tabs, a combined address/search bar, history,
bookmarks, downloads and the usual keyboard shortcuts.

## Features

- **Tabs** – open, close, reorder by dragging, duplicate, mute, reopen closed tabs.
  Links that ask for a new tab get one; sign-in popups open as real popups.
- **Address bar** – type a URL or a search. Suggestions come from your history and
  bookmarks, with inline autocompletion of sites you visit often.
- **Navigation** – back, forward, reload/stop, loading indicator, link preview,
  error pages with a reload button.
- **Find in page**, **zoom** (remembered per site), **print**, **full screen video**.
- **Bookmarks** – star button, bookmarks bar and a bookmarks panel.
- **History** – searchable, grouped by day; frequently visited sites appear on the
  new tab page.
- **Downloads** – saved to your Downloads folder with progress, pause/resume/cancel.
- **Session restore** – your tabs come back when you reopen the browser. Background
  tabs only load when you switch to them, so startup stays quick.
- **Right-click menus** on pages (open link in new tab, copy, save image, spelling
  suggestions, inspect).
- **Light and dark themes** that follow the system setting.
- **Search engine** choice: Google, DuckDuckGo or Bing (in the ⋮ menu).

## Getting started

Requires Node.js 22.12 or newer.

```bash
cd AnanthBrowser
npm install
npm start
```

`npm run dev` starts it with Chromium logging enabled.

## Keyboard shortcuts

On macOS use ⌘ instead of Ctrl unless noted.

| Action | Shortcut |
| --- | --- |
| New tab / close tab | Ctrl+T / Ctrl+W |
| Reopen closed tab | Ctrl+Shift+T |
| Next / previous tab | Ctrl+Tab / Ctrl+Shift+Tab |
| Jump to tab 1–8 / last tab | Ctrl+1…8 / Ctrl+9 |
| Focus address bar | Ctrl+L |
| Back / forward | Alt+← / Alt+→ (macOS: ⌘[ / ⌘]) |
| Reload / hard reload | Ctrl+R or F5 / Ctrl+Shift+R |
| Find in page | Ctrl+F, then Enter / Shift+Enter |
| Zoom in / out / reset | Ctrl++ / Ctrl+- / Ctrl+0 |
| Bookmark this tab | Ctrl+D |
| Show bookmarks bar | Ctrl+Shift+B |
| History | Ctrl+H (macOS: ⌘Y) |
| Downloads | Ctrl+J (macOS: ⌥⌘L) |
| Clear browsing data | Ctrl+Shift+Delete |
| Developer tools | F12 or Ctrl+Shift+I |
| Open typed address in a new tab | Alt+Enter |

## Project structure

```
AnanthBrowser/
├── package.json
└── src/
    ├── main.js       # Main process: window, menu and shortcuts, context menus,
    │                 # downloads, permissions, IPC
    ├── store.js      # JSON persistence (history, bookmarks, session, settings)
    ├── preload.js    # Bridge between the browser UI and the main process
    ├── index.html    # Browser UI markup
    ├── styles.css    # Browser UI styles (light and dark)
    └── renderer.js   # Tabs, address bar, panels and webview management
```

## Privacy and security

- Web pages run in sandboxed `<webview>` guests with no Node.js access and no
  preload script; only the browser UI can talk to the main process.
- Camera, microphone, location, notifications and clipboard-read access prompt per
  site; the answer is remembered until you quit.
- History, bookmarks and the open-tab list are stored locally as JSON files in the
  app's user data folder. Nothing is sent anywhere. **Clear Browsing Data** removes
  history and, optionally, cookies and cache.

## Building installers

```bash
npm install --save-dev electron-builder
npx electron-builder
```

## License

MIT
