/**
 * AuraBrowser - Renderer Process
 * 
 * Handles UI interactions, webview navigation, chat functionality,
 * and communication with the AI backend.
 */

// ============================================
// Configuration & State
// ============================================

const CONFIG = {
  // LLM API Configuration (defaults to Ollama local endpoint)
  llmEndpoint: 'http://localhost:11434/v1/chat/completions',
  llmModel: 'llama3',
  apiKey: '', // Set via environment or UI if using OpenAI
  
  // Page analysis settings
  maxContextLength: 8000, // Maximum characters to send to LLM
  
  // UI settings
  sidepanelMinWidth: 300,
  sidepanelMaxWidth: 600,
};

// Application state
const state = {
  currentUrl: '',
  isSidepanelOpen: true,
  isStreaming: false,
  chatHistory: [],
  pageContent: '',
  connectionStatus: 'connected'
};

// ============================================
// DOM Elements
// ============================================

const elements = {
  // Navigation
  btnBack: null,
  btnForward: null,
  btnRefresh: null,
  urlInput: null,
  btnGo: null,
  
  // Sidepanel
  btnSidepanelToggle: null,
  btnCloseSidepanel: null,
  sidepanel: null,
  
  // Quick Actions
  btnAnalyzePage: null,
  btnSummarize: null,
  btnContradictions: null,
  btnExplain: null,
  
  // Chat
  chatHistory: null,
  chatInput: null,
  btnSend: null,
  loadingIndicator: null,
  
  // WebView
  webview: null,
  
  // Status
  statusText: null,
  connectionStatus: null
};

// ============================================
// Initialization
// ============================================

/**
 * Initialize the application when DOM is ready
 */
document.addEventListener('DOMContentLoaded', () => {
  console.log('[AuraBrowser] Initializing renderer process...');
  
  // Cache DOM elements
  cacheElements();
  
  // Setup event listeners
  setupEventListeners();
  
  // Setup webview listeners
  setupWebviewListeners();
  
  // Setup streaming listeners
  setupStreamingListeners();
  
  // Load default page
  navigateToUrl('https://www.example.com');
  
  updateStatus('Ready');
  console.log('[AuraBrowser] Initialization complete');
});

/**
 * Cache all DOM element references
 */
function cacheElements() {
  // Navigation
  elements.btnBack = document.getElementById('btn-back');
  elements.btnForward = document.getElementById('btn-forward');
  elements.btnRefresh = document.getElementById('btn-refresh');
  elements.urlInput = document.getElementById('url-input');
  elements.btnGo = document.getElementById('btn-go');
  
  // Sidepanel
  elements.btnSidepanelToggle = document.getElementById('btn-sidepanel-toggle');
  elements.btnCloseSidepanel = document.getElementById('btn-close-sidepanel');
  elements.sidepanel = document.getElementById('sidepanel');
  
  // Quick Actions
  elements.btnAnalyzePage = document.getElementById('btn-analyze-page');
  elements.btnSummarize = document.getElementById('btn-summarize');
  elements.btnContradictions = document.getElementById('btn-contradictions');
  elements.btnExplain = document.getElementById('btn-explain');
  
  // Chat
  elements.chatHistory = document.getElementById('chat-history');
  elements.chatInput = document.getElementById('chat-input');
  elements.btnSend = document.getElementById('btn-send');
  elements.loadingIndicator = document.getElementById('loading-indicator');
  
  // WebView
  elements.webview = document.getElementById('webview');
  
  // Status
  elements.statusText = document.getElementById('status-text');
  elements.connectionStatus = document.getElementById('connection-status');
}

// ============================================
// Event Listeners Setup
// ============================================

/**
 * Setup all event listeners
 */
