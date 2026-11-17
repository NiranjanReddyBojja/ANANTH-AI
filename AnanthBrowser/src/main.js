/**
 * Ananth Browser - Main Process
 *
 * Owns the window, the application menu (and with it every keyboard
 * shortcut), page context menus, downloads, permissions and persistence.
 * Web pages live in <webview> guests managed by the renderer.
 */

const {
  app, BrowserWindow, Menu, ipcMain, session, shell, dialog, nativeTheme, clipboard
} = require('electron');
const fs = require('fs');
const path = require('path');
const { Store } = require('./store');

const isMac = process.platform === 'darwin';
const isWin = process.platform === 'win32';
const TAB_STRIP_HEIGHT = 40;

// Lets tests and side-by-side installs use an isolated profile.
if (process.env.ANANTH_USER_DATA) app.setPath('userData', process.env.ANANTH_USER_DATA);

let mainWindow = null;
let store = null;
let pendingUrls = urlsFromArgv(process.argv);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function urlsFromArgv(argv) {
  return argv.slice(1).filter((arg) => /^https?:\/\//i.test(arg));
}

function isSafeExternal(url) {
  return /^(https?|mailto):/i.test(url);
}

function chromeColors() {
  return nativeTheme.shouldUseDarkColors
    ? { background: '#202124', symbol: '#e8eaed' }
    : { background: '#dee1e6', symbol: '#202124' };
}

function sendToUI(channel, ...args) {
  if (mainWindow && !mainWindow.isDestroyed()) mainWindow.webContents.send(channel, ...args);
}

function command(name, arg) {
  return () => sendToUI('command', name, arg);
}

function openTab(url, options = {}) {
  if (!mainWindow) {
    pendingUrls.push(url);
    createWindow();
    return;
  }
  sendToUI('tab:open', { url, ...options });
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

function createWindow() {
  const saved = store.get('window', {});
  const colors = chromeColors();

  mainWindow = new BrowserWindow({
    width: saved.width || 1280,
    height: saved.height || 840,
    x: saved.x,
    y: saved.y,
    minWidth: 480,
    minHeight: 360,
    show: false,
    backgroundColor: colors.background,
    autoHideMenuBar: true,
    titleBarStyle: isMac ? 'hiddenInset' : isWin ? 'hidden' : 'default',
    titleBarOverlay: isWin
      ? { color: colors.background, symbolColor: colors.symbol, height: TAB_STRIP_HEIGHT }
      : false,
    trafficLightPosition: { x: 13, y: 13 },
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      nodeIntegration: false,
      contextIsolation: true,
      sandbox: true,
      webviewTag: true,
      spellcheck: false
    }
  });

  if (saved.maximized) mainWindow.maximize();
  mainWindow.loadFile(path.join(__dirname, 'index.html'));
  mainWindow.once('ready-to-show', () => mainWindow.show());

  // The browser UI itself must never navigate away or open windows.
  mainWindow.webContents.on('will-navigate', (event) => event.preventDefault());
  mainWindow.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));

  const saveBounds = () => {
    if (!mainWindow || mainWindow.isDestroyed()) return;
    const maximized = mainWindow.isMaximized();
    const bounds = maximized || mainWindow.isFullScreen()
      ? store.get('window', {})
      : mainWindow.getBounds();
    store.set('window', { ...bounds, maximized });
  };
  mainWindow.on('resize', saveBounds);
  mainWindow.on('move', saveBounds);
  mainWindow.on('close', saveBounds);

  mainWindow.on('enter-full-screen', () => sendToUI('window:fullscreen', true));
  mainWindow.on('leave-full-screen', () => sendToUI('window:fullscreen', false));

  // Mouse back/forward buttons (Windows, Linux) and trackpad swipes (macOS).
  mainWindow.on('app-command', (_event, cmd) => {
    if (cmd === 'browser-backward') sendToUI('command', 'back');
    if (cmd === 'browser-forward') sendToUI('command', 'forward');
  });
  mainWindow.on('swipe', (_event, direction) => {
    if (direction === 'left') sendToUI('command', 'back');
    if (direction === 'right') sendToUI('command', 'forward');
  });

  mainWindow.on('closed', () => {
    mainWindow = null;
  });
}

