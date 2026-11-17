// Tab strip (horizontal or vertical), tab groups, pinned tabs and window controls.
import { useState } from "react";
import { api, hostOf, prettyUrl, type TabInfo } from "../lib/ipc";
import { useBrowser } from "../store/browserStore";
import Icon, { IconButton } from "./Icon";

const GROUP_COLORS = ["#ff2d55", "#00e5ff", "#3ddc97", "#ffb020", "#b388ff", "#ff7a59"];

export function groupColor(name: string): string {
  let hash = 0;
  for (const char of name) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return GROUP_COLORS[hash % GROUP_COLORS.length];
}

export function tabLabel(tab: TabInfo): string {
  return tab.title || prettyUrl(tab.url) || "New Tab";
}

function Favicon({ tab }: { tab: TabInfo }) {
  const [failed, setFailed] = useState<string | null>(null);
  if (tab.loading) return <span className="spinner flex-none" />;
  if (tab.hibernated) return <Icon name="moon" size={13} className="flex-none text-cyan" />;
  const host = hostOf(tab.url);
  // Use the page's own origin so http:// sites don't get ERR_SSL_PROTOCOL_ERROR.
  let faviconUrl = '';
  try { faviconUrl = new URL(tab.url).origin + '/favicon.ico'; } catch { faviconUrl = ''; }
  if (!host || failed === host || !faviconUrl) return <Icon name={tab.url ? "globe" : "search"} size={13} className="flex-none text-dim" />;
  return (
    <img
      src={faviconUrl}
      alt=""
      width={14}
      height={14}
      className="flex-none rounded-sm"
      draggable={false}
      onError={() => setFailed(host)}
    />
  );
}

function Tab({ tab, vertical, onMenu }: { tab: TabInfo; vertical: boolean; onMenu: (tab: TabInfo) => void }) {
  const compact = tab.pinned && !vertical;
  return (
    <div
      role="tab"
      aria-selected={tab.active}
      title={`${tabLabel(tab)}${tab.url ? `\n${tab.url}` : ""}${tab.hibernated ? "\nHibernated - click to reload" : ""}`}
      draggable
      onDragStart={(event) => event.dataTransfer.setData("text/aether-tab", tab.id)}
      onDragOver={(event) => event.preventDefault()}
      onDrop={(event) => {
        const dragged = event.dataTransfer.getData("text/aether-tab");
        const index = Number(event.currentTarget.dataset.index);
        if (dragged && dragged !== tab.id) void api.move(dragged, index);
      }}
      onMouseDown={(event) => {
        if (event.button === 0) void api.activate(tab.id);
        if (event.button === 1) {
          event.preventDefault();
          void api.close(tab.id);
        }
      }}
      onContextMenu={(event) => {
        event.preventDefault();
        onMenu(tab);
      }}
      className={`group relative flex h-8 min-w-0 items-center gap-2 rounded-md px-2.5 text-xs transition-colors ${
        vertical ? "w-full" : compact ? "w-9 flex-none justify-center px-0" : "w-48 flex-initial"
      } ${
        tab.active
          ? "bg-raised text-ink shadow-[inset_0_-2px_0_var(--color-neon)]"
          : tab.in_split
            ? "bg-raised/60 text-ink shadow-[inset_0_-2px_0_var(--color-cyan)]"
            : "text-dim hover:bg-raised/60 hover:text-ink"
      } ${tab.hibernated ? "opacity-60" : ""}`}
    >
      {tab.group && <span className="h-4 w-1 flex-none rounded-full" style={{ background: groupColor(tab.group) }} />}
      <Favicon tab={tab} />
      {!compact && <span className="min-w-0 flex-1 truncate">{tabLabel(tab)}</span>}
      {!compact && tab.pinned && <Icon name="pin" size={11} className="flex-none text-dim" />}
      {!compact && (
        <button
          title="Close tab"
          aria-label="Close tab"
          onMouseDown={(event) => event.stopPropagation()}
          onClick={() => void api.close(tab.id)}
          className="flex h-4 w-4 flex-none items-center justify-center rounded opacity-0 hover:bg-line group-hover:opacity-100 aria-selected:opacity-100"
          aria-selected={tab.active}
        >
          <Icon name="close" size={11} />
        </button>
      )}
    </div>
  );
}

