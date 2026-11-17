// Toolbar of the compact in-game overlay window (toggled with Alt+G).
import { useEffect, useState } from "react";
import { api, prettyUrl } from "../lib/ipc";
import Icon, { IconButton } from "./Icon";

export default function OverlayHud() {
  const [text, setText] = useState("");

  useEffect(() => {
    void api.overlay().then((config) => setText(prettyUrl(config.url)));
  }, []);

  return (
    <form
      className="flex h-10 items-center gap-2 border-b border-line bg-panel px-2"
      onSubmit={(event) => {
        event.preventDefault();
        void api.overlayNavigate(text);
      }}
    >
      <Icon name="gamepad" size={16} className="flex-none text-neon" />
      <input
        value={text}
        spellCheck={false}
        aria-label="Overlay address"
        placeholder="Guide, map or stream URL"
        onChange={(event) => setText(event.target.value)}
        onFocus={(event) => event.target.select()}
        className="h-7 min-w-0 flex-1 rounded-md border border-line bg-void px-2 text-xs outline-none focus:border-cyan"
      />
      <IconButton icon="close" title="Hide overlay (Alt+G)" onClick={() => void api.toggleOverlay()} />
    </form>
  );
}
