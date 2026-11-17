/**
 * Ananth Browser - Preload Script
 *
 * The only bridge between the browser UI and the main process. It is loaded
 * into the UI window only; web pages never see it.
 */

const { contextBridge, ipcRenderer } = require('electron');

function subscribe(channel, callback) {
  const listener = (_event, ...args) => callback(...args);
  ipcRenderer.on(channel, listener);
  return () => ipcRenderer.removeListener(channel, listener);
}

contextBridge.exposeInMainWorld('ananth', {
  platform: process.platform,

  // Persistence
  loadState: () => ipcRenderer.invoke('state:load'),
  save: (key, value) => ipcRenderer.send('store:set', key, value),
  saveSync: (key, value) => ipcRenderer.sendSync('store:set-sync', key, value),

  // Native menus and dialogs
  popupMenu: (items, position) => ipcRenderer.invoke('menu:popup', items, position),
  confirmClearData: () => ipcRenderer.invoke('data:confirm-clear'),

  // Downloads
  downloadAction: (action, payload) => ipcRenderer.send('download:action', action, payload),
  onDownload: (callback) => subscribe('download:update', callback),

  // Events from the main process
  onCommand: (callback) => subscribe('command', callback),
  onOpenTab: (callback) => subscribe('tab:open', callback),
  onSearch: (callback) => subscribe('tab:search', callback),
  onFullscreen: (callback) => subscribe('window:fullscreen', callback),

  closeWindow: () => ipcRenderer.send('window:close')
});
