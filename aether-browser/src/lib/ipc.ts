// Typed bridge to the Rust kernel.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface TabInfo {
  id: string;
  url: string;
  title: string;
  loading: boolean;
  pinned: boolean;
  group: string | null;
  hibernated: boolean;
  active: boolean;
  in_split: boolean;
  blocked: number;
}
export interface TabsSnapshot { tabs: TabInfo[]; active: string | null; split: string | null }
export interface HistoryEntry { id: number; url: string; title: string; visit_count: number; last_visit: number }
export interface Bookmark { id: number; url: string; title: string; folder: string }
export interface DownloadRecord { id: string; url: string; path: string; state: string; started_at: number }
export interface ShieldStats {
  enabled: boolean;
  blocked_total: number;
  blocked_by_tab: Record<string, number>;
  filter_lists: number;
  using_full_lists: boolean;
}
export interface Telemetry {
  ram_mb: number;
  cpu_percent: number;
  process_count: number;
  system_ram_total_mb: number;
  system_ram_used_mb: number;
  ram_limit_mb: number | null;
  enforced: boolean;
}
export interface ResourceLimits {
  ram_limit_mb: number | null;
  cpu_limit_percent: number | null;
  network_limit_kbps: number | null;
  game_mode: boolean;
  game_ram_limit_mb: number;
  game_cpu_limit_percent: number;
  game_network_limit_kbps: number | null;
}
export interface GovernorStatus {
  telemetry: Telemetry;
  limits: ResourceLimits;
  game_active: boolean;
  game: string | null;
  live_tabs: number;
}
export type WebRtcPolicy = "default" | "public_interface_only" | "disable_non_proxied_udp";
export type DohProvider = { provider: "off" | "cloudflare" | "quad9" | "google" } | { provider: "custom"; template: string };
export interface PrivacySettings {
  shields_enabled: boolean;
  https_only: boolean;
  block_third_party_cookies: boolean;
  strip_tracking_params: boolean;
  fingerprint_guard: boolean;
  do_not_track: boolean;
  webrtc: WebRtcPolicy;
  doh: DohProvider;
  safe_browsing: boolean;
}
export interface HibernationPolicy {
  enabled: boolean;
  idle_minutes: number;
  game_mode_idle_minutes: number;
  max_live_tabs: number;
  game_mode_max_live_tabs: number;
  keep_pinned: boolean;
  keep_audible: boolean;
}
export interface OverlayConfig { enabled: boolean; hotkey: string; opacity: number; width: number; height: number; url: string }
export interface Login { id: number; origin: string; username: string; updated_at: number }
export interface ExtensionInfo { id: string; name: string; version: string; manifest_version: number; description: string; path: string }
export type SearchEngine = "google" | "duck_duck_go" | "bing" | "brave";
export type GameEvent = { event: "started"; game: string } | { event: "stopped" };

const call = <T>(command: string, args?: Record<string, unknown>) => invoke<T>(command, args);

export const api = {
  tabs: () => call<TabsSnapshot>("tabs_get"),
  createTab: (input?: string, background = false) => call<string>("tab_create", { input, background }),
  navigate: (id: string, input: string) => call<string | null>("tab_navigate", { id, input }),
  activate: (id: string) => call<void>("tab_activate", { id }),
  close: (id: string) => call<void>("tab_close", { id }),
  action: (id: string, action: "back" | "forward" | "reload" | "print" | "devtools") => call<void>("tab_action", { id, action }),
  zoom: (id: string, factor: number) => call<void>("tab_zoom", { id, factor }),
  pin: (id: string, pinned: boolean) => call<void>("tab_pin", { id, pinned }),
  group: (id: string, group: string | null) => call<void>("tab_group", { id, group }),
  move: (id: string, index: number) => call<void>("tab_move", { id, index }),
  hibernate: (id: string) => call<boolean>("tab_hibernate", { id }),
  preview: (id: string) => call<string | null>("tab_preview", { id }),
  setLayout: (layout: { x: number; y: number; width: number; height: number }) => call<void>("layout_set", { layout }),
  setSplit: (id: string | null) => call<void>("split_set", { id }),

  suggest: (query: string) => call<HistoryEntry[]>("suggest", { query }),
  history: (query?: string) => call<HistoryEntry[]>("history_list", { query }),
  deleteHistory: (id: number) => call<void>("history_delete", { id }),
  clearData: (history: boolean, downloads: boolean, siteData: boolean) =>
    call<void>("browsing_data_clear", { history, downloads, siteData }),
  bookmarks: () => call<Bookmark[]>("bookmarks_list"),
  toggleBookmark: (url: string, title: string) => call<boolean>("bookmark_toggle", { url, title }),
  removeBookmark: (id: number) => call<void>("bookmark_remove", { id }),
  isBookmarked: (url: string) => call<boolean>("is_bookmarked", { url }),
  downloads: () => call<DownloadRecord[]>("downloads_list"),

  shieldStats: () => call<ShieldStats>("shield_stats"),
  shieldSite: (host: string, enabled: boolean) => call<void>("shield_site", { host, enabled }),
  shieldSiteEnabled: (host: string) => call<boolean>("shield_site_enabled", { host }),
  shieldUpdate: () => call<{ updated: string[]; failed: string[] }>("shield_update"),
  privacy: () => call<PrivacySettings>("privacy_get"),
  setPrivacy: (settings: PrivacySettings) => call<void>("privacy_set", { settings }),

  governor: () => call<GovernorStatus>("governor_status"),
  setLimits: (limits: ResourceLimits) => call<unknown>("limits_set", { limits }),
  freeMemory: () => call<number>("memory_free"),
  hibernation: () => call<HibernationPolicy>("hibernation_get"),
  setHibernation: (policy: HibernationPolicy) => call<void>("hibernation_set", { policy }),

  vaultUnlock: (masterPassword?: string) => call<void>("vault_unlock", { masterPassword }),
  vaultLock: () => call<void>("vault_lock"),
  vaultUnlocked: () => call<boolean>("vault_is_unlocked"),
  vaultList: () => call<Login[]>("vault_list"),
  vaultSave: (origin: string, username: string, password: string) => call<void>("vault_save", { origin, username, password }),
  vaultReveal: (id: number) => call<string>("vault_reveal", { id }),
  vaultDelete: (id: number) => call<void>("vault_delete", { id }),

  extensions: () => call<ExtensionInfo[]>("extensions_list"),
  installExtension: (path: string) => call<ExtensionInfo>("extension_install", { path }),
  removeExtension: (id: string) => call<void>("extension_remove", { id }),
  chromeProfiles: () => call<string[]>("chrome_profiles"),
  chromeImport: (profile: string) => call<{ history: number; bookmarks: number }>("chrome_import", { profile }),

  overlay: () => call<OverlayConfig>("overlay_get"),
  setOverlay: (config: OverlayConfig) => call<void>("overlay_set", { config }),
  toggleOverlay: () => call<void>("overlay_toggle"),
  overlayNavigate: (input: string) => call<void>("overlay_navigate", { input }),

  setting: <T>(key: string) => call<T | null>("setting_get", { key }),
  setSetting: (key: string, value: unknown) => call<void>("setting_set", { key, value }),
  searchEngine: () => call<SearchEngine>("search_engine_get"),
  windowAction: (action: "minimize" | "maximize" | "close" | "drag") => call<void>("window_action", { action }),
};

export function on<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (message) => handler(message.payload));
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return "";
  }
}

export function prettyUrl(url: string): string {
  return url.replace(/^https?:\/\//, "").replace(/^([^/?#]+)\/$/, "$1");
}
