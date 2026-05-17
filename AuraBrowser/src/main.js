/**
 * AuraBrowser - Main Process
 * 
 * Handles window creation, IPC communication, and API proxying
 * for secure LLM API interactions.
 */

const { app, BrowserWindow, ipcMain, session } = require('electron');
const path = require('path');

// Keep a global reference of the window object to prevent garbage collection
let mainWindow;

/**
 * Create the main browser window
 */
function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1400,
    height: 900,
    minWidth: 800,
    minHeight: 600,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      nodeIntegration: false, // Security: disabled
      contextIsolation: true, // Security: enabled
      webviewTag: true,       // Enable webview tag for rendering pages
      sandbox: true           // Security: enable sandbox
    },
    titleBarStyle: 'default',
    show: false, // Don't show until ready
    backgroundColor: '#1a1a2e'
  });

  // Load the renderer HTML
  mainWindow.loadFile(path.join(__dirname, 'index.html'));

  // Show window when ready to prevent visual flash
  mainWindow.once('ready-to-show', () => {
    mainWindow.show();
  });

  // Handle external links (open in default browser)
  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (url.startsWith('http')) {
      require('electron').shell.openExternal(url);
    }
    return { action: 'deny' };
  });

  // Clean up when window is closed
  mainWindow.on('closed', () => {
    mainWindow = null;
  });

  // Configure session permissions for webview
  session.defaultSession.webRequest.onHeadersReceived((details, callback) => {
    callback({
      responseHeaders: {
        ...details.responseHeaders,
        'Content-Security-Policy': ["default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob: http://localhost:* https://*"]
      }
    });
  });
}

/**
 * Initialize IPC handlers for renderer-main communication
 */
function initializeIPC() {
  
  /**
   * Handle LLM API requests from renderer
   * This proxies requests to avoid exposing API keys in renderer
   */
  ipcMain.handle('llm-chat', async (event, { messages, endpoint, model, apiKey }) => {
    try {
      const apiEndpoint = endpoint || 'http://localhost:11434/v1/chat/completions';
      
      const headers = {
        'Content-Type': 'application/json'
      };

      // Add authorization header if API key is provided (for OpenAI-compatible APIs)
      if (apiKey && apiKey.trim()) {
        headers['Authorization'] = `Bearer ${apiKey}`;
      }

      const response = await fetch(apiEndpoint, {
        method: 'POST',
        headers: headers,
        body: JSON.stringify({
          model: model || 'llama3',
          messages: messages,
          stream: true // Enable streaming
        })
      });

      if (!response.ok) {
        const errorText = await response.text();
        throw new Error(`API Error: ${response.status} - ${errorText}`);
      }

      // Return the stream reader ID for handling in renderer
      // Note: For streaming, we'll use a different approach with events
      return { success: true, streamable: true };

    } catch (error) {
      console.error('LLM API Error:', error);
      return { 
        success: false, 
        error: error.message || 'Failed to connect to LLM API' 
      };
    }
  });

  /**
   * Handle streaming LLM responses
   * Creates a streaming connection and emits chunks to renderer
   */
  ipcMain.handle('llm-stream', async (event, { messages, endpoint, model, apiKey }) => {
    try {
      const apiEndpoint = endpoint || 'http://localhost:11434/v1/chat/completions';
      
      const headers = {
        'Content-Type': 'application/json'
      };

      if (apiKey && apiKey.trim()) {
        headers['Authorization'] = `Bearer ${apiKey}`;
      }

      const response = await fetch(apiEndpoint, {
        method: 'POST',
        headers: headers,
        body: JSON.stringify({
          model: model || 'llama3',
          messages: messages,
          stream: true
        })
      });

      if (!response.ok) {
        const errorText = await response.text();
        throw new Error(`API Error: ${response.status} - ${errorText}`);
      }

      // Read the stream and send chunks to renderer
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let accumulatedContent = '';

      while (true) {
        const { done, value } = await reader.read();
        
        if (done) {
          break;
        }

        const chunk = decoder.decode(value, { stream: true });
        
        // Parse SSE (Server-Sent Events) format
        const lines = chunk.split('\n');
        for (const line of lines) {
          if (line.startsWith('data: ')) {
            const data = line.slice(6);
            if (data === '[DONE]') {
              break;
            }
            try {
              const parsed = JSON.parse(data);
              const content = parsed.choices?.[0]?.delta?.content || '';
              if (content) {
                accumulatedContent += content;
                // Send chunk to renderer
                event.sender.send('llm-stream-chunk', {
                  content: content,
                  done: false
                });
              }
            } catch (e) {
              // Skip invalid JSON
            }
          }
        }
      }

      // Send completion signal
      event.sender.send('llm-stream-chunk', {
        content: '',
        done: true,
        fullContent: accumulatedContent
      });

      return { success: true };

    } catch (error) {
      console.error('LLM Stream Error:', error);
      event.sender.send('llm-stream-error', {
        error: error.message || 'Streaming failed'
      });
      return { success: false, error: error.message };
    }
  });

  /**
   * Get current webview URL
   */
  ipcMain.handle('get-webview-url', async () => {
    if (mainWindow) {
      // This will be handled by renderer since webview is in renderer process
      return null;
    }
    return null;
  });

  /**
   * Navigate webview to URL
   */
  ipcMain.handle('navigate-webview', async (event, url) => {
    mainWindow?.webContents.send('navigate-request', url);
  });
}

/**
 * Application lifecycle events
 */
app.whenReady().then(() => {
  initializeIPC();
  createWindow();

  // On macOS, re-create window when dock icon is clicked
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

// Quit when all windows are closed (except on macOS)
app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});

// Handle any uncaught exceptions
process.on('uncaughtException', (error) => {
  console.error('Uncaught Exception:', error);
});