nativeTheme.on('updated', () => {
  if (!mainWindow || mainWindow.isDestroyed()) return;
  const colors = chromeColors();
  mainWindow.setBackgroundColor(colors.background);
  if (isWin) {
    mainWindow.setTitleBarOverlay({
      color: colors.background, symbolColor: colors.symbol, height: TAB_STRIP_HEIGHT
    });
  }
});

// ---------------------------------------------------------------------------
// Application menu: the single source of keyboard shortcuts, so they work
// no matter whether focus is in the browser UI or inside a web page.
// ---------------------------------------------------------------------------

// Tab shortcuts a web page must not be able to swallow (as in Chrome).
const RESERVED_COMMANDS = new Set([
  'new-tab', 'close-tab', 'reopen-tab', 'next-tab', 'prev-tab', 'select-tab'
]);
const reservedShortcuts = [];

function parseAccelerator(accelerator) {
  const parts = accelerator.split('+');
  const key = parts.pop().toLowerCase();
  const has = (name) => parts.includes(name);
  return {
    key,
    control: has('Ctrl') || (!isMac && has('CmdOrCtrl')),
    meta: has('Cmd') || (isMac && has('CmdOrCtrl')),
    shift: has('Shift'),
    alt: has('Alt')
  };
}

function reservedCommandFor(input) {
  if (input.type !== 'keyDown') return null;
  const key = String(input.key).toLowerCase();
  return reservedShortcuts.find((shortcut) =>
    shortcut.key === key &&
    shortcut.control === input.control &&
    shortcut.meta === input.meta &&
    shortcut.shift === input.shift &&
    shortcut.alt === input.alt) || null;
}

function buildAppMenu() {
  reservedShortcuts.length = 0;
  const item = (label, accelerator, name, arg) => {
    if (RESERVED_COMMANDS.has(name)) reservedShortcuts.push({ ...parseAccelerator(accelerator), name, arg });
    return { label, accelerator, click: command(name, arg) };
  };
  const tabNumbers = [1, 2, 3, 4, 5, 6, 7, 8].map((n) =>
    item(`Select Tab ${n}`, `CmdOrCtrl+${n}`, 'select-tab', n - 1));

  const template = [
    ...(isMac ? [{ role: 'appMenu' }] : []),
    {
      label: 'File',
      submenu: [
        item('New Tab', 'CmdOrCtrl+T', 'new-tab'),
        item('Reopen Closed Tab', 'CmdOrCtrl+Shift+T', 'reopen-tab'),
        item('Open Location', 'CmdOrCtrl+L', 'focus-url'),
        { type: 'separator' },
        item('Close Tab', 'CmdOrCtrl+W', 'close-tab'),
        { type: 'separator' },
        item('Print…', 'CmdOrCtrl+P', 'print'),
        ...(isMac ? [] : [{ type: 'separator' }, { role: 'quit' }])
      ]
    },
    {
      label: 'Edit',
      submenu: [
        { role: 'undo' }, { role: 'redo' }, { type: 'separator' },
        { role: 'cut' }, { role: 'copy' }, { role: 'paste' },
        { role: 'pasteAndMatchStyle' }, { role: 'delete' }, { role: 'selectAll' },
        { type: 'separator' },
        item('Find in Page', 'CmdOrCtrl+F', 'find'),
        item('Find Next', 'CmdOrCtrl+G', 'find-next'),
        item('Find Previous', 'CmdOrCtrl+Shift+G', 'find-prev')
      ]
    },
    {
      label: 'View',
      submenu: [
        item('Reload', 'CmdOrCtrl+R', 'reload'),
        item('Reload (F5)', 'F5', 'reload'),
        item('Hard Reload', 'CmdOrCtrl+Shift+R', 'hard-reload'),
        { type: 'separator' },
        item('Zoom In', 'CmdOrCtrl+Plus', 'zoom-in'),
        item('Zoom In (=)', 'CmdOrCtrl+=', 'zoom-in'),
        item('Zoom Out', 'CmdOrCtrl+-', 'zoom-out'),
        item('Actual Size', 'CmdOrCtrl+0', 'zoom-reset'),
        { type: 'separator' },
        item('Show Bookmarks Bar', 'CmdOrCtrl+Shift+B', 'toggle-bookmarks-bar'),
        { role: 'togglefullscreen' },
        { type: 'separator' },
        item('Developer Tools', isMac ? 'Alt+Cmd+I' : 'Ctrl+Shift+I', 'devtools'),
        item('Developer Tools (F12)', 'F12', 'devtools')
      ]
    },
    {
      label: 'History',
      submenu: [
        item('Back', isMac ? 'Cmd+[' : 'Alt+Left', 'back'),
        item('Forward', isMac ? 'Cmd+]' : 'Alt+Right', 'forward'),
        { type: 'separator' },
        item('Show History', isMac ? 'Cmd+Y' : 'Ctrl+H', 'history'),
        item('Downloads', isMac ? 'Alt+Cmd+L' : 'Ctrl+J', 'downloads'),
        item('Clear Browsing Data…', 'CmdOrCtrl+Shift+Delete', 'clear-data')
      ]
    },
    {
      label: 'Bookmarks',
      submenu: [
        item('Bookmark This Tab', 'CmdOrCtrl+D', 'bookmark'),
        item('Show Bookmarks', isMac ? 'Alt+Cmd+B' : 'Ctrl+Shift+O', 'bookmarks')
      ]
    },
    {
      label: 'Tab',
      submenu: [
        item('Next Tab', 'Ctrl+Tab', 'next-tab'),
        item('Previous Tab', 'Ctrl+Shift+Tab', 'prev-tab'),
        item('Next Tab (Page Down)', 'CmdOrCtrl+PageDown', 'next-tab'),
        item('Previous Tab (Page Up)', 'CmdOrCtrl+PageUp', 'prev-tab'),
        { type: 'separator' },
        ...tabNumbers,
        item('Select Last Tab', 'CmdOrCtrl+9', 'select-tab', -1)
      ]
    },
    { role: 'windowMenu' }
  ];

  Menu.setApplicationMenu(Menu.buildFromTemplate(template));
}

