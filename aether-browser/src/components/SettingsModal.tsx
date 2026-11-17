// Settings: privacy and shields, search, passwords, tabs and memory, data,
// extensions, import and the overlay.
import { useEffect, useState } from "react";
import {
  api,
  prettyUrl,
  type Bookmark,
  type DownloadRecord,
  type ExtensionInfo,
  type HibernationPolicy,
  type HistoryEntry,
  type Login,
  type OverlayConfig,
  type PrivacySettings,
  type SearchEngine,
} from "../lib/ipc";
import { actions, useBrowser } from "../store/browserStore";
import { IconButton } from "./Icon";

const SECTIONS = ["Privacy & shields", "Search", "Passwords", "Tabs & memory", "History", "Bookmarks", "Downloads", "Extensions", "Import", "Overlay", "About"] as const;
type SectionName = (typeof SECTIONS)[number];

const field = "h-8 rounded-md border border-line bg-void px-2 text-xs outline-none focus:border-cyan";
const primary = "h-8 rounded-md bg-neon px-3 text-xs font-semibold text-white hover:brightness-110 disabled:opacity-40";
const secondary = "h-8 rounded-md border border-line px-3 text-xs text-dim hover:text-ink disabled:opacity-40";

function Toggle(props: { label: string; hint?: string; checked: boolean; onChange: (checked: boolean) => void }) {
  return (
    <label className="flex items-start justify-between gap-6 border-b border-line py-3">
      <span>
        <span className="block text-[13px]">{props.label}</span>
        {props.hint && <span className="mt-0.5 block text-xs text-dim">{props.hint}</span>}
      </span>
      <input type="checkbox" checked={props.checked} onChange={(event) => props.onChange(event.target.checked)} className="mt-1 h-4 w-4 flex-none accent-(--color-neon)" />
    </label>
  );
}

function Row({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-6 border-b border-line py-3">
      <span>
        <span className="block text-[13px]">{label}</span>
        {hint && <span className="mt-0.5 block text-xs text-dim">{hint}</span>}
      </span>
      <span className="flex flex-none items-center gap-2">{children}</span>
    </div>
  );
}

function Note({ children }: { children: React.ReactNode }) {
  return <p className="mt-3 rounded-md bg-raised p-3 text-xs text-dim">{children}</p>;
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="py-8 text-center text-xs text-dim">{children}</p>;
}

function Privacy() {
  const [settings, setSettings] = useState<PrivacySettings | null>(null);
  const [update, setUpdate] = useState("");
  const shield = useBrowser((s) => s.shield);
  useEffect(() => void api.privacy().then(setSettings), []);
  if (!settings) return null;
  const save = (patch: Partial<PrivacySettings>) => {
    const next = { ...settings, ...patch };
    setSettings(next);
    void api.setPrivacy(next).then(actions.refreshShield);
  };
  return (
    <>
      <Toggle label="Shield: block ads and trackers" hint="Built into the browser's network layer; no extension needed." checked={settings.shields_enabled} onChange={(v) => save({ shields_enabled: v })} />
      <Row label="Filter lists" hint={shield?.using_full_lists ? `${shield.filter_lists} lists loaded (EasyList, EasyPrivacy, uBlock Origin)` : "Using the built-in list until the full lists are downloaded"}>
        <span className="text-xs text-dim">{update}</span>
        <button className={secondary} onClick={() => {
          setUpdate("Updating…");
          void api.shieldUpdate().then(
            (report) => setUpdate(report.failed.length ? `${report.failed.length} failed` : `${report.updated.length} updated`),
            (error) => setUpdate(String(error)),
          ).then(actions.refreshShield);
        }}>Update now</button>
      </Row>
      <Toggle label="Remove tracking parameters from links" hint="Strips utm_*, fbclid, gclid and similar." checked={settings.strip_tracking_params} onChange={(v) => save({ strip_tracking_params: v })} />
      <Toggle label="Fingerprint guard" hint="Adds per-session noise to canvas, WebGL and audio read-back. Applies to newly opened tabs." checked={settings.fingerprint_guard} onChange={(v) => save({ fingerprint_guard: v })} />
      <Toggle label="Send “Do Not Track” and Global Privacy Control" checked={settings.do_not_track} onChange={(v) => save({ do_not_track: v })} />
      <Toggle label="HTTPS-only mode" hint="Upgrade to HTTPS and warn before loading sites that do not support it." checked={settings.https_only} onChange={(v) => save({ https_only: v })} />
      <Toggle label="Block third-party cookies" checked={settings.block_third_party_cookies} onChange={(v) => save({ block_third_party_cookies: v })} />
      <Toggle label="Safe Browsing" hint="Warn about dangerous sites and downloads." checked={settings.safe_browsing} onChange={(v) => save({ safe_browsing: v })} />
      <Row label="WebRTC IP handling" hint="Stops calls and voice chat from revealing your local addresses.">
        <select className={field} value={settings.webrtc} aria-label="WebRTC IP handling" onChange={(event) => save({ webrtc: event.target.value as PrivacySettings["webrtc"] })}>
          <option value="default">Chromium default</option>
          <option value="public_interface_only">Public address only</option>
          <option value="disable_non_proxied_udp">Proxy/VPN only (strictest)</option>
        </select>
      </Row>
      <Row label="Secure DNS (DNS-over-HTTPS)">
        <select className={field} value={settings.doh.provider} aria-label="Secure DNS provider" onChange={(event) => {
          const provider = event.target.value;
          save({ doh: provider === "custom" ? { provider: "custom", template: "https://" } : ({ provider } as PrivacySettings["doh"]) });
        }}>
          <option value="off">Off</option>
          <option value="cloudflare">Cloudflare</option>
          <option value="quad9">Quad9</option>
          <option value="google">Google</option>
          <option value="custom">Custom</option>
        </select>
        {settings.doh.provider === "custom" && (
          <input className={`${field} w-64`} aria-label="DNS-over-HTTPS template" value={settings.doh.template} onChange={(event) => save({ doh: { provider: "custom", template: event.target.value } })} />
        )}
      </Row>
      <Note>HTTPS-only, cookies, Safe Browsing, WebRTC and secure DNS are applied by Chromium when the browser starts; restart Aether after changing them.</Note>
    </>
  );
}

