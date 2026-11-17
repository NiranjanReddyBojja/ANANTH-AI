/**
 * AuraBrowser - Browser UI
 *
 * Tabs, address bar, navigation, find-in-page, zoom, bookmarks, history and
 * downloads. Every web page is a <webview> guest; pages that are not the
 * active tab stay alive but hidden so switching tabs is instant.
 */

'use strict';

// ============================================================================
// Constants and state
// ============================================================================

const SEARCH_ENGINES = {
  google: { name: 'Google', url: 'https://www.google.com/search?q=%s' },
  duckduckgo: { name: 'DuckDuckGo', url: 'https://duckduckgo.com/?q=%s' },
  bing: { name: 'Bing', url: 'https://www.bing.com/search?q=%s' }
};

const ZOOM_STEPS = [0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4, 5];
const MAX_HISTORY = 3000;
const MAX_CLOSED_TABS = 25;
const MAX_SUGGESTIONS = 6;
const NEW_TAB_TITLE = 'New Tab';

const state = {
  tabs: [],
  activeId: null,
  closedTabs: [],
  history: [],
  historyByUrl: new Map(),
  bookmarks: [],
  downloads: [],
  settings: { searchEngine: 'google', showBookmarksBar: true },
  panel: null,
  nextTabId: 1
};

const $ = (id) => document.getElementById(id);
const dom = {
  tabs: $('tabs'),
  webviews: $('webviews'),
  urlInput: $('url-input'),
  omnibox: $('omnibox'),
  suggestions: $('suggestions'),
  siteIcon: $('site-icon'),
  siteIconUse: $('site-icon-use'),
  starIcon: $('star-icon'),
  reloadIcon: $('reload-icon'),
  zoomBadge: $('zoom-badge'),
  progress: $('progress'),
  bookmarksBar: $('bookmarks-bar'),
  ntp: $('ntp'),
  ntpInput: $('ntp-input'),
  ntpTiles: $('ntp-tiles'),
  errorPage: $('error-page'),
  findbar: $('findbar'),
  findInput: $('find-input'),
  findCount: $('find-count'),
  statusBubble: $('status-bubble'),
  panel: $('panel'),
  panelBody: $('panel-body'),
  panelTools: $('panel-tools'),
  panelSearch: $('panel-search'),
  btnBack: $('btn-back'),
  btnForward: $('btn-forward'),
  btnReload: $('btn-reload'),
  btnDownloads: $('btn-downloads'),
  downloadsProgress: $('downloads-progress')
};

// ============================================================================
// Small helpers
// ============================================================================

function el(tag, props = {}, children = []) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (key === 'class') node.className = value;
    else if (key === 'text') node.textContent = value;
    else if (key.startsWith('on')) node.addEventListener(key.slice(2), value);
    else if (value !== false && value != null) node.setAttribute(key, value === true ? '' : value);
  }
  for (const child of [].concat(children)) if (child) node.append(child);
  return node;
}

function icon(name, className = '') {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  if (className) svg.setAttribute('class', className);
  const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
  use.setAttribute('href', `#i-${name}`);
  svg.append(use);
  return svg;
}

/** A favicon image that quietly falls back to a globe. */
function faviconNode(url) {
  if (!url || !/^(https?:|data:image\/)/i.test(url)) return icon('globe', 'favicon');
  const img = el('img', { class: 'favicon', src: url, alt: '', draggable: 'false' });
  img.addEventListener('error', () => img.replaceWith(icon('globe', 'favicon')), { once: true });
  return img;
}

function debounce(fn, delay) {
  let timer = null;
  const wrapped = () => {
    clearTimeout(timer);
    timer = setTimeout(fn, delay);
  };
  wrapped.now = () => {
    clearTimeout(timer);
    fn();
  };
  return wrapped;
}

function hostOf(url) {
  try { return new URL(url).host; } catch { return ''; }
}

