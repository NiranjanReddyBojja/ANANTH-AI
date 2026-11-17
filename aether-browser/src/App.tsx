import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { api, hostOf, on, type Bookmark, type HistoryEntry } from "./lib/ipc";
import { actions, activeTab, getState, startStore, useBrowser } from "./store/browserStore";
import TitleBar, { VerticalTabs } from "./components/TitleBar";
import Omnibox from "./components/Omnibox";
import GamerSidebar from "./components/GamerSidebar";
import SettingsModal from "./components/SettingsModal";
import Icon, { IconButton } from "./components/Icon";

/**
 * The area web pages are drawn over. Its position is reported to the kernel,
 * which places the active tab's webview exactly on top of it.
 */
function ContentArea({ children }: { children?: React.ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const report = () => {
      const rect = element.getBoundingClientRect();
      void api.setLayout({ x: rect.left, y: rect.top, width: rect.width, height: rect.height });
    };
    report();
    const observer = new ResizeObserver(report);
    observer.observe(element);
    observer.observe(document.body);
    window.addEventListener("resize", report);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", report);
      // Nothing to draw over any more: hide every page.
      void api.setLayout({ x: 0, y: 0, width: 0, height: 0 });
    };
  }, []);
  return <div ref={ref} className="relative min-h-0 min-w-0 flex-1 bg-void">{children}</div>;
}

