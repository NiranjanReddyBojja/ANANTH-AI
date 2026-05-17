/**
 * AuraBrowser - Preload Script
 * 
 * Secure bridge between renderer and main process.
 * Exposes only necessary APIs to the renderer via contextBridge.
 */

const { contextBridge, ipcRenderer } = require('electron');

// Expose protected methods that allow the renderer process to use
// IPC functions without exposing the entire ipcRenderer object
contextBridge.exposeInMainWorld('electronAPI', {
  
  /**
   * Send a chat message to LLM API with streaming
   * @param {Object} options - Chat options
   * @param {Array} options.messages - Array of message objects {role, content}
   * @param {string} options.endpoint - API endpoint URL
   * @param {string} options.model - Model name to use
   * @param {string} options.apiKey - API key for authentication
   * @returns {Promise<Object>} Result object
   */
  sendChatMessage: (options) => ipcRenderer.invoke('llm-stream', options),

  /**
   * Listen for streaming response chunks from LLM
   * @param {Function} callback - Function to call with each chunk
   * @returns {Function} Unsubscribe function
   */
  onStreamChunk: (callback) => {
    const subscription = (_event, data) => callback(data);
    ipcRenderer.on('llm-stream-chunk', subscription);
    return () => ipcRenderer.removeListener('llm-stream-chunk', subscription);
  },

  /**
   * Listen for streaming errors
   * @param {Function} callback - Function to call on error
   * @returns {Function} Unsubscribe function
   */
  onStreamError: (callback) => {
    const subscription = (_event, data) => callback(data);
    ipcRenderer.on('llm-stream-error', subscription);
    return () => ipcRenderer.removeListener('llm-stream-error', subscription);
  },

  /**
   * Request navigation to a URL
   * @param {string} url - URL to navigate to
   */
  navigateToUrl: (url) => ipcRenderer.invoke('navigate-webview', url),

  /**
   * Get current webview URL
   * @returns {Promise<string|null>} Current URL or null
   */
  getCurrentUrl: () => ipcRenderer.invoke('get-webview-url'),

  /**
   * Check if running in Electron
   * @returns {boolean} True if in Electron environment
   */
  isElectron: true,

  /**
   * Platform information
   */
  platform: process.platform
});

// Log preload script execution for debugging
console.log('[AuraBrowser] Preload script loaded successfully');