function setupEventListeners() {
  // Navigation controls
  elements.btnBack.addEventListener('click', goBack);
  elements.btnForward.addEventListener('click', goForward);
  elements.btnRefresh.addEventListener('click', refreshPage);
  elements.btnGo.addEventListener('click', () => navigateFromInput());
  elements.urlInput.addEventListener('keypress', (e) => {
    if (e.key === 'Enter') navigateFromInput();
  });
  
  // Sidepanel controls
  elements.btnSidepanelToggle.addEventListener('click', toggleSidepanel);
  elements.btnCloseSidepanel.addEventListener('click', () => closeSidepanel());
  
  // Quick action buttons
  elements.btnAnalyzePage.addEventListener('click', analyzeCurrentPage);
  elements.btnSummarize.addEventListener('click', () => quickAction('summarize'));
  elements.btnContradictions.addEventListener('click', () => quickAction('contradictions'));
  elements.btnExplain.addEventListener('click', () => quickAction('explain'));
  
  // Chat controls
  elements.btnSend.addEventListener('click', sendMessage);
  elements.chatInput.addEventListener('keypress', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  });
  
  // Auto-resize chat input
  elements.chatInput.addEventListener('input', autoResizeTextarea);
}

/**
 * Setup webview-specific event listeners
 */
function setupWebviewListeners() {
  // Update URL when navigation occurs
  elements.webview.addEventListener('did-navigate', (e) => {
    updateUrlBar(e.url);
    updateStatus(`Loaded: ${e.url}`);
  });
  
  elements.webview.addEventListener('did-navigate-in-page', (e) => {
    updateUrlBar(e.url);
  });
  
  // Loading states
  elements.webview.addEventListener('did-start-loading', () => {
    updateStatus('Loading...');
    elements.btnRefresh.classList.add('spinning');
  });
  
  elements.webview.addEventListener('did-stop-loading', () => {
    if (elements.webview.getURL()) {
      updateStatus('Page loaded');
    }
    elements.btnRefresh.classList.remove('spinning');
  });
  
  // Error handling
  elements.webview.addEventListener('did-fail-load', (e) => {
    updateStatus(`Error: ${e.errorDescription}`);
    addSystemMessage(`Failed to load page: ${e.errorDescription}`);
  });
  
  // Update title
  elements.webview.addEventListener('page-title-updated', (e) => {
    if (e.title) {
      document.title = `${e.title} - AuraBrowser`;
    }
  });
}

/**
 * Setup listeners for streaming responses
 */
function setupStreamingListeners() {
  if (window.electronAPI && window.electronAPI.onStreamChunk) {
    window.electronAPI.onStreamChunk((data) => {
      handleStreamChunk(data);
    });
  }
  
  if (window.electronAPI && window.electronAPI.onStreamError) {
    window.electronAPI.onStreamError((data) => {
      handleStreamError(data);
    });
  }
}

// ============================================
// Navigation Functions
// ============================================

/**
 * Navigate to URL from input field
 */
function navigateFromInput() {
  let url = elements.urlInput.value.trim();
  
  if (!url) return;
  
  // Add protocol if missing
  if (!url.startsWith('http://') && !url.startsWith('https://')) {
    // Check if it looks like a domain
    if (url.includes('.') && !url.includes(' ')) {
      url = 'https://' + url;
    } else {
      // Treat as search query
      url = 'https://www.google.com/search?q=' + encodeURIComponent(url);
    }
  }
  
  navigateToUrl(url);
}

/**
 * Navigate webview to specified URL
 * @param {string} url - URL to navigate to
 */
function navigateToUrl(url) {
  console.log('[AuraBrowser] Navigating to:', url);
  elements.webview.src = url;
  state.currentUrl = url;
  updateUrlBar(url);
}

/**
 * Go back in history
 */
function goBack() {
  if (elements.webview.canGoBack()) {
    elements.webview.goBack();
  }
}

/**
 * Go forward in history
 */
function goForward() {
  if (elements.webview.canGoForward()) {
    elements.webview.goForward();
  }
}

/**
 * Refresh current page
 */
function refreshPage() {
  elements.webview.reload();
}

/**
 * Update URL bar with current URL
 * @param {string} url - Current URL
 */
function updateUrlBar(url) {
  state.currentUrl = url;
  elements.urlInput.value = url;
}

// ============================================
// Sidepanel Functions
// ============================================

/**
 * Toggle sidepanel visibility
 */