/** URL as shown in the address bar while it is not being edited. */
function displayUrl(url) {
  if (!url) return '';
  return url.replace(/^https?:\/\//i, '').replace(/^([^/?#]+)\/$/, '$1');
}

/** Comparable form used for matching typed text against known URLs. */
function matchableUrl(url) {
  return url.replace(/^https?:\/\//i, '').replace(/^www\./i, '').toLowerCase();
}

function searchUrl(query) {
  const engine = SEARCH_ENGINES[state.settings.searchEngine] || SEARCH_ENGINES.google;
  return engine.url.replace('%s', encodeURIComponent(query));
}

/** Turns whatever was typed into a URL to load: an address or a search. */
function resolveInput(raw) {
  const text = raw.trim();
  if (!text) return null;
  if (/^javascript:/i.test(text)) return null;
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(text)) return { type: 'url', url: text };
  if (/^(about|data|blob|view-source):/i.test(text)) return { type: 'url', url: text };
  if (/\s/.test(text)) return { type: 'search', url: searchUrl(text) };
  if (/^(localhost|\d{1,3}(\.\d{1,3}){3}|\[[0-9a-f:]+\])(:\d+)?([/?#].*)?$/i.test(text)) {
    return { type: 'url', url: `http://${text}` };
  }
  if (/^[^\s/?#@]+\.[a-z][a-z0-9-]{1,}(:\d+)?([/?#].*)?$/i.test(text)) {
    return { type: 'url', url: `https://${text}` };
  }
  return { type: 'search', url: searchUrl(text) };
}

function formatBytes(bytes) {
  if (!bytes) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** exponent;
  return `${value >= 100 || exponent === 0 ? Math.round(value) : value.toFixed(1)} ${units[exponent]}`;
}

// ============================================================================
// Persistence
// ============================================================================

function sessionSnapshot() {
  return {
    tabs: state.tabs.map((tab) => ({ url: tab.url, title: tab.title, favicon: tab.favicon })),
    active: Math.max(0, state.tabs.findIndex((tab) => tab.id === state.activeId))
  };
}

const saveSession = debounce(() => window.aura.save('session', sessionSnapshot()), 500);
const saveHistory = debounce(() => window.aura.save('history', state.history), 1500);
const saveBookmarks = debounce(() => window.aura.save('bookmarks', state.bookmarks), 300);
const saveSettings = debounce(() => window.aura.save('settings', state.settings), 300);

window.addEventListener('beforeunload', () => {
  window.aura.saveSync('session', sessionSnapshot());
  window.aura.saveSync('history', state.history);
  window.aura.saveSync('bookmarks', state.bookmarks);
  window.aura.saveSync('settings', state.settings);
});

// ============================================================================
// History and bookmarks data
// ============================================================================

function recordVisit(url) {
  if (!/^https?:/i.test(url)) return;
  let entry = state.historyByUrl.get(url);
  if (entry) {
    entry.visits += 1;
    entry.time = Date.now();
  } else {
    entry = { url, title: '', favicon: '', time: Date.now(), visits: 1 };
    state.history.push(entry);
    state.historyByUrl.set(url, entry);
    if (state.history.length > MAX_HISTORY) {
      state.history.sort((a, b) => a.time - b.time);
      for (const old of state.history.splice(0, state.history.length - MAX_HISTORY)) {
        state.historyByUrl.delete(old.url);
      }
    }
  }
  saveHistory();
  if (state.panel === 'history') renderPanel();
}

function updatePageMeta(url, patch) {
  const entry = state.historyByUrl.get(url);
  if (entry) {
    Object.assign(entry, patch);
    saveHistory();
  }
  const bookmark = state.bookmarks.find((item) => item.url === url);
  if (bookmark && patch.favicon && bookmark.favicon !== patch.favicon) {
    bookmark.favicon = patch.favicon;
    saveBookmarks();
    renderBookmarksBar();
  }
}

function removeHistoryEntry(entry) {
  state.history.splice(state.history.indexOf(entry), 1);
  state.historyByUrl.delete(entry.url);
  saveHistory();
}

function clearHistory() {
  state.history = [];
  state.historyByUrl.clear();
  state.closedTabs = [];
  saveHistory.now();
  renderNewTabPage();
  if (state.panel) renderPanel();
}

function isBookmarked(url) {
  return Boolean(url) && state.bookmarks.some((item) => item.url === url);
}

function toggleBookmark(tab = activeTab()) {
  if (!tab || !tab.url) return;
  const index = state.bookmarks.findIndex((item) => item.url === tab.url);
  if (index >= 0) state.bookmarks.splice(index, 1);
  else state.bookmarks.push({ url: tab.url, title: tab.title || displayUrl(tab.url), favicon: tab.favicon });
  bookmarksChanged();
}

function removeBookmark(bookmark) {
  const index = state.bookmarks.indexOf(bookmark);
  if (index >= 0) state.bookmarks.splice(index, 1);
  bookmarksChanged();
}

function bookmarksChanged() {
  saveBookmarks();
  renderBookmarksBar();
  updateStar();
  if (state.panel === 'bookmarks') renderPanel();
}

// ============================================================================
// Tabs
// ============================================================================

function activeTab() {
  return state.tabs.find((tab) => tab.id === state.activeId) || null;
}

function tabByWebContentsId(id) {
  return id == null ? null : state.tabs.find((tab) => tab.wcId === id) || null;
}

function tabTitle(tab) {
  return tab.title || displayUrl(tab.url) || NEW_TAB_TITLE;
}

function createTab({ url = '', title = '', favicon = '', background = false, index, lazy = false, openerId } = {}) {
  const tab = {
    id: state.nextTabId++,
    url,
    title,
    favicon,
    webview: null,
    ready: false,
    wcId: null,
    committed: false,
    openerId: openerId ?? null,
    loading: false,
    canGoBack: false,
    canGoForward: false,
    audible: false,
    muted: false,
    zoom: 1,
    draft: null,
    error: null,
    find: { open: false, text: '', count: '' },
    element: null
  };

  tab.element = buildTabElement(tab);
  const position = index == null ? state.tabs.length : Math.max(0, Math.min(index, state.tabs.length));
  state.tabs.splice(position, 0, tab);
  dom.tabs.insertBefore(tab.element, dom.tabs.children[position] || null);

  if (url && !lazy) attachWebview(tab, url);
  renderTab(tab);
  if (!background) activateTab(tab);
  saveSession();
  return tab;
}

function buildTabElement(tab) {
  const closeBtn = el('button', { class: 'tab-close', title: 'Close tab' }, icon('close'));
  closeBtn.addEventListener('click', (event) => {
    event.stopPropagation();
    closeTab(tab);
  });

  const audioBtn = el('button', { class: 'tab-audio', hidden: true }, icon('volume'));
  audioBtn.addEventListener('click', (event) => {
    event.stopPropagation();
    toggleMute(tab);
  });

  const node = el('div', { class: 'tab tab-enter', role: 'tab', draggable: 'true' }, [
    el('span', { class: 'tab-icon' }),
    el('span', { class: 'tab-title' }),
    audioBtn,
    closeBtn
  ]);

  node.addEventListener('mousedown', (event) => {
    if (event.button === 0) activateTab(tab);
    if (event.button === 1) event.preventDefault();
  });
  node.addEventListener('auxclick', (event) => {
    if (event.button === 1) closeTab(tab);
  });
  node.addEventListener('contextmenu', (event) => {
    event.preventDefault();
    showTabMenu(tab, event);
  });
  node.addEventListener('animationend', () => node.classList.remove('tab-enter'), { once: true });
  node.addEventListener('dragstart', (event) => {
    draggedTab = tab;
    event.dataTransfer.effectAllowed = 'move';
    event.dataTransfer.setData('text/plain', tab.url);
    node.classList.add('dragging');
  });
  node.addEventListener('dragend', () => {
    draggedTab = null;
    node.classList.remove('dragging');
    saveSession();
  });
  return node;
}

let draggedTab = null;

dom.tabs.addEventListener('dragover', (event) => {
  if (!draggedTab) return;
  event.preventDefault();
  const target = event.target.closest('.tab');
  if (!target || target === draggedTab.element) return;
  const over = state.tabs.find((tab) => tab.element === target);
  if (!over) return;
  const rect = target.getBoundingClientRect();
  const from = state.tabs.indexOf(draggedTab);
  const to = state.tabs.indexOf(over);
  const pastMiddle = event.clientX > rect.left + rect.width / 2;
  // Only swap once the pointer has crossed the neighbour's midpoint.
  if ((from < to && !pastMiddle) || (from > to && pastMiddle)) return;
  state.tabs.splice(from, 1);
  state.tabs.splice(to, 0, draggedTab);
  dom.tabs.insertBefore(draggedTab.element, from < to ? target.nextSibling : target);
});

function renderTab(tab) {
  const node = tab.element;
  const title = tabTitle(tab);
  node.querySelector('.tab-title').textContent = title;
  node.title = tab.url ? `${title}\n${tab.url}` : title;

  const iconSlot = node.querySelector('.tab-icon');
  const iconKey = tab.loading ? 'loading' : tab.favicon || (tab.url ? 'globe' : 'none');
  if (iconSlot.dataset.key !== iconKey) {
    iconSlot.dataset.key = iconKey;
    iconSlot.replaceChildren(
      tab.loading ? el('span', { class: 'spinner' })
        : tab.url ? faviconNode(tab.favicon)
          : icon('search', 'favicon')
    );
  }

  const audioBtn = node.querySelector('.tab-audio');
  audioBtn.hidden = !(tab.audible || tab.muted);
  audioBtn.title = tab.muted ? 'Unmute tab' : 'Mute tab';
  audioBtn.firstChild.firstChild.setAttribute('href', tab.muted ? '#i-muted' : '#i-volume');
}

function activateTab(tab) {
  if (!tab) return;
  const previous = activeTab();
  if (previous && previous !== tab) {
    previous.element.classList.remove('active');
    if (previous.webview) previous.webview.classList.remove('active');
    if (document.activeElement === dom.urlInput) dom.urlInput.blur();
  }

  state.activeId = tab.id;
  tab.element.classList.add('active');
  tab.element.setAttribute('aria-selected', 'true');
  if (previous && previous !== tab) previous.element.removeAttribute('aria-selected');

  // Tabs restored from the last session load on first use.
  if (tab.url && !tab.webview) attachWebview(tab, tab.url);
  if (tab.webview) tab.webview.classList.add('active');

  tab.element.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  hideSuggestions();
  updateChrome();
  refreshZoom(tab);

  if (!tab.url) focusAddressBar();
  else if (tab.webview) tab.webview.focus();
  saveSession();
}

function closeTab(tab, { remember = true } = {}) {
  const index = state.tabs.indexOf(tab);
  if (index < 0) return;

  // Closing the last tab closes the window; the session keeps that tab.
  if (state.tabs.length === 1) {
    window.aura.closeWindow();
    return;
  }

  if (remember && tab.url) {
    state.closedTabs.push({ url: tab.url, title: tab.title, favicon: tab.favicon, index });
    if (state.closedTabs.length > MAX_CLOSED_TABS) state.closedTabs.shift();
  }

  state.tabs.splice(index, 1);
  if (tab.id === state.activeId) activateTab(state.tabs[index] || state.tabs[index - 1]);

  if (tab.webview) tab.webview.remove();
  tab.webview = null;

  const node = tab.element;
  node.classList.add('closing');
  const remove = () => node.remove();
  node.addEventListener('transitionend', remove, { once: true });
  setTimeout(remove, 200);
  saveSession();
}

function reopenClosedTab() {
  const closed = state.closedTabs.pop();
  if (closed) createTab(closed);
}

function selectTabByOffset(offset) {
  const index = state.tabs.findIndex((tab) => tab.id === state.activeId);
  const count = state.tabs.length;
  activateTab(state.tabs[(index + offset + count) % count]);
}

function toggleMute(tab) {
  tab.muted = !tab.muted;
  callWebview(tab, (webview) => webview.setAudioMuted(tab.muted));
  renderTab(tab);
}

async function showTabMenu(tab, event) {
  const index = state.tabs.indexOf(tab);
  const choice = await window.aura.popupMenu([
    { id: 'new', label: 'New Tab to the Right' },
    { type: 'separator' },
    { id: 'reload', label: 'Reload', enabled: Boolean(tab.url) },
    { id: 'duplicate', label: 'Duplicate', enabled: Boolean(tab.url) },
    { id: 'mute', label: tab.muted ? 'Unmute Tab' : 'Mute Tab', enabled: Boolean(tab.webview) },
    { type: 'separator' },
    { id: 'close', label: 'Close' },
    { id: 'close-others', label: 'Close Other Tabs', enabled: state.tabs.length > 1 },
    { id: 'close-right', label: 'Close Tabs to the Right', enabled: index < state.tabs.length - 1 },
    { type: 'separator' },
    { id: 'reopen', label: 'Reopen Closed Tab', enabled: state.closedTabs.length > 0 }
  ], { x: event.clientX, y: event.clientY });

  if (choice === 'new') createTab({ index: index + 1 });
  if (choice === 'reload') reload(tab);
  if (choice === 'duplicate') createTab({ url: tab.url, title: tab.title, favicon: tab.favicon, index: index + 1 });
  if (choice === 'mute') toggleMute(tab);
  if (choice === 'close') closeTab(tab);
  if (choice === 'close-others') {
    activateTab(tab);
    state.tabs.filter((other) => other !== tab).forEach((other) => closeTab(other));
  }
  if (choice === 'close-right') {
    if (state.tabs.indexOf(activeTab()) > index) activateTab(tab);
    state.tabs.slice(index + 1).forEach((other) => closeTab(other));
  }
  if (choice === 'reopen') reopenClosedTab();
}

// ============================================================================
// Webviews
// ============================================================================

/** Runs fn against the tab's webview once it is able to take calls. */
function callWebview(tab, fn) {
  if (!tab || !tab.webview || !tab.ready) return undefined;
  try {
    return fn(tab.webview);
  } catch (error) {
    console.warn('[webview]', error.message);
    return undefined;
  }
}

function attachWebview(tab, url) {
  const webview = document.createElement('webview');
  webview.className = 'webview';
  webview.setAttribute('allowpopups', '');
  webview.setAttribute('src', url);
  tab.webview = webview;
  bindWebviewEvents(tab, webview);
  if (tab.id === state.activeId) webview.classList.add('active');
  dom.webviews.append(webview);
}

function bindWebviewEvents(tab, webview) {
  const on = (name, handler) => webview.addEventListener(name, (event) => {
    if (tab.webview === webview) handler(event);
  });
  const ifActive = (fn) => { if (tab.id === state.activeId) fn(); };

  const syncNavigation = () => {
    tab.canGoBack = callWebview(tab, (w) => w.canGoBack()) || false;
    tab.canGoForward = callWebview(tab, (w) => w.canGoForward()) || false;
    ifActive(updateNavButtons);
  };

  on('did-attach', () => {
    try { tab.wcId = webview.getWebContentsId(); } catch { /* not attached yet */ }
  });

  on('dom-ready', () => {
    tab.ready = true;
    if (tab.wcId == null) tab.wcId = webview.getWebContentsId();
    if (tab.muted) webview.setAudioMuted(true);
    syncNavigation();
    ifActive(() => refreshZoom(tab));
  });

  on('did-start-loading', () => {
    tab.loading = true;
    renderTab(tab);
    ifActive(updateLoading);
  });

  on('did-stop-loading', () => {
    tab.loading = false;
    renderTab(tab);
    syncNavigation();
    ifActive(updateLoading);
  });

  on('did-start-navigation', (event) => {
    if (!event.isMainFrame || event.isSameDocument) return;
    if (tab.error) {
      tab.error = null;
      ifActive(updateOverlays);
    }
  });

  const committed = (event) => {
    if (event.isMainFrame === false) return;
    const changed = tab.url !== event.url;
    tab.url = event.url;
    tab.committed = true;
    tab.draft = null;
    if (changed && event.type === 'did-navigate') {
      tab.title = '';
      tab.favicon = '';
    }
    recordVisit(event.url);
    renderTab(tab);
    syncNavigation();
    ifActive(() => {
      updateAddressBar();
      updateStar();
      refreshZoom(tab);
    });
    saveSession();
  };

  on('did-navigate', (event) => {
    tab.error = null;
    tab.find.count = '';
    committed(event);
    ifActive(updateOverlays);
  });
  on('did-navigate-in-page', committed);

  on('page-title-updated', (event) => {
    tab.title = event.title;
    updatePageMeta(tab.url, { title: event.title });
    renderTab(tab);
    ifActive(updateWindowTitle);
    saveSession();
  });

  on('page-favicon-updated', (event) => {
    const favicon = (event.favicons || [])[0] || '';
    if (!favicon || favicon === tab.favicon) return;
    tab.favicon = favicon;
    updatePageMeta(tab.url, { favicon });
    renderTab(tab);
    saveSession();
  });

  on('did-fail-load', (event) => {
    if (!event.isMainFrame || event.errorCode === -3) return; // -3: cancelled
    tab.url = event.validatedURL || tab.url;
    tab.error = { code: event.errorCode, description: event.errorDescription };
    tab.loading = false;
    renderTab(tab);
    ifActive(updateChrome);
  });

  on('render-process-gone', (event) => {
    if (event.reason === 'clean-exit') return;
    tab.error = { code: 0, description: 'RENDERER_CRASHED', crashed: true };
    tab.loading = false;
    renderTab(tab);
    ifActive(updateChrome);
  });

  on('found-in-page', (event) => {
    const { activeMatchOrdinal, matches } = event.result;
    tab.find.count = matches ? `${activeMatchOrdinal}/${matches}` : '0/0';
    ifActive(() => {
      dom.findCount.textContent = tab.find.count;
      dom.findbar.classList.toggle('no-match', !matches);
    });
  });

  on('update-target-url', (event) => ifActive(() => showStatus(event.url)));

  on('media-started-playing', () => {
    tab.audible = true;
    renderTab(tab);
  });
  on('media-paused', () => {
    setTimeout(() => {
      tab.audible = callWebview(tab, (w) => w.isCurrentlyAudible()) || false;
      renderTab(tab);
    }, 300);
  });

  on('enter-html-full-screen', () => document.body.classList.add('page-fullscreen'));
  on('leave-html-full-screen', () => document.body.classList.remove('page-fullscreen'));

  // window.close() from the page.
  on('close', () => closeTab(tab));

  // Clicking into the page dismisses any open popups in the browser UI.
  on('focus', () => {
    hideSuggestions();
    if (state.panel) closePanel();
  });
}

// ============================================================================
// Navigation
// ============================================================================

function navigate(tab, url) {
  if (!tab || !url) return;
  tab.url = url;
  tab.error = null;
  tab.draft = null;
  tab.title = '';
  tab.favicon = '';

  if (!tab.webview) attachWebview(tab, url);
  else if (tab.ready) tab.webview.loadURL(url).catch(() => { /* reported via did-fail-load */ });
  else tab.webview.setAttribute('src', url);

  renderTab(tab);
  if (tab.id === state.activeId) {
    hideSuggestions();
    updateChrome();
    tab.webview.focus();
  }
  saveSession();
}

function navigateFromText(text, { newTab = false } = {}) {
  const resolved = resolveInput(text);
  if (!resolved) return;
  if (newTab) createTab({ url: resolved.url, index: state.tabs.indexOf(activeTab()) + 1 });
  else navigate(activeTab(), resolved.url);
}

function goBack(tab = activeTab()) {
  callWebview(tab, (w) => w.canGoBack() && w.goBack());
}

function goForward(tab = activeTab()) {
  callWebview(tab, (w) => w.canGoForward() && w.goForward());
}

function reload(tab = activeTab(), { ignoreCache = false } = {}) {
  if (!tab || !tab.url) return;
  if (tab.error || !tab.ready) return navigate(tab, tab.url);
  callWebview(tab, (w) => (ignoreCache ? w.reloadIgnoringCache() : w.reload()));
}

function stopLoading(tab = activeTab()) {
  callWebview(tab, (w) => w.stop());
}

// ============================================================================
// Toolbar state
// ============================================================================

function updateChrome() {
  updateAddressBar();
  updateNavButtons();
  updateLoading();
  updateStar();
  updateOverlays();
  updateFindbar();
  updateWindowTitle();
  showStatus('');
}

function updateWindowTitle() {
  const tab = activeTab();
  document.title = tab && tab.url ? `${tabTitle(tab)} - AuraBrowser` : 'AuraBrowser';
}

function updateNavButtons() {
  const tab = activeTab();
  dom.btnBack.disabled = !tab || !tab.canGoBack;
  dom.btnForward.disabled = !tab || !tab.canGoForward;
}

function updateLoading() {
  const tab = activeTab();
  const loading = Boolean(tab && tab.loading);
  dom.reloadIcon.setAttribute('href', loading ? '#i-close' : '#i-reload');
  dom.btnReload.title = loading ? 'Stop' : 'Reload';
  dom.btnReload.disabled = !tab || !tab.url;

  if (loading) {
    dom.progress.classList.remove('done');
    dom.progress.classList.add('loading');
  } else if (dom.progress.classList.contains('loading')) {
    dom.progress.classList.remove('loading');
    dom.progress.classList.add('done');
  }
}

function updateStar() {
  const tab = activeTab();
  const bookmarked = Boolean(tab) && isBookmarked(tab.url);
  dom.starIcon.setAttribute('href', bookmarked ? '#i-star-filled' : '#i-star');
  $('btn-star').classList.toggle('on', bookmarked);
  $('btn-star').hidden = !tab || !tab.url;
  $('btn-star').title = bookmarked ? 'Remove bookmark' : 'Bookmark this tab';
}

function updateAddressBar() {
  const tab = activeTab();
  if (!tab) return;
  const editing = document.activeElement === dom.urlInput;

  if (!(editing && tab.draft != null)) {
    dom.urlInput.value = editing ? tab.url : (tab.draft ?? displayUrl(tab.url));
    if (editing) dom.urlInput.select();
  }

  let symbol = 'search';
  let label = '';
  if (/^https:/i.test(tab.url)) { symbol = 'lock'; label = 'Connection is secure'; }
  else if (/^http:/i.test(tab.url)) { symbol = 'warning'; label = 'Connection is not secure'; }
  else if (tab.url) { symbol = 'globe'; }
  dom.siteIconUse.setAttribute('href', `#i-${symbol}`);
  dom.siteIcon.title = label;
  dom.siteIcon.classList.toggle('insecure', symbol === 'warning');
}

function focusAddressBar() {
  dom.urlInput.focus();
  dom.urlInput.select();
}

// ============================================================================
// Overlays: new tab page and error page
// ============================================================================

const ERROR_TEXT = {
  '-105': ['This site can’t be reached', 'The server’s address could not be found. Check the address for typos.'],
  '-137': ['This site can’t be reached', 'The server’s address could not be found. Check the address for typos.'],
  '-106': ['No internet connection', 'Check your network cables, modem and router, then try again.'],
  '-102': ['This site can’t be reached', 'The server refused to connect.'],
  '-118': ['This site can’t be reached', 'The server took too long to respond.'],
  '-7': ['This site can’t be reached', 'The server took too long to respond.'],
  '-21': ['Your connection was interrupted', 'A network change was detected.'],
  '-101': ['This site can’t be reached', 'The connection was reset.']
};

function updateOverlays() {
  const tab = activeTab();
  const showNewTab = Boolean(tab) && !tab.url;
  if (showNewTab && dom.ntp.hidden) renderNewTabPage();
  dom.ntp.hidden = !showNewTab;

  const error = tab && tab.error;
  dom.errorPage.hidden = !error;
  if (!error) return;

  let [title, detail] = ERROR_TEXT[String(error.code)] || ['This page couldn’t be loaded', 'Something went wrong while loading this page.'];
  if (error.crashed) {
    [title, detail] = ['This page crashed', 'Something went wrong while displaying this page. Reload to try again.'];
  } else if (error.code <= -200 && error.code > -300) {
    [title, detail] = ['Your connection is not private', `The security certificate for ${hostOf(tab.url) || 'this site'} is not trusted, so the page was not loaded.`];
  }
  $('error-title').textContent = title;
  $('error-detail').textContent = detail;
  $('error-code').textContent = error.description || '';
}

function topSites(limit = 8) {
  const sites = new Map();
  for (const entry of state.history) {
    let origin;
    try { origin = new URL(entry.url).origin; } catch { continue; }
    const site = sites.get(origin) || { url: `${origin}/`, visits: 0, time: 0, favicon: '' };
    site.visits += entry.visits;
    if (entry.time >= site.time) {
      site.time = entry.time;
      site.favicon = entry.favicon || site.favicon;
    }
    sites.set(origin, site);
  }
  return [...sites.values()].sort((a, b) => b.visits - a.visits || b.time - a.time).slice(0, limit);
}

function renderNewTabPage() {
  dom.ntpInput.value = '';
  dom.ntpTiles.replaceChildren(...topSites().map((site) => {
    const name = hostOf(site.url).replace(/^www\./, '');
    const badge = el('span', { class: 'tile-icon' });
    if (site.favicon) badge.append(faviconNode(site.favicon));
    else badge.textContent = name.charAt(0).toUpperCase();
    return el('button', {
      class: 'tile',
      title: site.url,
      onclick: () => navigate(activeTab(), site.url)
    }, [badge, el('span', { class: 'tile-name', text: name })]);
  }));
}

// ============================================================================
// Address bar suggestions
// ============================================================================

let suggestions = [];
let selectedSuggestion = 0;

function computeSuggestions(text) {
  const query = text.trim().toLowerCase();
  if (!query) return [];
  const needle = matchableUrl(query);
  const words = query.split(/\s+/);
  const now = Date.now();
  const scored = new Map();

  const consider = (item, bonus) => {
    const haystack = matchableUrl(item.url);
    const title = (item.title || '').toLowerCase();
    let score;
    if (haystack.startsWith(needle)) score = 100;
    else if (haystack.split('/')[0].includes(needle)) score = 60;
    else if (haystack.includes(needle)) score = 40;
    else if (title && words.every((word) => title.includes(word))) score = 30;
    else return;
    score += bonus + Math.min(item.visits || 0, 20) * 2;
    if (item.time) score += Math.max(0, 10 - (now - item.time) / 2.6e8); // fades over ~30 days
    const existing = scored.get(item.url);
    if (!existing || existing.score < score) {
      scored.set(item.url, { kind: 'page', url: item.url, title: item.title, favicon: item.favicon, score, prefix: haystack.startsWith(needle) });
    }
  };

  state.history.forEach((item) => consider(item, 0));
  state.bookmarks.forEach((item) => consider(item, 15));

  const resolved = resolveInput(text);
  const first = resolved.type === 'url'
    ? { kind: 'url', url: resolved.url, title: text.trim() }
    : { kind: 'search', url: resolved.url, title: text.trim() };

  const pages = [...scored.values()]
    .filter((item) => item.url !== first.url)
    .sort((a, b) => b.score - a.score)
    .slice(0, MAX_SUGGESTIONS);
  return [first, ...pages];
}

/** Text to append so the typed prefix completes to the best known address. */
function inlineCompletion(text, list) {
  if (/\s/.test(text) || text.length < 2) return '';
  const needle = matchableUrl(text);
  const best = list.find((item) => item.kind === 'page' && item.prefix);
  if (!best || !needle) return '';
  const full = matchableUrl(best.url);
  // Complete to the host unless the user is already typing a path.
  const target = needle.includes('/') ? full : `${full.split('/')[0]}/`;
  return target.startsWith(needle) ? target.slice(needle.length) : '';
}

function showSuggestions(text, allowCompletion) {
  suggestions = computeSuggestions(text);
  selectedSuggestion = 0;

  if (allowCompletion) {
    const completion = inlineCompletion(text, suggestions);
    if (completion) {
      dom.urlInput.value = text + completion;
      dom.urlInput.setSelectionRange(text.length, text.length + completion.length);
      const completed = resolveInput(dom.urlInput.value);
      suggestions[0] = { kind: 'url', url: completed.url, title: dom.urlInput.value };
    }
  }
  renderSuggestions();
}

function renderSuggestions() {
  if (!suggestions.length) return hideSuggestions();
  const engine = (SEARCH_ENGINES[state.settings.searchEngine] || SEARCH_ENGINES.google).name;

  dom.suggestions.replaceChildren(...suggestions.map((item, index) => {
    const row = el('div', {
      class: `suggestion${index === selectedSuggestion ? ' selected' : ''}`,
      role: 'option'
    }, [
      item.kind === 'page' ? faviconNode(item.favicon) : icon(item.kind === 'search' ? 'search' : 'globe', 'favicon'),
      el('span', { class: 'suggestion-title', text: item.kind === 'page' ? (item.title || displayUrl(item.url)) : item.title }),
      el('span', {
        class: 'suggestion-detail',
        text: item.kind === 'search' ? `${engine} Search` : item.kind === 'page' ? displayUrl(item.url) : ''
      })
    ]);
    // mousedown (not click) so the input keeps focus until we navigate.
    row.addEventListener('mousedown', (event) => {
      event.preventDefault();
      if (event.button !== 0) return;
      navigate(activeTab(), item.url);
    });
    return row;
  }));
  dom.suggestions.hidden = false;
  dom.omnibox.classList.add('open');
}

function hideSuggestions() {
  suggestions = [];
  dom.suggestions.hidden = true;
  dom.omnibox.classList.remove('open');
}

function moveSuggestion(delta) {
  if (!suggestions.length) return;
  selectedSuggestion = (selectedSuggestion + delta + suggestions.length) % suggestions.length;
  const item = suggestions[selectedSuggestion];
  dom.urlInput.value = item.kind === 'page' ? item.url : item.title;
  [...dom.suggestions.children].forEach((row, index) => row.classList.toggle('selected', index === selectedSuggestion));
}

dom.urlInput.addEventListener('mousedown', (event) => {
  // First click selects everything, like every other browser.
  if (document.activeElement !== dom.urlInput) {
    event.preventDefault();
    dom.urlInput.focus();
  }
});

dom.urlInput.addEventListener('focus', () => {
  const tab = activeTab();
  dom.omnibox.classList.add('focused');
  if (!tab) return;
  dom.urlInput.value = tab.draft ?? tab.url;
  dom.urlInput.select();
});

dom.urlInput.addEventListener('blur', () => {
  const tab = activeTab();
  dom.omnibox.classList.remove('focused');
  hideSuggestions();
  if (!tab) return;
  if (tab.draft === '' || tab.draft === tab.url) tab.draft = null;
  if (tab.draft == null) dom.urlInput.value = displayUrl(tab.url);
});

dom.urlInput.addEventListener('input', (event) => {
  const tab = activeTab();
  if (!tab) return;
  const text = dom.urlInput.value;
  tab.draft = text;
  const typing = !event.isComposing && String(event.inputType || '').startsWith('insert')
    && dom.urlInput.selectionStart === text.length;
  showSuggestions(text, typing);
});

dom.urlInput.addEventListener('keydown', (event) => {
  if (event.isComposing) return;
  if (event.key === 'ArrowDown') { event.preventDefault(); moveSuggestion(1); }
  else if (event.key === 'ArrowUp') { event.preventDefault(); moveSuggestion(-1); }
  else if (event.key === 'Enter') {
    event.preventDefault();
    const picked = selectedSuggestion > 0 ? suggestions[selectedSuggestion] : null;
    if (picked && !event.altKey) navigate(activeTab(), picked.url);
    else navigateFromText(picked ? picked.url : dom.urlInput.value, { newTab: event.altKey });
  } else if (event.key === 'Escape') {
    event.preventDefault();
    const tab = activeTab();
    if (tab && (tab.draft != null || suggestions.length)) {
      tab.draft = null;
      hideSuggestions();
      dom.urlInput.value = tab.url;
      dom.urlInput.select();
    } else if (tab && tab.webview) {
      tab.webview.focus();
    }
  }
});

dom.ntpInput.addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && !event.isComposing) navigateFromText(dom.ntpInput.value);
});

// ============================================================================
// Find in page
// ============================================================================

function openFind() {
  const tab = activeTab();
  if (!tab || !tab.webview) return;
  tab.find.open = true;
  updateFindbar();
  dom.findInput.focus();
  dom.findInput.select();
}

function closeFind() {
  const tab = activeTab();
  if (!tab) return;
  tab.find.open = false;
  tab.find.count = '';
  callWebview(tab, (w) => w.stopFindInPage('clearSelection'));
  updateFindbar();
  if (tab.webview) tab.webview.focus();
}

function updateFindbar() {
  const tab = activeTab();
  const open = Boolean(tab && tab.find.open && tab.webview);
  dom.findbar.hidden = !open;
  if (!open) return;
  dom.findInput.value = tab.find.text;
  dom.findCount.textContent = tab.find.count;
}

function findInPage({ forward = true, next = false } = {}) {
  const tab = activeTab();
  if (!tab) return;
  if (!tab.find.open) return openFind();
  const text = tab.find.text;
  if (!text) {
    tab.find.count = '';
    dom.findCount.textContent = '';
    dom.findbar.classList.remove('no-match');
    callWebview(tab, (w) => w.stopFindInPage('clearSelection'));
    return;
  }
  callWebview(tab, (w) => w.findInPage(text, { forward, findNext: next }));
}

dom.findInput.addEventListener('input', () => {
  const tab = activeTab();
  if (!tab) return;
  tab.find.text = dom.findInput.value;
  findInPage();
});

dom.findInput.addEventListener('keydown', (event) => {
  if (event.key === 'Enter') findInPage({ forward: !event.shiftKey, next: true });
  if (event.key === 'Escape') closeFind();
});

// ============================================================================
// Zoom
// ============================================================================

function refreshZoom(tab) {
  const factor = callWebview(tab, (w) => w.getZoomFactor());
  if (factor) tab.zoom = factor;
  if (tab.id !== state.activeId) return;
  const percent = Math.round(tab.zoom * 100);
  dom.zoomBadge.hidden = percent === 100 || !tab.url;
  dom.zoomBadge.textContent = `${percent}%`;
}

function setZoom(factor) {
  const tab = activeTab();
  if (!tab || !tab.webview) return;
  callWebview(tab, (w) => w.setZoomFactor(factor));
  tab.zoom = factor;
  refreshZoom(tab);
}

function stepZoom(direction) {
  const tab = activeTab();
  if (!tab) return;
  const current = tab.zoom;
  const next = direction > 0
    ? ZOOM_STEPS.find((step) => step > current + 0.001)
    : [...ZOOM_STEPS].reverse().find((step) => step < current - 0.001);
  if (next) setZoom(next);
}

// ============================================================================
// Status bubble (link preview in the bottom-left corner)
// ============================================================================

function showStatus(url) {
  dom.statusBubble.hidden = !url;
  if (url) dom.statusBubble.textContent = url;
}

// ============================================================================
// Bookmarks bar
// ============================================================================

function renderBookmarksBar() {
  const visible = state.settings.showBookmarksBar && state.bookmarks.length > 0;
  dom.bookmarksBar.hidden = !visible;
  if (!visible) return;

  dom.bookmarksBar.replaceChildren(...state.bookmarks.map((bookmark) => {
    const chip = el('button', { class: 'bookmark', title: `${bookmark.title}\n${bookmark.url}` }, [
      faviconNode(bookmark.favicon),
      el('span', { text: bookmark.title || displayUrl(bookmark.url) })
    ]);
    chip.addEventListener('click', (event) => {
      if (event.ctrlKey || event.metaKey) createTab({ url: bookmark.url, background: true });
      else navigate(activeTab(), bookmark.url);
    });
    chip.addEventListener('auxclick', (event) => {
      if (event.button === 1) createTab({ url: bookmark.url, background: true });
    });
    chip.addEventListener('contextmenu', async (event) => {
      event.preventDefault();
      const choice = await window.aura.popupMenu([
        { id: 'open', label: 'Open in New Tab' },
        { type: 'separator' },
        { id: 'delete', label: 'Delete' }
      ], { x: event.clientX, y: event.clientY });
      if (choice === 'open') createTab({ url: bookmark.url });
      if (choice === 'delete') removeBookmark(bookmark);
    });
    return chip;
  }));
}

// ============================================================================
// Side panel: history, bookmarks, downloads
// ============================================================================

function openPanel(name) {
  state.panel = name;
  dom.panel.hidden = false;
  dom.panelSearch.value = '';
  renderPanel();
  if (name === 'history') dom.panelSearch.focus();
}

function closePanel() {
  state.panel = null;
  dom.panel.hidden = true;
}

function togglePanel(name) {
  if (state.panel === name) closePanel();
  else openPanel(name);
}

function emptyState(text) {
  return el('p', { class: 'panel-empty', text });
}

function pageRow(item, { detail, onRemove, removeTitle }) {
  const open = (event) => {
    if (event.ctrlKey || event.metaKey || event.button === 1) createTab({ url: item.url, background: true });
    else {
      navigate(activeTab(), item.url);
      closePanel();
    }
  };
  const main = el('button', { class: 'row-main', title: item.url }, [
    faviconNode(item.favicon),
    el('span', { class: 'row-text' }, [
      el('span', { class: 'row-title', text: item.title || displayUrl(item.url) }),
      el('span', { class: 'row-detail', text: detail })
    ])
  ]);
  main.addEventListener('click', open);
  main.addEventListener('auxclick', open);
  return el('div', { class: 'row' }, [
    main,
    el('button', { class: 'icon-btn small row-action', title: removeTitle, onclick: onRemove }, icon('close'))
  ]);
}

function dayLabel(time) {
  const day = new Date(time);
  const today = new Date();
  const startOf = (date) => new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  const diff = Math.round((startOf(today) - startOf(day)) / 86400000);
  if (diff === 0) return 'Today';
  if (diff === 1) return 'Yesterday';
  return day.toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric', year: 'numeric' });
}

function renderPanel() {
  const name = state.panel;
  if (!name) return;
  document.querySelectorAll('.panel-tab').forEach((button) => {
    button.classList.toggle('active', button.dataset.panel === name);
  });
  dom.panelTools.hidden = name !== 'history';

  let rows = [];
  if (name === 'history') {
    const query = dom.panelSearch.value.trim().toLowerCase();
    const entries = [...state.history]
      .sort((a, b) => b.time - a.time)
      .filter((entry) => !query || entry.url.toLowerCase().includes(query) || (entry.title || '').toLowerCase().includes(query))
      .slice(0, 300);
    let lastDay = '';
    for (const entry of entries) {
      const label = dayLabel(entry.time);
      if (label !== lastDay) {
        rows.push(el('h3', { class: 'panel-heading', text: label }));
        lastDay = label;
      }
      const time = new Date(entry.time).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
      rows.push(pageRow(entry, {
        detail: `${time} · ${hostOf(entry.url)}`,
        removeTitle: 'Remove from history',
        onRemove: () => { removeHistoryEntry(entry); renderPanel(); }
      }));
    }
    if (!rows.length) rows = [emptyState(query ? 'No matching history.' : 'Pages you visit will appear here.')];
  } else if (name === 'bookmarks') {
    rows = state.bookmarks.map((bookmark) => pageRow(bookmark, {
      detail: displayUrl(bookmark.url),
      removeTitle: 'Remove bookmark',
      onRemove: () => removeBookmark(bookmark)
    }));
    if (!rows.length) rows = [emptyState('Click the star in the address bar to bookmark a page.')];
  } else {
    rows = [...state.downloads].reverse().map(downloadRow);
    if (!rows.length) rows = [emptyState('Files you download will appear here.')];
  }
  dom.panelBody.replaceChildren(...rows);
}

function downloadStatus(download) {
  if (download.state === 'completed') return formatBytes(download.total || download.received);
  if (download.state === 'cancelled') return 'Cancelled';
  if (download.state === 'interrupted') return 'Failed';
  const progress = download.total
    ? `${formatBytes(download.received)} of ${formatBytes(download.total)}`
    : formatBytes(download.received);
  return download.paused ? `Paused · ${progress}` : progress;
}

function downloadRow(download) {
  const active = download.state === 'progressing';
  const done = download.state === 'completed';
  const percent = download.total ? Math.min(100, (download.received / download.total) * 100) : 0;

  const bar = el('span', { class: 'download-bar' }, el('span', { class: 'download-fill' }));
  bar.firstChild.style.width = `${percent}%`;

  const main = el('button', { class: 'row-main', title: download.path, disabled: !done }, [
    icon('download', 'favicon'),
    el('span', { class: 'row-text' }, [
      el('span', { class: 'row-title', text: download.filename }),
      el('span', { class: 'row-detail', text: downloadStatus(download) }),
      active ? bar : null
    ])
  ]);
  main.addEventListener('click', () => window.aura.downloadAction('open', { path: download.path }));

  const actions = [];
  if (active) {
    actions.push(el('button', {
      class: 'text-btn',
      text: download.paused ? 'Resume' : 'Pause',
      onclick: () => window.aura.downloadAction(download.paused ? 'resume' : 'pause', { id: download.id })
    }));
    actions.push(el('button', {
      class: 'icon-btn small', title: 'Cancel',
      onclick: () => window.aura.downloadAction('cancel', { id: download.id })
    }, icon('close')));
  } else {
    if (done) {
      actions.push(el('button', {
        class: 'icon-btn small', title: 'Show in folder',
        onclick: () => window.aura.downloadAction('reveal', { path: download.path })
      }, icon('folder')));
    }
    actions.push(el('button', {
      class: 'icon-btn small', title: 'Remove from list',
      onclick: () => {
        state.downloads = state.downloads.filter((item) => item.id !== download.id);
        updateDownloadsButton();
        renderPanel();
      }
    }, icon('close')));
  }
  return el('div', { class: `row download${done ? '' : ' pending'}` }, [main, ...actions]);
}

function updateDownloadsButton() {
  const active = state.downloads.filter((item) => item.state === 'progressing');
  dom.btnDownloads.hidden = state.downloads.length === 0;
  dom.btnDownloads.classList.toggle('active', active.length > 0);
  const total = active.reduce((sum, item) => sum + item.total, 0);
  const received = active.reduce((sum, item) => sum + item.received, 0);
  dom.downloadsProgress.style.width = total ? `${Math.min(100, (received / total) * 100)}%` : '0%';
}

function onDownloadUpdate(download) {
  const index = state.downloads.findIndex((item) => item.id === download.id);
  if (index >= 0) {
    state.downloads[index] = download;
  } else {
    state.downloads.push(download);
    // A link that opened a new tab only to start a download leaves it blank.
    const source = tabByWebContentsId(download.sourceId);
    if (source && !source.committed && source.openerId != null) closeTab(source, { remember: false });
    openPanel('downloads');
  }
  updateDownloadsButton();
  if (state.panel === 'downloads') renderPanel();
}

async function clearBrowsingData() {
  const choice = await window.aura.confirmClearData();
  if (choice >= 1) clearHistory();
}

// ============================================================================
// App menu
// ============================================================================

async function showAppMenu() {
  const rect = $('btn-menu').getBoundingClientRect();
  const tab = activeTab();
  const hasPage = Boolean(tab && tab.webview);
  const choice = await window.aura.popupMenu([
    { id: 'new-tab', label: 'New Tab', accelerator: 'CmdOrCtrl+T' },
    { id: 'reopen-tab', label: 'Reopen Closed Tab', accelerator: 'CmdOrCtrl+Shift+T', enabled: state.closedTabs.length > 0 },
    { type: 'separator' },
    { id: 'history', label: 'History' },
    { id: 'bookmarks', label: 'Bookmarks' },
    { id: 'downloads', label: 'Downloads' },
    { type: 'separator' },
    { id: 'zoom-in', label: 'Zoom In', accelerator: 'CmdOrCtrl+Plus', enabled: hasPage },
    { id: 'zoom-out', label: 'Zoom Out', accelerator: 'CmdOrCtrl+-', enabled: hasPage },
    { id: 'zoom-reset', label: `Actual Size (now ${Math.round((tab ? tab.zoom : 1) * 100)}%)`, accelerator: 'CmdOrCtrl+0', enabled: hasPage },
    { type: 'separator' },
    { id: 'find', label: 'Find in Page…', accelerator: 'CmdOrCtrl+F', enabled: hasPage },
    { id: 'print', label: 'Print…', accelerator: 'CmdOrCtrl+P', enabled: hasPage },
    { type: 'separator' },
    { id: 'toggle-bookmarks-bar', label: 'Show Bookmarks Bar', type: 'checkbox', checked: state.settings.showBookmarksBar },
    {
      label: 'Search Engine',
      submenu: Object.entries(SEARCH_ENGINES).map(([key, engine]) => ({
        id: `engine:${key}`, label: engine.name, type: 'radio', checked: state.settings.searchEngine === key
      }))
    },
    { type: 'separator' },
    { id: 'clear-data', label: 'Clear Browsing Data…' },
    { id: 'devtools', label: 'Developer Tools', enabled: hasPage }
  ], { x: rect.left, y: rect.bottom + 4 });

  if (!choice) return;
  if (choice.startsWith('engine:')) {
    state.settings.searchEngine = choice.slice(7);
    saveSettings();
  } else {
    runCommand(choice);
  }
}

// ============================================================================
// Commands (keyboard shortcuts arrive here from the application menu)
// ============================================================================

const commands = {
  'new-tab': () => createTab(),
  'close-tab': () => closeTab(activeTab()),
  'reopen-tab': reopenClosedTab,
  'next-tab': () => selectTabByOffset(1),
  'prev-tab': () => selectTabByOffset(-1),
  'select-tab': (index) => activateTab(index < 0 ? state.tabs[state.tabs.length - 1] : state.tabs[index]),
  'focus-url': focusAddressBar,
  back: () => goBack(),
  forward: () => goForward(),
  reload: () => reload(),
  'hard-reload': () => reload(activeTab(), { ignoreCache: true }),
  stop: () => stopLoading(),
  find: openFind,
  'find-next': () => findInPage({ forward: true, next: true }),
  'find-prev': () => findInPage({ forward: false, next: true }),
  'zoom-in': () => stepZoom(1),
  'zoom-out': () => stepZoom(-1),
  'zoom-reset': () => setZoom(1),
  bookmark: () => toggleBookmark(),
  'toggle-bookmarks-bar': () => {
    state.settings.showBookmarksBar = !state.settings.showBookmarksBar;
    saveSettings();
    renderBookmarksBar();
  },
  history: () => togglePanel('history'),
  bookmarks: () => togglePanel('bookmarks'),
  downloads: () => togglePanel('downloads'),
  'clear-data': clearBrowsingData,
  print: () => callWebview(activeTab(), (w) => w.print()),
  devtools: () => callWebview(activeTab(), (w) => (w.isDevToolsOpened() ? w.closeDevTools() : w.openDevTools()))
};

function runCommand(name, arg) {
  const handler = commands[name];
  if (handler) handler(arg);
}

// ============================================================================
// Wiring
// ============================================================================

function wireUI() {
  $('btn-new-tab').addEventListener('click', () => createTab());
  dom.btnBack.addEventListener('click', () => goBack());
  dom.btnForward.addEventListener('click', () => goForward());
  dom.btnReload.addEventListener('click', () => {
    const tab = activeTab();
    if (tab && tab.loading) stopLoading(tab);
    else reload(tab);
  });
  $('btn-star').addEventListener('click', () => toggleBookmark());
  $('btn-menu').addEventListener('click', showAppMenu);
  dom.btnDownloads.addEventListener('click', () => togglePanel('downloads'));
  dom.zoomBadge.addEventListener('click', () => setZoom(1));
  $('btn-error-reload').addEventListener('click', () => reload());

  $('btn-find-next').addEventListener('click', () => findInPage({ forward: true, next: true }));
  $('btn-find-prev').addEventListener('click', () => findInPage({ forward: false, next: true }));
  $('btn-find-close').addEventListener('click', closeFind);

  $('btn-panel-close').addEventListener('click', closePanel);
  $('btn-clear-data').addEventListener('click', clearBrowsingData);
  dom.panelSearch.addEventListener('input', renderPanel);
  document.querySelectorAll('.panel-tab').forEach((button) => {
    button.addEventListener('click', () => openPanel(button.dataset.panel));
  });

  // Double-clicking empty tab strip space opens a tab, as in Chrome.
  dom.tabs.addEventListener('dblclick', (event) => {
    if (event.target === dom.tabs) createTab();
  });

  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && state.panel && !event.defaultPrevented) closePanel();
  });

  // Files dropped on the browser UI should not replace it.
  document.addEventListener('dragover', (event) => event.preventDefault());
  document.addEventListener('drop', (event) => event.preventDefault());
}