// ---------------------------------------------------------------------------
// Web contents: harden guests, route new windows into tabs, context menus
// ---------------------------------------------------------------------------

function isBrowserUI(contents) {
  return mainWindow && !mainWindow.isDestroyed() && contents === mainWindow.webContents;
}

app.on('web-contents-created', (_event, contents) => {
  // Guests never get a preload script or Node access, whatever the page asks for.
  contents.on('will-attach-webview', (_e, webPreferences, params) => {
    delete webPreferences.preload;
    webPreferences.nodeIntegration = false;
    webPreferences.nodeIntegrationInSubFrames = false;
    webPreferences.contextIsolation = true;
    webPreferences.sandbox = true;
    webPreferences.webSecurity = true;
    delete params.preload;
  });

  // Everything below is for web pages (tabs and their popups), not our UI.
  setImmediate(() => {
    if (contents.isDestroyed() || isBrowserUI(contents)) return;
    if (contents.getType() !== 'webview' && contents.getType() !== 'window') return;
    attachPageHandlers(contents);
  });
});

function attachPageHandlers(contents) {
  contents.setWindowOpenHandler(({ url, disposition }) => {
    if (!/^(https?|about|blob|data):/i.test(url)) {
      if (isSafeExternal(url)) shell.openExternal(url);
      return { action: 'deny' };
    }
    // Real popups (sign-in flows etc.) keep their opener and get a window;
    // everything else becomes a tab.
    if (disposition === 'new-window') {
      return {
        action: 'allow',
        overrideBrowserWindowOptions: {
          width: 520,
          height: 680,
          autoHideMenuBar: true,
          parent: mainWindow || undefined,
          webPreferences: {
            preload: undefined, nodeIntegration: false, contextIsolation: true, sandbox: true
          }
        }
      };
    }
    openTab(url, { background: disposition === 'background-tab', openerId: contents.id });
    return { action: 'deny' };
  });

  if (contents.getType() === 'window') {
    // Popups have no address bar, so always show where they really are.
    contents.on('page-title-updated', (event, title) => {
      event.preventDefault();
      const win = BrowserWindow.fromWebContents(contents);
      let host = '';
      try { host = new URL(contents.getURL()).host; } catch { /* about:blank */ }
      if (win) win.setTitle(host ? `${host} — ${title}` : title);
    });
  }

  contents.on('context-menu', (_event, params) => showPageContextMenu(contents, params));

  // Handle reserved tab shortcuts before the page sees the key press.
  if (contents.getType() === 'webview') {
    contents.on('before-input-event', (event, input) => {
      const shortcut = reservedCommandFor(input);
      if (!shortcut) return;
      event.preventDefault();
      sendToUI('command', shortcut.name, shortcut.arg);
    });
  }
}