function toggleSidepanel() {
  if (state.isSidepanelOpen) {
    closeSidepanel();
  } else {
    openSidepanel();
  }
}

/**
 * Open the sidepanel
 */
function openSidepanel() {
  elements.sidepanel.classList.remove('closed');
  elements.btnSidepanelToggle.classList.add('active');
  state.isSidepanelOpen = true;
}

/**
 * Close the sidepanel
 */
function closeSidepanel() {
  elements.sidepanel.classList.add('closed');
  elements.btnSidepanelToggle.classList.remove('active');
  state.isSidepanelOpen = false;
}

// ============================================
// Page Analysis Functions
// ============================================

/**
 * Scrape and analyze current page content
 */
async function analyzeCurrentPage() {
  try {
    updateStatus('Analyzing page...');
    showLoading(true);
    
    // Execute JavaScript in webview to extract content
    const pageContent = await scrapePageContent();
    
    if (!pageContent || pageContent.trim().length === 0) {
      addSystemMessage('No content found on this page.');
      showLoading(false);
      updateStatus('No content to analyze');
      return;
    }
    
    state.pageContent = pageContent;
    
    // Send to AI for analysis
    const prompt = `I'm viewing a webpage. Here's the content I can see:\n\n${truncateText(pageContent, CONFIG.maxContextLength)}\n\nPlease provide a brief analysis of this content, including:\n1. Main topic/subject\n2. Key points\n3. Content type (article, product page, etc.)\n4. Any notable observations`;
    
    await sendChatRequest([{ role: 'user', content: prompt }]);
    
    updateStatus('Page analyzed');
  } catch (error) {
    console.error('[AuraBrowser] Analysis error:', error);
    addSystemMessage(`Error analyzing page: ${error.message}`);
    updateStatus('Analysis failed');
  } finally {
    showLoading(false);
  }
}

/**
 * Scrape visible text content from webview
 * @returns {Promise<string>} Extracted text content
 */
async function scrapePageContent() {
  return new Promise((resolve, reject) => {
    try {
      elements.webview.executeJavaScript(`
        (function() {
          // Get main content areas
          const selectors = [
            'article', 'main', '.content', '#content',
            '.post', '.article', 'body'
          ];
          
          let mainContent = null;
          for (const selector of selectors) {
            const el = document.querySelector(selector);
            if (el && el.innerText.trim().length > 100) {
              mainContent = el;
              break;
            }
          }
          
          // If no main content found, use body
          if (!mainContent) {
            mainContent = document.body;
          }
          
          // Extract text, remove extra whitespace
          const text = mainContent.innerText
            .replace(/\\s+/g, ' ')
            .replace(/\\n{3,}/g, '\\n\\n')
            .trim();
          
          // Also get page title
          const title = document.title || '';
          
          return {
            title: title,
            url: window.location.href,
            content: text,
            fullText: title + '\\n\\n' + text
          };
        })();
      `, true).then(resolve).catch(reject);
    } catch (error) {
      reject(error);
    }
  });
}

/**
 * Execute quick action macros
 * @param {string} action - Action type: 'summarize', 'contradictions', 'explain'
 */
async function quickAction(action) {
  if (!state.pageContent) {
    addSystemMessage('Please click "Analyze Page" first to load content.');
    return;
  }
  
  let prompt = '';
  
  switch (action) {
    case 'summarize':
      prompt = `Based on this webpage content, please provide a concise summary:\n\n${truncateText(state.pageContent, CONFIG.maxContextLength)}\n\nSummary:`;
      break;
      
    case 'contradictions':
      prompt = `Analyze this webpage content for any contradictions, logical inconsistencies, or potential issues:\n\n${truncateText(state.pageContent, CONFIG.maxContextLength)}\n\nPlease identify any problems you find:`;
      break;
      
    case 'explain':
      prompt = `Identify technical terms, jargon, or complex concepts in this webpage content and explain them in simple terms:\n\n${truncateText(state.pageContent, CONFIG.maxContextLength)}\n\nTechnical terms explained:`;
      break;
      
    default:
      console.error('[AuraBrowser] Unknown quick action:', action);
      return;
  }
  
  showLoading(true);
  await sendChatRequest([{ role: 'user', content: prompt }]);
  showLoading(false);
}