async function start() {
  document.body.classList.add(`platform-${window.aura.platform}`);
  wireUI();

  const saved = await window.aura.loadState();
  state.history = Array.isArray(saved.history) ? saved.history.filter((entry) => entry && entry.url) : [];
  state.historyByUrl = new Map(state.history.map((entry) => [entry.url, entry]));
  state.bookmarks = Array.isArray(saved.bookmarks) ? saved.bookmarks.filter((item) => item && item.url) : [];
  Object.assign(state.settings, saved.settings);
  document.body.classList.toggle('fullscreen', Boolean(saved.fullscreen));

  window.aura.onCommand(runCommand);
  window.aura.onDownload(onDownloadUpdate);
  window.aura.onFullscreen((fullscreen) => {
    document.body.classList.toggle('fullscreen', fullscreen);
    if (!fullscreen) document.body.classList.remove('page-fullscreen');
  });
  window.aura.onOpenTab(({ url, background, openerId }) => {
    const opener = tabByWebContentsId(openerId);
    createTab({
      url,
      background: Boolean(background),
      openerId,
      index: opener ? state.tabs.indexOf(opener) + 1 : undefined
    });
  });
  window.aura.onSearch(({ query, openerId }) => {
    const opener = tabByWebContentsId(openerId);
    createTab({ url: searchUrl(query), index: opener ? state.tabs.indexOf(opener) + 1 : undefined });
  });

  renderBookmarksBar();

  // Restore the previous session; only the active tab loads right away.
  const session = saved.session && Array.isArray(saved.session.tabs) ? saved.session : { tabs: [], active: 0 };
  const restored = session.tabs.filter((tab) => tab && typeof tab.url === 'string');
  restored.forEach((tab) => createTab({ ...tab, background: true, lazy: true }));
  if (!state.tabs.length) createTab();
  else activateTab(state.tabs[Math.min(session.active || 0, state.tabs.length - 1)]);

  (saved.openUrls || []).forEach((url) => createTab({ url }));
}

start();