function Search() {
  const [engine, setEngine] = useState<SearchEngine>("google");
  useEffect(() => void api.searchEngine().then(setEngine), []);
  return (
    <Row label="Search engine used in the address bar">
      <select className={field} value={engine} aria-label="Search engine" onChange={(event) => {
        const next = event.target.value as SearchEngine;
        setEngine(next);
        void api.setSetting("search_engine", next);
      }}>
        <option value="google">Google</option>
        <option value="duck_duck_go">DuckDuckGo</option>
        <option value="bing">Bing</option>
        <option value="brave">Brave Search</option>
      </select>
    </Row>
  );
}

function Passwords() {
  const [unlocked, setUnlocked] = useState(false);
  const [logins, setLogins] = useState<Login[]>([]);
  const [revealed, setRevealed] = useState<Record<number, string>>({});
  const [master, setMaster] = useState("");
  const [error, setError] = useState("");
  const [draft, setDraft] = useState({ origin: "", username: "", password: "" });
  const refresh = () => void api.vaultList().then(setLogins);
  useEffect(() => {
    void api.vaultUnlocked().then(setUnlocked);
    refresh();
  }, []);
  const unlock = (password?: string) =>
    api.vaultUnlock(password).then(() => { setUnlocked(true); setError(""); setMaster(""); }, (reason) => setError(String(reason)));

  return (
    <>
      {!unlocked ? (
        <div className="border-b border-line py-3">
          <p className="text-[13px]">The vault is locked</p>
          <p className="mt-0.5 text-xs text-dim">Passwords are encrypted with AES-256-GCM. Unlock with the key kept in your operating system's keyring, or with a master password (Argon2id).</p>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <button className={primary} onClick={() => void unlock()}>Unlock with system keyring</button>
            <span className="text-xs text-dim">or</span>
            <input type="password" className={`${field} w-56`} placeholder="Master password" aria-label="Master password" value={master} onChange={(event) => setMaster(event.target.value)} />
            <button className={secondary} disabled={!master} onClick={() => void unlock(master)}>Unlock</button>
          </div>
          {error && <p className="mt-2 text-xs text-neon" role="alert">{error}</p>}
        </div>
      ) : (
        <form className="flex flex-wrap items-center gap-2 border-b border-line py-3" onSubmit={(event) => {
          event.preventDefault();
          void api.vaultSave(draft.origin, draft.username, draft.password).then(() => { setDraft({ origin: "", username: "", password: "" }); refresh(); }, (reason) => setError(String(reason)));
        }}>
          <input className={`${field} w-56`} placeholder="https://site.example" aria-label="Site" required value={draft.origin} onChange={(event) => setDraft({ ...draft, origin: event.target.value })} />
          <input className={`${field} w-40`} placeholder="User name" aria-label="User name" required value={draft.username} onChange={(event) => setDraft({ ...draft, username: event.target.value })} />
          <input type="password" className={`${field} w-40`} placeholder="Password" aria-label="Password" required value={draft.password} onChange={(event) => setDraft({ ...draft, password: event.target.value })} />
          <button className={primary} type="submit">Save</button>
          <button className={secondary} type="button" onClick={() => void api.vaultLock().then(() => { setUnlocked(false); setRevealed({}); })}>Lock vault</button>
        </form>
      )}
      {logins.length === 0 ? <Empty>No saved passwords.</Empty> : logins.map((login) => (
        <div key={login.id} className="flex items-center gap-3 border-b border-line py-2 text-xs">
          <span className="w-64 truncate">{login.origin}</span>
          <span className="w-40 truncate text-dim">{login.username}</span>
          <span className="flex-1 truncate font-mono">{revealed[login.id] ?? "••••••••"}</span>
          <button className={secondary} disabled={!unlocked} onClick={() => {
            if (revealed[login.id]) setRevealed(({ [login.id]: _, ...rest }) => rest);
            else void api.vaultReveal(login.id).then((password) => setRevealed((current) => ({ ...current, [login.id]: password })));
          }}>{revealed[login.id] ? "Hide" : "Show"}</button>
          <IconButton icon="close" title="Delete" onClick={() => void api.vaultDelete(login.id).then(refresh)} />
        </div>
      ))}
      <Note>Saving and filling passwords automatically on web pages is not implemented yet; this is a manual vault.</Note>
    </>
  );
}