// ============================================
// Chat Functions
// ============================================

/**
 * Send user message to chat
 */
async function sendMessage() {
  const message = elements.chatInput.value.trim();
  
  if (!message || state.isStreaming) return;
  
  // Add user message to chat
  addChatMessage(message, 'user');
  elements.chatInput.value = '';
  autoResizeTextarea();
  
  // Build conversation history for context
  const messages = buildConversationHistory(message);
  
  // Send to AI
  showLoading(true);
  await sendChatRequest(messages);
}

/**
 * Build conversation history for LLM context
 * @param {string} newMessage - New user message
 * @returns {Array} Array of message objects
 */
function buildConversationHistory(newMessage) {
  const messages = [];
  
  // Add system context if we have page content
  if (state.pageContent) {
    messages.push({
      role: 'system',
      content: `You are an AI assistant helping users understand web content. The user is currently viewing: ${state.currentUrl}\n\nRelevant page content: ${truncateText(state.pageContent, 2000)}`
    });
  }
  
  // Add recent conversation history (last 10 messages)
  const recentHistory = state.chatHistory.slice(-10);
  for (const msg of recentHistory) {
    messages.push({
      role: msg.role,
      content: msg.content
    });
  }
  
  // Add new message
  messages.push({
    role: 'user',
    content: newMessage
  });
  
  return messages;
}

/**
 * Send chat request to LLM API
 * @param {Array} messages - Array of message objects
 */
async function sendChatRequest(messages) {
  if (!window.electronAPI) {
    addSystemMessage('Error: Electron API not available. Please ensure you\'re running in Electron.');
    return;
  }
  
  state.isStreaming = true;
  elements.btnSend.disabled = true;
  
  try {
    // Create placeholder for assistant response
    const assistantMessageId = addChatMessage('', 'assistant', true);
    
    // Send to main process for streaming
    await window.electronAPI.sendChatMessage({
      messages: messages,
      endpoint: CONFIG.llmEndpoint,
      model: CONFIG.llmModel,
      apiKey: CONFIG.apiKey
    });
    
  } catch (error) {
    console.error('[AuraBrowser] Chat request error:', error);
    addSystemMessage(`Error: ${error.message}`);
    state.isStreaming = false;
    elements.btnSend.disabled = false;
    showLoading(false);
  }
}

/**
 * Handle incoming stream chunk
 * @param {Object} data - Chunk data with content and done flag
 */
function handleStreamChunk(data) {
  if (data.content) {
    // Append content to last assistant message
    appendToLastMessage(data.content);
  }
  
  if (data.done) {
    // Streaming complete
    state.isStreaming = false;
    elements.btnSend.disabled = false;
    showLoading(false);
    updateStatus('Response complete');
    
    // Save complete message to history
    saveLastMessage();
  }
}

/**
 * Handle streaming error
 * @param {Object} data - Error data
 */
function handleStreamError(data) {
  console.error('[AuraBrowser] Stream error:', data);
  addSystemMessage(`Streaming error: ${data.error}`);
  state.isStreaming = false;
  elements.btnSend.disabled = false;
  showLoading(false);
  updateStatus('Error occurred');
}

// ============================================
// Chat UI Functions
// ============================================

/**
 * Add a chat message to the UI
 * @param {string} content - Message content
 * @param {string} role - 'user' or 'assistant'
 * @param {boolean} isLoading - Whether message is still loading
 * @returns {string} Message ID
 */