function showPageContextMenu(contents, params) {
  const { linkURL, srcURL, mediaType, selectionText, isEditable, editFlags } = params;
  const template = [];
  const separator = () => {
    if (template.length && template[template.length - 1].type !== 'separator') {
      template.push({ type: 'separator' });
    }
  };

  if (linkURL) {
    template.push(
      { label: 'Open Link in New Tab', click: () => openTab(linkURL, { background: true, openerId: contents.id }) },
      { label: 'Copy Link Address', click: () => clipboard.writeText(linkURL) }
    );
  }

  if (mediaType === 'image' && srcURL) {
    separator();
    template.push(
      { label: 'Open Image in New Tab', click: () => openTab(srcURL, { openerId: contents.id }) },
      { label: 'Save Image As…', click: () => contents.downloadURL(srcURL) },
      { label: 'Copy Image', click: () => contents.copyImageAt(params.x, params.y) },
      { label: 'Copy Image Address', click: () => clipboard.writeText(srcURL) }
    );
  }

  if (isEditable) {
    separator();
    for (const suggestion of (params.dictionarySuggestions || []).slice(0, 5)) {
      template.push({ label: suggestion, click: () => contents.replaceMisspelling(suggestion) });
    }
    if (params.misspelledWord) {
      template.push({
        label: 'Add to Dictionary',
        click: () => contents.session.addWordToSpellCheckerDictionary(params.misspelledWord)
      });
      separator();
    }
    template.push(
      { role: 'undo', enabled: editFlags.canUndo },
      { role: 'redo', enabled: editFlags.canRedo },
      { type: 'separator' },
      { role: 'cut', enabled: editFlags.canCut },
      { role: 'copy', enabled: editFlags.canCopy },
      { role: 'paste', enabled: editFlags.canPaste },
      { role: 'selectAll', enabled: editFlags.canSelectAll }
    );
  } else if (selectionText && selectionText.trim()) {
    separator();
    const text = selectionText.trim();
    const preview = text.length > 30 ? `${text.slice(0, 30)}…` : text;
    template.push(
      { role: 'copy' },
      { label: `Search the Web for “${preview}”`, click: () => sendToUI('tab:search', { query: text, openerId: contents.id }) }
    );
  }

  if (!linkURL && mediaType === 'none' && !isEditable && !(selectionText && selectionText.trim())) {
    const history = contents.navigationHistory;
    template.push(
      { label: 'Back', enabled: history.canGoBack(), click: () => history.goBack() },
      { label: 'Forward', enabled: history.canGoForward(), click: () => history.goForward() },
      { label: 'Reload', click: () => contents.reload() },
      { type: 'separator' },
      { label: 'Save Page As…', click: () => contents.downloadURL(contents.getURL()) },
      { label: 'Print…', click: () => contents.print() }
    );
  }

  separator();
  template.push({ label: 'Inspect', click: () => contents.inspectElement(params.x, params.y) });

  Menu.buildFromTemplate(template).popup({
    window: BrowserWindow.fromWebContents(contents.hostWebContents || contents) || undefined
  });
}

// ---------------------------------------------------------------------------
// Session: downloads and permissions
// ---------------------------------------------------------------------------

const downloads = new Map();
let nextDownloadId = 1;