/** Actions for one tab, shown in a bar (the page itself covers any popup). */
function TabMenu({ tab, onClose }: { tab: TabInfo; onClose: () => void }) {
  const tabs = useBrowser((s) => s.tabs);
  const splitId = useBrowser((s) => s.splitId);
  const groups = [...new Set(tabs.map((t) => t.group).filter((g): g is string => Boolean(g)))];
  const [groupName, setGroupName] = useState(tab.group ?? "");
  const run = (action: Promise<unknown>) => {
    void action;
    onClose();
  };
  const button = "rounded-md px-2.5 py-1 text-dim hover:bg-raised hover:text-ink disabled:opacity-30";
  return (
    <div className="flex items-center gap-1 border-b border-line bg-panel px-3 py-1.5 text-xs">
      <span className="mr-2 max-w-56 truncate font-medium text-ink">{tabLabel(tab)}</span>
      <button className={button} onClick={() => run(api.pin(tab.id, !tab.pinned))}>{tab.pinned ? "Unpin" : "Pin"}</button>
      <button className={button} disabled={tab.active} onClick={() => run(api.setSplit(splitId === tab.id ? null : tab.id))}>
        {splitId === tab.id ? "Close split" : "Open in split view"}
      </button>
      <button className={button} disabled={tab.active || tab.hibernated || !tab.url} onClick={() => run(api.hibernate(tab.id))}>
        Hibernate
      </button>
      <button className={button} disabled={!tab.url} onClick={() => run(api.createTab(tab.url, true))}>Duplicate</button>
      <form
        className="ml-2 flex items-center gap-1"
        onSubmit={(event) => {
          event.preventDefault();
          run(api.group(tab.id, groupName.trim() || null));
        }}
      >
        <input
          value={groupName}
          onChange={(event) => setGroupName(event.target.value)}
          placeholder="Group name"
          list="aether-groups"
          aria-label="Tab group"
          className="h-6 w-32 rounded-md border border-line bg-void px-2 outline-none focus:border-cyan"
        />
        <datalist id="aether-groups">{groups.map((group) => <option key={group} value={group} />)}</datalist>
        <button className={button} type="submit">{groupName.trim() ? "Set group" : "Ungroup"}</button>
      </form>
      <span className="flex-1" />
      <button className={button} onClick={() => run(api.close(tab.id))}>Close tab</button>
      <IconButton icon="close" title="Dismiss" onClick={onClose} />
    </div>
  );
}

function useTabMenu() {
  const tabs = useBrowser((s) => s.tabs);
  const [menuId, setMenuId] = useState<string | null>(null);
  const menuTab = tabs.find((tab) => tab.id === menuId) ?? null;
  return { menuTab, open: (tab: TabInfo) => setMenuId(tab.id), close: () => setMenuId(null) };
}

function WindowControls() {
  const button = "flex h-9 w-11 items-center justify-center text-dim hover:bg-raised hover:text-ink";
  return (
    <div className="flex flex-none">
      <button className={button} title="Minimize" aria-label="Minimize" onClick={() => void api.windowAction("minimize")}><Icon name="minimize" size={14} /></button>
      <button className={button} title="Maximize" aria-label="Maximize" onClick={() => void api.windowAction("maximize")}><Icon name="maximize" size={13} /></button>
      <button className={`${button} hover:!bg-neon hover:!text-white`} title="Close" aria-label="Close" onClick={() => void api.windowAction("close")}><Icon name="close" size={15} /></button>
    </div>
  );
}

function startDrag(event: React.MouseEvent) {
  if (event.button !== 0 || event.target !== event.currentTarget) return;
  if (event.detail === 2) void api.windowAction("maximize");
  else void api.windowAction("drag");
}

