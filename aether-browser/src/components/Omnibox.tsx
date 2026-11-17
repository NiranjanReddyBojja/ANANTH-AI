// Unified URL / search / history / command bar with shield and HTTPS indicators.
import { useEffect, useRef, useState } from "react";
import { api, hostOf, prettyUrl, type HistoryEntry } from "../lib/ipc";
import { actions, activeTab, useBrowser } from "../store/browserStore";
import Icon, { IconButton } from "./Icon";

const COMMANDS: Record<string, { label: string; run: () => void }> = {
  settings: { label: "Open settings", run: actions.openSettings },
  sidebar: { label: "Toggle the gamer sidebar", run: actions.toggleSidebar },
  overlay: { label: "Toggle the in-game overlay", run: () => void api.toggleOverlay() },
  boost: { label: "Free memory: hibernate background tabs", run: () => void api.freeMemory() },
  split: { label: "Close split view", run: () => void api.setSplit(null) },
};

export function runCommand(name: string): boolean {
  const command = COMMANDS[name];
  if (command) command.run();
  return Boolean(command);
}

type Row = { key: string; icon: string; title: string; detail: string; input: string };

export default function Omnibox({ focusSignal }: { focusSignal: number }) {
  const tab = useBrowser(activeTab);
  const shield = useBrowser((s) => s.shield);
  const [text, setText] = useState("");
  const [editing, setEditing] = useState(false);
  const [rows, setRows] = useState<Row[]>([]);
  const [selected, setSelected] = useState(-1);
  const [bookmarked, setBookmarked] = useState(false);
  const [siteShield, setSiteShield] = useState(true);
  const input = useRef<HTMLInputElement>(null);
  const url = tab?.url ?? "";
  const host = hostOf(url);

  useEffect(() => {
    if (!editing) setText(prettyUrl(url));
  }, [url, editing, tab?.id]);

  useEffect(() => {
    if (!url) {
      setBookmarked(false);
      return;
    }
    void api.isBookmarked(url).then(setBookmarked);
    if (host) void api.shieldSiteEnabled(host).then(setSiteShield);
  }, [url, host]);

  // New tabs and Ctrl+L focus the bar.
  useEffect(() => {
    if (focusSignal > 0 || (tab && !tab.url)) {
      input.current?.focus();
      input.current?.select();
    }
  }, [focusSignal, tab?.id, tab?.url]);

  useEffect(() => {
    if (!editing || !text.trim()) {
      setRows([]);
      return;
    }
    let cancelled = false;
    const typed = text.trim();
    if (typed.startsWith(">")) {
      const query = typed.slice(1).trim().toLowerCase();
      setRows(
        Object.entries(COMMANDS)
          .filter(([name]) => name.startsWith(query))
          .map(([name, command]) => ({ key: name, icon: "bolt", title: `>${name}`, detail: command.label, input: `>${name}` })),
      );
      return;
    }
    void api.suggest(typed).then((entries: HistoryEntry[]) => {
      if (cancelled) return;
      setRows(entries.map((entry) => ({ key: entry.url, icon: "globe", title: entry.title || prettyUrl(entry.url), detail: prettyUrl(entry.url), input: entry.url })));
    });
    return () => {
      cancelled = true;
    };
  }, [text, editing]);

  useEffect(() => setSelected(-1), [rows]);

  async function go(value: string, newTab = false) {
    const target = value.trim();
    if (!target) return;
    setEditing(false);
    setRows([]);
    input.current?.blur();
    if (newTab || !tab) {
      await api.createTab(target);
      return;
    }
    const command = await api.navigate(tab.id, target);
    if (command && !runCommand(command)) setText(prettyUrl(url));
  }

  const blocked = tab ? (shield?.blocked_by_tab[tab.id] ?? 0) : 0;
  const shieldOn = Boolean(shield?.enabled) && siteShield;
  const secure = url.startsWith("https://");

  return (
    <div className="min-w-0 flex-1">
      <div className={`flex h-8 items-center gap-2 rounded-lg border bg-void px-2.5 transition-colors ${editing ? "border-cyan" : "border-line hover:border-dim"}`}>
        <span title={!url ? "" : secure ? "Connection is secure" : "Connection is not secure"} className={`flex flex-none ${!url ? "text-dim" : secure ? "text-good" : "text-neon"}`}>
          <Icon name={!url ? "search" : secure ? "lock" : "warning"} size={14} />
        </span>
        <input
          ref={input}
          value={text}
          spellCheck={false}
          autoComplete="off"
          aria-label="Address and search bar"
          placeholder="Search, enter a URL, or type > for commands"
          className="h-full min-w-0 flex-1 bg-transparent text-[13px] outline-none placeholder:text-dim"
          onChange={(event) => {
            setText(event.target.value);
            setEditing(true);
          }}
          onFocus={(event) => {
            setEditing(true);
            setText(url);
            const target = event.target;
            requestAnimationFrame(() => target.select());
          }}
          onBlur={() => {
            // Let a click on a suggestion land first.
            setTimeout(() => setEditing(false), 120);
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              event.preventDefault();
              if (!rows.length) return;
              const next = (selected + (event.key === "ArrowDown" ? 1 : -1) + rows.length + 1) % (rows.length + 1);
              setSelected(next === rows.length ? -1 : next);
            } else if (event.key === "Enter") {
              void go(selected >= 0 ? rows[selected].input : text, event.altKey);
            } else if (event.key === "Escape") {
              setEditing(false);
              setText(prettyUrl(url));
              input.current?.blur();
            }
          }}
        />
        {url && (
          <button
            title={shieldOn ? `Shield is on for ${host || "this site"} - ${blocked} blocked. Click to turn off here.` : "Shield is off here. Click to turn on."}
            aria-label="Toggle shield for this site"
            aria-pressed={shieldOn}
            disabled={!host || !shield?.enabled}
            onClick={() => {
              void api.shieldSite(host, !siteShield).then(() => setSiteShield(!siteShield));
            }}
            className={`flex h-6 flex-none items-center gap-1 rounded-md px-1.5 text-[11px] font-medium ${shieldOn ? "bg-neon/15 text-neon" : "text-dim hover:bg-raised"}`}
          >
            <Icon name="shield" size={14} />
            {shieldOn ? blocked : "off"}
          </button>
        )}
        {url && (
          <IconButton
            icon={bookmarked ? "starFilled" : "star"}
            title={bookmarked ? "Remove bookmark" : "Bookmark this page (Ctrl+D)"}
            active={bookmarked}
            size={15}
            className="!h-6 !w-6"
            onClick={() => void api.toggleBookmark(url, tab?.title ?? "").then(setBookmarked)}
          />
        )}
      </div>
      {editing && rows.length > 0 && (
        // Rendered in the page flow so it pushes the web page down instead of
        // hiding behind it.
        <ul role="listbox" className="mt-1 overflow-hidden rounded-lg border border-line bg-panel py-1">
          {rows.map((row, index) => (
            <li
              key={row.key}
              role="option"
              aria-selected={index === selected}
              onMouseDown={(event) => {
                event.preventDefault();
                void go(row.input);
              }}
              className={`flex h-8 items-center gap-3 px-3 text-[13px] ${index === selected ? "bg-raised" : "hover:bg-raised/60"}`}
            >
              <Icon name={row.icon} size={14} className="flex-none text-dim" />
              <span className="min-w-0 flex-initial truncate">{row.title}</span>
              <span className="min-w-0 flex-1 truncate text-cyan">{row.detail}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
