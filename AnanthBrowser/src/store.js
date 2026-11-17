/**
 * Ananth Browser - Persistent store
 *
 * Tiny JSON-file store kept in the user data directory. One file per key,
 * written atomically (temp file + rename) and debounced so frequent updates
 * (history, session) never block the UI.
 */

const fs = require('fs');
const path = require('path');

const WRITE_DELAY_MS = 400;

class Store {
  constructor(dir, keys) {
    this.dir = dir;
    this.keys = new Set(keys);
    this.cache = new Map();
    this.timers = new Map();
    fs.mkdirSync(dir, { recursive: true });
  }

  file(key) {
    return path.join(this.dir, `${key}.json`);
  }

  has(key) {
    return this.keys.has(key);
  }

  get(key, fallback) {
    if (this.cache.has(key)) return this.cache.get(key);
    let value = fallback;
    try {
      value = JSON.parse(fs.readFileSync(this.file(key), 'utf8'));
    } catch {
      // Missing or corrupt file: fall back to the default.
    }
    this.cache.set(key, value);
    return value;
  }

  set(key, value) {
    if (!this.has(key)) return;
    this.cache.set(key, value);
    clearTimeout(this.timers.get(key));
    this.timers.set(key, setTimeout(() => this.write(key), WRITE_DELAY_MS));
  }

  write(key) {
    clearTimeout(this.timers.get(key));
    this.timers.delete(key);
    const target = this.file(key);
    const temp = `${target}.tmp`;
    try {
      fs.writeFileSync(temp, JSON.stringify(this.cache.get(key)));
      fs.renameSync(temp, target);
    } catch (error) {
      console.error(`[store] failed to write ${key}:`, error.message);
    }
  }

  /** Write everything that is still pending. Called before quitting. */
  flush() {
    for (const key of [...this.timers.keys()]) this.write(key);
  }
}

module.exports = { Store };