function uniquePath(dir, filename) {
  const { name, ext } = path.parse(filename);
  let candidate = path.join(dir, filename);
  for (let i = 1; fs.existsSync(candidate); i++) {
    candidate = path.join(dir, `${name} (${i})${ext}`);
  }
  return candidate;
}

function setupDownloads(ses) {
  ses.on('will-download', (_event, item, contents) => {
    const id = nextDownloadId++;
    const savePath = uniquePath(app.getPath('downloads'), item.getFilename() || 'download');
    item.setSavePath(savePath);
    downloads.set(id, item);

    const snapshot = (state) => ({
      id,
      state,
      url: item.getURL(),
      filename: path.basename(savePath),
      path: savePath,
      received: item.getReceivedBytes(),
      total: item.getTotalBytes(),
      paused: item.isPaused(),
      startedAt: item.getStartTime() * 1000,
      sourceId: contents && !contents.isDestroyed() ? contents.id : null
    });

    sendToUI('download:update', snapshot('progressing'));
    item.on('updated', (_e, state) => sendToUI('download:update', snapshot(state)));
    item.once('done', (_e, state) => {
      downloads.delete(id);
      sendToUI('download:update', snapshot(state));
    });
  });
}

const AUTO_ALLOWED_PERMISSIONS = new Set([
  'fullscreen', 'pointerLock', 'clipboard-sanitized-write', 'keyboardLock'
]);
const PROMPTED_PERMISSIONS = {
  media: 'use your camera and microphone',
  geolocation: 'know your location',
  notifications: 'show notifications',
  'clipboard-read': 'see text and images copied to the clipboard',
  midi: 'use your MIDI devices',
  midiSysex: 'use your MIDI devices',
  'display-capture': 'share your screen'
};
const permissionDecisions = new Map();

function setupPermissions(ses) {
  ses.setPermissionRequestHandler((contents, permission, callback, details) => {
    if (AUTO_ALLOWED_PERMISSIONS.has(permission)) return callback(true);
    const description = PROMPTED_PERMISSIONS[permission];
    if (!description) return callback(false);

    let origin;
    try {
      origin = new URL(details.requestingUrl || contents.getURL()).origin;
    } catch {
      return callback(false);
    }
    if (!/^https?:/.test(origin)) return callback(false);

    const key = `${origin}|${permission}`;
    if (permissionDecisions.has(key)) return callback(permissionDecisions.get(key));

    dialog.showMessageBox(mainWindow, {
      type: 'question',
      message: `${origin} wants to ${description}`,
      detail: 'Your choice is remembered for this site until you quit.',
      buttons: ['Allow', 'Block'],
      defaultId: 1,
      cancelId: 1,
      noLink: true
    }).then(({ response }) => {
      const allowed = response === 0;
      permissionDecisions.set(key, allowed);
      callback(allowed);
    }).catch(() => callback(false));
  });
}

// ---------------------------------------------------------------------------
// IPC (only the browser UI may talk to us)
// ---------------------------------------------------------------------------

const STORE_KEYS = ['history', 'bookmarks', 'session', 'settings', 'window'];
const UI_STORE_KEYS = new Set(['history', 'bookmarks', 'session', 'settings']);

function fromUI(handler) {
  return (event, ...args) => {
    if (!isBrowserUI(event.sender)) return undefined;
    return handler(event, ...args);
  };
}