function TabsMemory() {
  const [policy, setPolicy] = useState<HibernationPolicy | null>(null);
  const vertical = useBrowser((s) => s.verticalTabs);
  useEffect(() => void api.hibernation().then(setPolicy), []);
  if (!policy) return null;
  const save = (patch: Partial<HibernationPolicy>) => {
    const next = { ...policy, ...patch };
    setPolicy(next);
    void api.setHibernation(next);
  };
  const number = (key: keyof HibernationPolicy, min: number, max: number) => (
    <input type="number" min={min} max={max} className={`${field} w-20`} aria-label={key} value={policy[key] as number}
      onChange={(event) => save({ [key]: Math.max(min, Math.min(max, Number(event.target.value) || min)) })} />
  );
  return (
    <>
      <Toggle label="Vertical tabs" checked={vertical} onChange={actions.setVerticalTabs} />
      <Toggle label="Hibernate idle tabs" hint="Unloads background tabs to free memory; they reload when selected." checked={policy.enabled} onChange={(v) => save({ enabled: v })} />
      <Row label="Hibernate after" hint="Minutes a background tab may sit unused.">{number("idle_minutes", 1, 720)}</Row>
      <Row label="Keep at most this many tabs loaded" hint="0 means no limit.">{number("max_live_tabs", 0, 200)}</Row>
      <Row label="While a game is running" hint="Idle minutes and loaded-tab limit in game mode.">{number("game_mode_idle_minutes", 1, 60)}{number("game_mode_max_live_tabs", 1, 50)}</Row>
      <Toggle label="Never hibernate pinned tabs" checked={policy.keep_pinned} onChange={(v) => save({ keep_pinned: v })} />
    </>
  );
}