function NewTabPage() {
  const [sites, setSites] = useState<HistoryEntry[]>([]);
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([]);
  const tab = useBrowser(activeTab);
  const shield = useBrowser((s) => s.shield);
  useEffect(() => {
    void api.history().then((entries) => {
      const seen = new Set<string>();
      setSites(
        [...entries]
          .sort((a, b) => b.visit_count - a.visit_count)
          .filter((entry) => {
            const host = hostOf(entry.url);
            if (!host || seen.has(host)) return false;
            seen.add(host);
            return true;
          })
          .slice(0, 8),
      );
    });
    void api.bookmarks().then(setBookmarks);
  }, [tab?.id]);
  const open = (url: string) => tab && void api.navigate(tab.id, url);
  const tiles = [...bookmarks.slice(0, 8).map((b) => ({ url: b.url, name: b.title || hostOf(b.url) })), ...sites.map((s) => ({ url: s.url, name: hostOf(s.url).replace(/^www\./, "") }))].slice(0, 8);
  return (
    <div className="absolute inset-0 flex flex-col items-center overflow-auto px-6 pt-[14vh]">
      <h1 className="text-5xl font-semibold tracking-[0.3em] text-neon">AETHER</h1>
      <p className="mt-3 text-xs text-dim">
        {(shield?.blocked_total ?? 0).toLocaleString()} ads and trackers blocked this session · type in the bar above to begin
      </p>
      <div className="mt-10 grid w-full max-w-2xl grid-cols-4 gap-2">
        {tiles.map((tile) => (
          <button key={tile.url} title={tile.url} onClick={() => open(tile.url)} className="flex flex-col items-center gap-2 rounded-lg p-3 hover:bg-raised">
            <span className="flex h-11 w-11 items-center justify-center rounded-full bg-raised text-base font-semibold text-cyan">
              {tile.name.charAt(0).toUpperCase()}
            </span>
            <span className="max-w-full truncate text-xs text-dim">{tile.name}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function Toolbar({ focusSignal }: { focusSignal: number }) {
  const tab = useBrowser(activeTab);
  const sidebarOpen = useBrowser((s) => s.sidebarOpen);
  const verticalTabs = useBrowser((s) => s.verticalTabs);
  const splitId = useBrowser((s) => s.splitId);
  const hasPage = Boolean(tab?.url);
  const act = (action: "back" | "forward" | "reload" | "devtools") => tab && void api.action(tab.id, action);
  return (
    <div className="flex flex-none items-start gap-1 border-b border-line bg-panel px-2 py-1.5">
      <div className="flex h-8 items-center gap-0.5">
        <IconButton icon="back" title="Back (Alt+Left)" disabled={!hasPage} onClick={() => act("back")} />
        <IconButton icon="forward" title="Forward (Alt+Right)" disabled={!hasPage} onClick={() => act("forward")} />
        <IconButton icon="reload" title="Reload (Ctrl+R)" disabled={!hasPage} onClick={() => act("reload")} />
      </div>
      <Omnibox focusSignal={focusSignal} />
      <div className="flex h-8 items-center gap-0.5">
        {splitId && <IconButton icon="split" title="Close split view" active onClick={() => void api.setSplit(null)} />}
        <IconButton icon="tabs" title={verticalTabs ? "Use horizontal tabs" : "Use vertical tabs"} active={verticalTabs} onClick={() => actions.setVerticalTabs(!verticalTabs)} />
        <IconButton icon="code" title="Developer tools (F12)" disabled={!hasPage} onClick={() => act("devtools")} />
        <IconButton icon="gamepad" title="Gamer sidebar" active={sidebarOpen} onClick={actions.toggleSidebar} />
        <IconButton icon="settings" title="Settings" onClick={actions.openSettings} />
      </div>
    </div>
  );
}

export default function App() {
  const ready = useBrowser((s) => s.ready);
  const view = useBrowser((s) => s.view);
  const tab = useBrowser(activeTab);
  const verticalTabs = useBrowser((s) => s.verticalTabs);
  const sidebarOpen = useBrowser((s) => s.sidebarOpen);
  const [focusSignal, setFocusSignal] = useState(0);

  useEffect(() => {
    void startStore();
    // Shortcuts pressed while a web page has focus are forwarded by the kernel.
    const unlisten = on<string>("shortcut", (command) => {
      const current = activeTab(getState());
      if (command === "focus-url") setFocusSignal((n) => n + 1);
      if (command === "settings") actions.openSettings();
      if (command === "bookmark" && current?.url) void api.toggleBookmark(current.url, current.title);
    });
    return () => void unlisten.then((stop) => stop());
  }, []);

  // Shortcuts while focus is in the shell. (Inside a page, Chromium owns the keyboard.)
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const mod = event.ctrlKey || event.metaKey;
      const key = event.key.toLowerCase();
      const current = tab;
      const handled = (): boolean => {
        if (mod && key === "t") return void api.createTab(), true;
        if (mod && key === "w" && current) return void api.close(current.id), true;
        if (mod && key === "l") return setFocusSignal((n) => n + 1), true;
        if (mod && key === "r" && current) return void api.action(current.id, "reload"), true;
        if (mod && key === "d" && current?.url) return void api.toggleBookmark(current.url, current.title), true;
        if (mod && key === ",") return actions.openSettings(), true;
        if (event.key === "F12" && current) return void api.action(current.id, "devtools"), true;
        if (event.altKey && event.key === "ArrowLeft" && current) return void api.action(current.id, "back"), true;
        if (event.altKey && event.key === "ArrowRight" && current) return void api.action(current.id, "forward"), true;
        return false;
      };
      if (handled()) event.preventDefault();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tab]);

  if (!ready) return <div className="flex h-full items-center justify-center text-dim"><span className="spinner" /></div>;

  return (
    <div className="flex h-full flex-col">
      <TitleBar />
      {view === "settings" ? (
        <SettingsModal />
      ) : (
        <>
          <Toolbar focusSignal={focusSignal} />
          <div className="flex min-h-0 flex-1">
            {verticalTabs && <VerticalTabs />}
            <ContentArea>
              {!tab?.url && <NewTabPage />}
              {tab?.hibernated && (
                <div className="absolute inset-0 flex items-center justify-center gap-2 text-dim"><Icon name="moon" /> Waking tab…</div>
              )}
            </ContentArea>
            {sidebarOpen && <GamerSidebar />}
          </div>
        </>
      )}
    </div>
  );
}