function initializeIPC() {
  ipcMain.handle('state:load', fromUI(() => {
    const urls = pendingUrls;
    pendingUrls = [];
    return {
      history: store.get('history', []),
      bookmarks: store.get('bookmarks', []),
      session: store.get('session', null),
      settings: store.get('settings', {}),
      openUrls: urls,
      fullscreen: mainWindow.isFullScreen()
    };
  }));

  ipcMain.on('store:set', fromUI((_event, key, value) => {
    if (UI_STORE_KEYS.has(key)) store.set(key, value);
  }));

  // Synchronous variant used while the window is unloading.
  ipcMain.on('store:set-sync', fromUI((event, key, value) => {
    if (UI_STORE_KEYS.has(key)) {
      store.set(key, value);
      store.write(key);
    }
    event.returnValue = true;
  }));

  // Generic native popup menu. Resolves with the id of the chosen item.
  ipcMain.handle('menu:popup', fromUI((_event, items, position) => new Promise((resolve) => {
    const build = (list) => list.map((entry) => {
      if (entry.type === 'separator') return { type: 'separator' };
      return {
        label: String(entry.label),
        type: entry.type === 'checkbox' || entry.type === 'radio' ? entry.type : undefined,
        checked: Boolean(entry.checked),
        enabled: entry.enabled !== false,
        accelerator: entry.accelerator,
        registerAccelerator: false,
        submenu: Array.isArray(entry.submenu) ? build(entry.submenu) : undefined,
        click: Array.isArray(entry.submenu) ? undefined : () => resolve(entry.id)
      };
    });
    const menu = Menu.buildFromTemplate(build(items));
    menu.popup({
      window: mainWindow,
      x: position ? Math.round(position.x) : undefined,
      y: position ? Math.round(position.y) : undefined,
      // The click handler can run just after the menu closes; give it a moment.
      callback: () => setTimeout(() => resolve(null), 150)
    });
  })));

  ipcMain.handle('data:confirm-clear', fromUI(async () => {
    const { response } = await dialog.showMessageBox(mainWindow, {
      type: 'warning',
      message: 'Clear browsing data?',
      detail: 'History is removed from this device. Clearing cookies signs you out of most sites.',
      buttons: ['Cancel', 'Clear History', 'Clear History, Cookies and Cache'],
      defaultId: 0,
      cancelId: 0,
      noLink: true
    });
    if (response === 2) {
      const ses = session.defaultSession;
      await Promise.all([ses.clearCache(), ses.clearStorageData()]);
    }
    return response; // 0 = cancelled, 1 = history, 2 = everything
  }));

  ipcMain.on('download:action', fromUI((_event, action, payload) => {
    const item = downloads.get(payload.id);
    if (action === 'cancel' && item) item.cancel();
    if (action === 'pause' && item) item.pause();
    if (action === 'resume' && item && item.canResume()) item.resume();
    // Only files we saved ourselves may be opened or revealed.
    if ((action === 'open' || action === 'reveal') && typeof payload.path === 'string') {
      const target = path.resolve(payload.path);
      if (!target.startsWith(path.resolve(app.getPath('downloads')) + path.sep)) return;
      if (!fs.existsSync(target)) return;
      if (action === 'open') shell.openPath(target);
      else shell.showItemInFolder(target);
    }
  }));

  ipcMain.on('window:close', fromUI(() => mainWindow.close()));
}

// ---------------------------------------------------------------------------
// App lifecycle
// ---------------------------------------------------------------------------

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on('second-instance', (_event, argv) => {
    const urls = urlsFromArgv(argv);
    if (!mainWindow) {
      pendingUrls.push(...urls);
      if (app.isReady()) createWindow();
      return;
    }
    if (mainWindow.isMinimized()) mainWindow.restore();
    mainWindow.focus();
    urls.forEach((url) => openTab(url));
  });

  // macOS: links opened from other apps.
  app.on('open-url', (event, url) => {
    event.preventDefault();
    if (!/^https?:\/\//i.test(url)) return;
    if (app.isReady()) openTab(url);
    else pendingUrls.push(url);
  });

  app.whenReady().then(() => {
    // Present as a regular Chromium browser; many sites reject unknown shells.
    app.userAgentFallback = app.userAgentFallback
      .replace(/\sElectron\/\S+/i, '')
      .replace(new RegExp(`\\s${app.getName()}/\\S+`, 'i'), '');

    store = new Store(app.getPath('userData'), STORE_KEYS);
    setupDownloads(session.defaultSession);
    setupPermissions(session.defaultSession);
    initializeIPC();
    buildAppMenu();
    createWindow();

    app.on('activate', () => {
      if (BrowserWindow.getAllWindows().length === 0) createWindow();
    });
  });

  app.on('window-all-closed', () => {
    if (!isMac) app.quit();
  });

  app.on('before-quit', () => {
    if (store) store.flush();
  });
}