function addChatMessage(content, role, isLoading = false) {
  const messageId = `msg-${Date.now()}`;
  const timestamp = new Date().toLocaleTimeString();
  
  const messageDiv = document.createElement('div');
  messageDiv.id = messageId;
  messageDiv.className = `message message-${role}${isLoading ? ' loading' : ''}`;
  
  const contentDiv = document.createElement('div');
  contentDiv.className = 'message-content';
  contentDiv.textContent = content;
  
  const timestampDiv = document.createElement('div');
  timestampDiv.className = 'message-timestamp';
  timestampDiv.textContent = timestamp;
  
  messageDiv.appendChild(contentDiv);
  messageDiv.appendChild(timestampDiv);
  
  // Remove welcome message if exists
  const welcomeMsg = elements.chatHistory.querySelector('.welcome-message');
  if (welcomeMsg) {
    welcomeMsg.remove();
  }
  
  elements.chatHistory.appendChild(messageDiv);
  scrollToBottom();
  
  // Store in state if not loading
  if (!isLoading) {
    state.chatHistory.push({
      id: messageId,
      role: role,
      content: content,
      timestamp: timestamp
    });
  }
  
  return messageId;
}

/**
 * Append content to the last assistant message
 * @param {string} content - Content to append
 */
function appendToLastMessage(content) {
  const lastAssistantMsg = elements.chatHistory.querySelector('.message-assistant:last-child .message-content');
  if (lastAssistantMsg) {
    lastAssistantMsg.textContent += content;
    scrollToBottom();
  }
}

/**
 * Save the last message to history
 */
function saveLastMessage() {
  const lastAssistantMsg = elements.chatHistory.querySelector('.message-assistant:last-child');
  if (lastAssistantMsg) {
    const content = lastAssistantMsg.querySelector('.message-content').textContent;
    const timestamp = lastAssistantMsg.querySelector('.message-timestamp').textContent;
    
    state.chatHistory.push({
      id: lastAssistantMsg.id,
      role: 'assistant',
      content: content,
      timestamp: timestamp
    });
  }
}

/**
 * Add a system message (non-chat UI message)
 * @param {string} content - Message content
 */
function addSystemMessage(content) {
  const messageDiv = document.createElement('div');
  messageDiv.className = 'message message-assistant system-message';
  messageDiv.style.fontStyle = 'italic';
  messageDiv.style.opacity = '0.7';
  
  const contentDiv = document.createElement('div');
  contentDiv.className = 'message-content';
  contentDiv.textContent = `⚙️ ${content}`;
  
  messageDiv.appendChild(contentDiv);
  
  elements.chatHistory.appendChild(messageDiv);
  scrollToBottom();
}

/**
 * Scroll chat history to bottom
 */
function scrollToBottom() {
  elements.chatHistory.scrollTop = elements.chatHistory.scrollHeight;
}

/**
 * Auto-resize textarea based on content
 */
function autoResizeTextarea() {
  const textarea = elements.chatInput;
  textarea.style.height = 'auto';
  textarea.style.height = Math.min(textarea.scrollHeight, 120) + 'px';
}

// ============================================
// Utility Functions
// ============================================

/**
 * Show/hide loading indicator
 * @param {boolean} show - Whether to show loading
 */
function showLoading(show) {
  if (show) {
    elements.loadingIndicator.classList.remove('hidden');
  } else {
    elements.loadingIndicator.classList.add('hidden');
  }
}

/**
 * Update status bar text
 * @param {string} text - Status text
 */
function updateStatus(text) {
  elements.statusText.textContent = text;
}

/**
 * Truncate text to maximum length
 * @param {string} text - Text to truncate
 * @param {number} maxLength - Maximum length
 * @returns {string} Truncated text
 */
function truncateText(text, maxLength) {
  if (!text || text.length <= maxLength) {
    return text;
  }
  return text.substring(0, maxLength) + '... [truncated]';
}

/**
 * Update connection status indicator
 * @param {string} status - 'connected' or 'disconnected'
 */
function updateConnectionStatus(status) {
  state.connectionStatus = status;
  elements.connectionStatus.className = `status-indicator ${status}`;
  elements.connectionStatus.textContent = status === 'connected' ? '●' : '○';
}

// ============================================
// Cleanup
// ============================================

/**
 * Cleanup on window unload
 */
window.addEventListener('beforeunload', () => {
  console.log('[AuraBrowser] Cleaning up...');
  // Any cleanup code here
});

console.log('[AuraBrowser] Renderer script loaded');