function History() {
  const [query, setQuery] = useState("");
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const refresh = () => void api.history(query).then(setEntries);
  useEffect(refresh, [query]);
  return (
    <>
      <div className="flex items-center gap-2 border-b border-line py-3">
        <input className={`${field} flex-1`} placeholder="Search history" aria-label="Search history" value={query} onChange={(event) => setQuery(event.target.value)} />
        <button className={secondary} onClick={() => void api.clearData(true, false, false).then(refresh)}>Clear history</button>
        <button className={secondary} onClick={() => void api.clearData(true, true, true).then(refresh)} title="History, downloads list, cookies, cache and site storage">Clear all browsing data</button>
      </div>
      {entries.length === 0 ? <Empty>No history.</Empty> : entries.map((entry) => (
        <div key={entry.id} className="group flex items-center gap-3 border-b border-line/50 py-1.5 text-xs">
          <span className="w-32 flex-none text-dim">{new Date(entry.last_visit).toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" })}</span>
          <button className="min-w-0 flex-1 truncate text-left hover:text-cyan" title={entry.url} onClick={() => { void api.createTab(entry.url); actions.closeSettings(); }}>
            {entry.title || prettyUrl(entry.url)} <span className="text-dim">· {prettyUrl(entry.url)}</span>
          </button>
          <IconButton icon="close" title="Remove from history" onClick={() => void api.deleteHistory(entry.id).then(refresh)} />
        </div>
      ))}
    </>
  );
}

function Bookmarks() {
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([]);
  const refresh = () => void api.bookmarks().then(setBookmarks);
  useEffect(refresh, []);
  if (!bookmarks.length) return <Empty>No bookmarks yet. Click the star in the address bar to add one.</Empty>;
  return bookmarks.map((bookmark) => (
    <div key={bookmark.id} className="flex items-center gap-3 border-b border-line/50 py-1.5 text-xs">
      {bookmark.folder && <span className="w-40 flex-none truncate text-dim">{bookmark.folder}</span>}
      <button className="min-w-0 flex-1 truncate text-left hover:text-cyan" title={bookmark.url} onClick={() => { void api.createTab(bookmark.url); actions.closeSettings(); }}>
        {bookmark.title || prettyUrl(bookmark.url)} <span className="text-dim">· {prettyUrl(bookmark.url)}</span>
      </button>
      <IconButton icon="close" title="Remove bookmark" onClick={() => void api.removeBookmark(bookmark.id).then(refresh)} />
    </div>
  ));
}

function Downloads() {
  const [downloads, setDownloads] = useState<DownloadRecord[]>([]);
  useEffect(() => void api.downloads().then(setDownloads), []);
  if (!downloads.length) return <Empty>No downloads.</Empty>;
  return downloads.map((download) => (
    <div key={download.id} className="flex items-center gap-3 border-b border-line/50 py-2 text-xs">
      <span className="min-w-0 flex-1 truncate" title={download.path}>{download.path.split(/[\\/]/).pop()}</span>
      <span className="w-64 truncate text-dim" title={download.url}>{prettyUrl(download.url)}</span>
      <span className={download.state === "completed" ? "text-good" : download.state === "failed" ? "text-neon" : "text-cyan"}>{download.state.replace("_", " ")}</span>
    </div>
  ));
}

function Extensions() {
  const [extensions, setExtensions] = useState<ExtensionInfo[]>([]);
  const [path, setPath] = useState("");
  const [message, setMessage] = useState("");
  const refresh = () => void api.extensions().then(setExtensions);
  useEffect(refresh, []);
  return (
    <>
      <form className="flex items-center gap-2 border-b border-line py-3" onSubmit={(event) => {
        event.preventDefault();
        void api.installExtension(path).then((info) => { setMessage(`Installed ${info.name}. Restart Aether to load it.`); setPath(""); refresh(); }, (error) => setMessage(String(error)));
      }}>
        <input className={`${field} flex-1`} placeholder="Path to a .crx file, .zip file or unpacked extension folder" aria-label="Extension path" required value={path} onChange={(event) => setPath(event.target.value)} />
        <button className={primary} type="submit">Install</button>
      </form>
      {message && <p className="py-2 text-xs text-cyan" role="status">{message}</p>}
      {extensions.length === 0 ? <Empty>No extensions installed.</Empty> : extensions.map((extension) => (
        <div key={extension.id} className="flex items-center gap-3 border-b border-line/50 py-2 text-xs">
          <span className="w-56 truncate font-medium">{extension.name}</span>
          <span className="w-16 text-dim">{extension.version}</span>
          <span className="min-w-0 flex-1 truncate text-dim">{extension.description}</span>
          <span className="text-dim">MV{extension.manifest_version}</span>
          <IconButton icon="close" title="Remove" onClick={() => void api.removeExtension(extension.id).then(refresh)} />
        </div>
      ))}
      <Note>Extensions are loaded as unpacked extensions when the browser starts. Signature verification and the Chrome Web Store are not supported, so only install extensions you trust.</Note>
    </>
  );
}

function Import() {
  const [profiles, setProfiles] = useState<string[]>([]);
  const [custom, setCustom] = useState("");
  const [message, setMessage] = useState("");
  useEffect(() => void api.chromeProfiles().then(setProfiles), []);
  const run = (profile: string) => void api.chromeImport(profile).then(
    (report) => setMessage(`Imported ${report.history.toLocaleString()} history entries and ${report.bookmarks.toLocaleString()} bookmarks.`),
    (error) => setMessage(String(error)),
  );
  return (
    <>
      {profiles.length === 0 ? <Empty>No Chrome, Edge or Brave profiles were found automatically.</Empty> : profiles.map((profile) => (
        <Row key={profile} label={profile.split(/[\\/]/).slice(-3).join(" / ")} hint={profile}>
          <button className={primary} onClick={() => run(profile)}>Import history &amp; bookmarks</button>
        </Row>
      ))}
      <form className="flex items-center gap-2 py-3" onSubmit={(event) => { event.preventDefault(); run(custom); }}>
        <input className={`${field} flex-1`} placeholder="Or enter a profile folder (the one containing History and Bookmarks)" aria-label="Profile folder" required value={custom} onChange={(event) => setCustom(event.target.value)} />
        <button className={secondary} type="submit">Import</button>
      </form>
      {message && <p className="text-xs text-cyan" role="status">{message}</p>}
      <Note>Passwords and cookies are not imported: Chrome encrypts them so that only Chrome can read them.</Note>
    </>
  );
}

function Overlay() {
  const [config, setConfig] = useState<OverlayConfig | null>(null);
  const [error, setError] = useState("");
  useEffect(() => void api.overlay().then(setConfig), []);
  if (!config) return null;
  const save = (patch: Partial<OverlayConfig>) => {
    const next = { ...config, ...patch };
    setConfig(next);
    void api.setOverlay(next).then(() => setError(""), (reason) => setError(String(reason)));
  };
  return (
    <>
      <Toggle label="In-game overlay" hint="A small always-on-top browser window for guides, maps or a stream." checked={config.enabled} onChange={(v) => save({ enabled: v })} />
      <Row label="Hotkey" hint="For example Alt+G or Ctrl+Shift+F10. Applies after a restart.">
        <input className={`${field} w-40`} aria-label="Overlay hotkey" value={config.hotkey} onChange={(event) => save({ hotkey: event.target.value })} />
      </Row>
      {error && <p className="py-1 text-xs text-neon" role="alert">{error}</p>}
      <Row label="Opacity"><input type="range" aria-label="Overlay opacity" min={0.2} max={1} step={0.05} value={config.opacity} onChange={(event) => save({ opacity: Number(event.target.value) })} /><span className="w-10 text-right text-xs tabular-nums text-cyan">{Math.round(config.opacity * 100)}%</span></Row>
      <Row label="Size"><input type="number" className={`${field} w-20`} aria-label="Overlay width" min={240} max={1920} value={config.width} onChange={(event) => save({ width: Number(event.target.value) })} /><span className="text-dim">×</span><input type="number" className={`${field} w-20`} aria-label="Overlay height" min={180} max={1440} value={config.height} onChange={(event) => save({ height: Number(event.target.value) })} /></Row>
      <Row label="Show or hide it now"><button className={secondary} onClick={() => void api.toggleOverlay()}>Toggle overlay</button></Row>
      <Note>The overlay appears over games running in borderless or windowed fullscreen. Games in exclusive fullscreen take over the display and hide every other window, including this one.</Note>
    </>
  );
}

function About() {
  return (
    <div className="space-y-2 py-3 text-xs text-dim">
      <p className="text-base font-semibold tracking-[0.2em] text-neon">AETHER 0.1.0</p>
      <p>A gaming browser built with Rust, Tauri and the Chromium Embedded Framework.</p>
      <p>Ad blocking by Brave's adblock engine with EasyList, EasyPrivacy and uBlock Origin filter lists.</p>
    </div>
  );
}

const PANELS: Record<SectionName, () => React.ReactNode> = {
  "Privacy & shields": Privacy,
  Search,
  Passwords,
  "Tabs & memory": TabsMemory,
  History,
  Bookmarks,
  Downloads,
  Extensions,
  Import,
  Overlay,
  About,
};

export default function SettingsModal() {
  const [section, setSection] = useState<SectionName>("Privacy & shields");
  const Panel = PANELS[section];
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => event.key === "Escape" && actions.closeSettings();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  return (
    <div className="flex min-h-0 flex-1 bg-panel" role="dialog" aria-label="Settings">
      <nav className="w-52 flex-none space-y-0.5 border-r border-line p-3">
        <div className="mb-3 flex items-center justify-between">
          <h1 className="text-sm font-semibold">Settings</h1>
          <IconButton icon="close" title="Close settings (Esc)" onClick={actions.closeSettings} />
        </div>
        {SECTIONS.map((name) => (
          <button key={name} aria-current={name === section} onClick={() => setSection(name)}
            className={`block h-8 w-full rounded-md px-2.5 text-left text-xs ${name === section ? "bg-neon/15 text-neon" : "text-dim hover:bg-raised hover:text-ink"}`}>
            {name}
          </button>
        ))}
      </nav>
      <main className="min-w-0 flex-1 overflow-y-auto px-8 py-5">
        <h2 className="mb-2 text-lg font-semibold">{section}</h2>
        <div className="max-w-3xl"><Panel /></div>
      </main>
    </div>
  );
}
