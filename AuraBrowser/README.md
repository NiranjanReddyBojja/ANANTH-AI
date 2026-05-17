# AuraBrowser

An AI-powered desktop web browser with an intelligent sidepanel assistant. Built with Electron, featuring real-time LLM integration for page analysis, summarization, and interactive chat.

## Features

- **Minimalist UI**: Clean, dark-themed interface with intuitive navigation controls
- **WebView Rendering**: Full Chromium-based web page rendering
- **AI Sidepanel**: Collapsible intelligent assistant panel
- **Page Analysis**: Scrape and analyze webpage content with AI
- **Quick Actions**: 
  - Summarize page content
  - Find contradictions and issues
  - Explain technical terms
- **Streaming Responses**: Real-time AI response generation
- **Chat Interface**: Persistent conversation history with context awareness

## Project Structure

```
AuraBrowser/
├── package.json          # Project configuration and dependencies
├── src/
│   ├── main.js          # Electron main process
│   ├── preload.js       # Secure IPC bridge
│   ├── index.html       # Application HTML structure
│   ├── styles.css       # Complete styling
│   └── renderer.js      # Frontend logic and AI integration
└── README.md            # This file
```

## Prerequisites

- Node.js (v18 or higher recommended)
- npm or yarn
- For AI features, one of the following:
  - **Ollama** (local): Install from https://ollama.ai and run `ollama run llama3`
  - **OpenAI API**: Valid API key

## Installation

1. **Clone or navigate to the project directory:**
   ```bash
   cd AuraBrowser
   ```

2. **Install dependencies:**
   ```bash
   npm install
   ```

3. **Setup AI Backend (choose one):**

   **Option A: Ollama (Local, Recommended for Development)**
   ```bash
   # Install Ollama from https://ollama.ai
   ollama pull llama3
   ollama serve
   ```

   **Option B: OpenAI API**
   - Edit `src/renderer.js` and update the CONFIG object:
   ```javascript
   const CONFIG = {
     llmEndpoint: 'https://api.openai.com/v1/chat/completions',
     llmModel: 'gpt-3.5-turbo',
     apiKey: 'your-api-key-here',
     // ...
   };
   ```

4. **Run the application:**
   ```bash
   npm start
   ```

   Or for development with logging:
   ```bash
   npm run dev
   ```

## Configuration

Edit `src/renderer.js` to customize:

```javascript
const CONFIG = {
  // API endpoint (Ollama default)
  llmEndpoint: 'http://localhost:11434/v1/chat/completions',
  
  // Model name
  llmModel: 'llama3',
  
  // API key (for OpenAI or other authenticated APIs)
  apiKey: '',
  
  // Maximum characters to send for page analysis
  maxContextLength: 8000,
};
```

## Usage Guide

### Basic Navigation
- Enter URLs in the top bar and press Enter or click Go
- Use Back/Forward buttons for history navigation
- Click Refresh to reload the current page

### AI Sidepanel
- Toggle the sidepanel using the icon in the top-right
- The sidepanel can be resized by dragging (future enhancement)

### Page Analysis
1. Navigate to any webpage
2. Click **"Analyze Page"** in the sidepanel
3. The AI will scrape the content and provide analysis
4. After analysis, use quick actions:
   - **Summarize**: Get a concise summary
   - **Find Issues**: Identify contradictions or problems
   - **Explain**: Get explanations of technical terms

### Chat
- Type questions in the chat input
- Press Enter or click Send
- The AI has context about the current page after analysis
- Conversation history is maintained during the session

## API Compatibility

AuraBrowser supports any OpenAI-compatible API endpoint:

- **Ollama** (local): `http://localhost:11434/v1/chat/completions`
- **OpenAI**: `https://api.openai.com/v1/chat/completions`
- **LocalAI**: `http://localhost:8080/v1/chat/completions`
- **vLLM**: Compatible endpoints
- **Together AI**, **Groq**, etc.

## Security Notes

- **Context Isolation**: Enabled for security
- **Node Integration**: Disabled in renderer
- **Sandbox**: Enabled for webview
- **API Keys**: Never expose API keys in production; use environment variables or secure storage

For production, consider:
```javascript
// In main.js, load API key from environment
const apiKey = process.env.LLM_API_KEY || '';
```

## Troubleshooting

### "Failed to connect to LLM API"
- Ensure Ollama is running: `ollama serve`
- Check if the model is downloaded: `ollama list`
- Verify the endpoint URL in CONFIG

### "No content found on this page"
- Some websites block webview access
- Try a different website
- Check browser console for errors

### WebView not loading pages
- Check network connectivity
- Some sites may block embedding
- Review CSP settings in main.js

## Development

### Debugging
Enable detailed logging:
```bash
npm run dev
```

View console logs in:
- Renderer: DevTools (Ctrl+Shift+I)
- Main process: Terminal output

### Building for Production

To create distributable packages:

```bash
npm install --save-dev electron-builder
```

Add to `package.json`:
```json
"scripts": {
  "build": "electron-builder"
},
"build": {
  "appId": "com.aurabrowser.app",
  "mac": { "target": "dmg" },
  "win": { "target": "nsis" },
  "linux": { "target": "AppImage" }
}
```

Then run:
```bash
npm run build
```

## Future Enhancements

- [ ] Multiple tab support
- [ ] Bookmarks and history management
- [ ] Download manager
- [ ] Custom themes
- [ ] Voice input for chat
- [ ] Export chat conversations
- [ ] Plugin/extensions system
- [ ] Split-view mode
- [ ] Reading mode
- [ ] Screenshot capture

## License

MIT License - Feel free to use and modify for your projects.

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

---

**Built with ❤️ using Electron and AI**