/** Top bar: brand, horizontal tabs (unless vertical tabs are on) and window controls. */
export default function TitleBar() {
  const tabs = useBrowser((s) => s.tabs);
  const vertical = useBrowser((s) => s.verticalTabs);
  const game = useBrowser((s) => s.game);
  const menu = useTabMenu();
  return (
    <>
      <header className="flex h-9 flex-none items-stretch bg-void">
        <div className="flex flex-none items-center gap-1.5 pl-3 pr-2 text-[11px] font-semibold tracking-[0.2em] text-neon" onMouseDown={startDrag}>
          AETHER
        </div>
        {vertical ? (
          <div className="flex-1" onMouseDown={startDrag} />
        ) : (
          <div role="tablist" className="flex min-w-0 flex-1 items-end gap-0.5 overflow-hidden pt-1" onMouseDown={startDrag}>
            {tabs.map((tab, index) => (
              <div key={tab.id} data-index={index} className="contents">
                <Tab tab={tab} vertical={false} onMenu={menu.open} />
              </div>
            ))}
            <IconButton icon="plus" title="New tab (Ctrl+T)" onClick={() => void api.createTab()} className="mb-0.5" />
            <div className="min-w-6 flex-1 self-stretch" onMouseDown={startDrag} />
          </div>
        )}
        {game && (
          <div className="flex flex-none items-center gap-1.5 px-3 text-[11px] text-good" title="Game mode is limiting the browser">
            <Icon name="gamepad" size={14} /> GAME MODE · {game}
          </div>
        )}
        <WindowControls />
      </header>
      {!vertical && menu.menuTab && <TabMenu tab={menu.menuTab} onClose={menu.close} />}
    </>
  );
}

/** Left column of tabs, grouped, used when vertical tabs are on. */
export function VerticalTabs() {
  const tabs = useBrowser((s) => s.tabs);
  const menu = useTabMenu();
  const groups: { name: string | null; tabs: { tab: TabInfo; index: number }[] }[] = [];
  tabs.forEach((tab, index) => {
    const name = tab.pinned ? "Pinned" : tab.group;
    let group = groups.find((g) => g.name === name);
    if (!group) groups.push((group = { name, tabs: [] }));
    group.tabs.push({ tab, index });
  });
  return (
    <aside className="flex w-56 flex-none flex-col border-r border-line bg-void">
      {menu.menuTab && (
        <div className="border-b border-line p-2 text-xs">
          <div className="mb-1 truncate font-medium">{tabLabel(menu.menuTab)}</div>
          <div className="flex flex-wrap gap-1">
            {[
              [menu.menuTab.pinned ? "Unpin" : "Pin", () => api.pin(menu.menuTab!.id, !menu.menuTab!.pinned)],
              ["Split", () => api.setSplit(menu.menuTab!.id)],
              ["Hibernate", () => api.hibernate(menu.menuTab!.id)],
              ["Close", () => api.close(menu.menuTab!.id)],
            ].map(([label, run]) => (
              <button
                key={label as string}
                className="rounded bg-raised px-2 py-0.5 text-dim hover:text-ink"
                onClick={() => {
                  void (run as () => Promise<unknown>)();
                  menu.close();
                }}
              >
                {label as string}
              </button>
            ))}
          </div>
        </div>
      )}
      <div role="tablist" aria-orientation="vertical" className="flex-1 space-y-0.5 overflow-y-auto p-1.5">
        {groups.map((group) => (
          <div key={group.name ?? "ungrouped"}>
            {group.name && (
              <div className="flex items-center gap-1.5 px-2 pb-0.5 pt-2 text-[10px] font-semibold uppercase tracking-wider" style={{ color: group.name === "Pinned" ? undefined : groupColor(group.name) }}>
                {group.name}
              </div>
            )}
            {group.tabs.map(({ tab, index }) => (
              <div key={tab.id} data-index={index} className="contents">
                <Tab tab={tab} vertical onMenu={menu.open} />
              </div>
            ))}
          </div>
        ))}
      </div>
      <button className="m-1.5 flex h-8 items-center gap-2 rounded-md px-2.5 text-xs text-dim hover:bg-raised hover:text-ink" onClick={() => void api.createTab()}>
        <Icon name="plus" size={14} /> New tab
      </button>
    </aside>
  );
}
