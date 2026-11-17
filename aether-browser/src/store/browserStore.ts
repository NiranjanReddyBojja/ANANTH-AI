// Reactive browser state: tabs, shield statistics and hardware telemetry.
// A tiny external store, so no state library is shipped in the shell.
import { useSyncExternalStore } from "react";
import { api, on, type GameEvent, type GovernorStatus, type ShieldStats, type TabInfo, type TabsSnapshot, type Telemetry } from "../lib/ipc";

export interface BrowserState {
  tabs: TabInfo[];
  activeId: string | null;
  splitId: string | null;
  shield: ShieldStats | null;
  governor: GovernorStatus | null;
  game: string | null;
  verticalTabs: boolean;
  sidebarOpen: boolean;
  view: "browser" | "settings";
  ready: boolean;
}

let state: BrowserState = {
  tabs: [],
  activeId: null,
  splitId: null,
  shield: null,
  governor: null,
  game: null,
  verticalTabs: false,
  sidebarOpen: false,
  view: "browser",
  ready: false,
};

const listeners = new Set<() => void>();

function set(patch: Partial<BrowserState>) {
  state = { ...state, ...patch };
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useBrowser<T>(selector: (state: BrowserState) => T): T {
  return useSyncExternalStore(subscribe, () => selector(state));
}

export const getState = () => state;
export const activeTab = (s: BrowserState) => s.tabs.find((tab) => tab.id === s.activeId) ?? null;

function applyTabs(snapshot: TabsSnapshot) {
  set({ tabs: snapshot.tabs, activeId: snapshot.active, splitId: snapshot.split });
}

export const actions = {
  toggleSidebar() {
    set({ sidebarOpen: !state.sidebarOpen });
    void api.setSetting("sidebar_open", state.sidebarOpen);
  },
  setVerticalTabs(vertical: boolean) {
    set({ verticalTabs: vertical });
    void api.setSetting("vertical_tabs", vertical);
  },
  openSettings: () => set({ view: "settings" }),
  closeSettings: () => set({ view: "browser" }),
  async refreshShield() {
    set({ shield: await api.shieldStats() });
  },
  async refreshGovernor() {
    set({ governor: await api.governor() });
  },
};

let started = false;

/** Loads the initial state and subscribes to kernel events. */
export async function startStore() {
  if (started) return;
  started = true;

  void on<TabsSnapshot>("tabs-changed", applyTabs);
  void on<Telemetry>("telemetry", (telemetry) => {
    if (state.governor) set({ governor: { ...state.governor, telemetry } });
  });
  void on<GameEvent>("game-mode", (event) => {
    set({ game: event.event === "started" ? event.game : null });
    void actions.refreshGovernor();
  });
  void on("shield-updated", () => void actions.refreshShield());

  const [tabs, shield, governor, vertical, sidebar] = await Promise.all([
    api.tabs(),
    api.shieldStats(),
    api.governor(),
    api.setting<boolean>("vertical_tabs"),
    api.setting<boolean>("sidebar_open"),
  ]);
  applyTabs(tabs);
  set({
    shield,
    governor,
    game: governor.game,
    verticalTabs: vertical ?? false,
    sidebarOpen: sidebar ?? false,
    ready: true,
  });

  // Blocked-request counters change constantly; poll them gently.
  setInterval(() => void actions.refreshShield(), 3000);
}
