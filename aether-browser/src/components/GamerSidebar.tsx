// Resource limiter HUD, shield stats, quick web panels and a scratchpad.
import { useEffect, useState } from "react";
import { api, type ResourceLimits } from "../lib/ipc";
import { actions, useBrowser } from "../store/browserStore";
import Icon from "./Icon";

function Gauge({ label, value, max, unit, limit }: { label: string; value: number; max: number; unit: string; limit?: number | null }) {
  const percent = Math.min(100, max > 0 ? (value / max) * 100 : 0);
  const hot = percent > 85;
  return (
    <div>
      <div className="mb-1 flex items-baseline justify-between text-[11px]">
        <span className="font-semibold uppercase tracking-wider text-dim">{label}</span>
        <span className="tabular-nums">
          <span className={hot ? "text-neon" : "text-ink"}>{Math.round(value).toLocaleString()}</span>
          <span className="text-dim"> / {limit ? `${Math.round(max).toLocaleString()} ${unit} cap` : `${Math.round(max).toLocaleString()} ${unit}`}</span>
        </span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-line" role="meter" aria-label={label} aria-valuenow={Math.round(percent)} aria-valuemin={0} aria-valuemax={100}>
        <div className={`h-full rounded-full transition-[width] duration-500 ${hot ? "bg-neon" : "bg-cyan"}`} style={{ width: `${percent}%` }} />
      </div>
    </div>
  );
}

function LimitSlider(props: {
  label: string;
  value: number | null;
  min: number;
  max: number;
  step: number;
  format: (value: number) => string;
  onChange: (value: number | null) => void;
}) {
  const enabled = props.value !== null;
  return (
    <div>
      <label className="flex items-center justify-between text-xs">
        <span className="flex items-center gap-2">
          <input type="checkbox" checked={enabled} onChange={(event) => props.onChange(event.target.checked ? props.max : null)} className="accent-(--color-neon)" />
          {props.label}
        </span>
        <span className="tabular-nums text-cyan">{enabled ? props.format(props.value!) : "Unlimited"}</span>
      </label>
      <input
        type="range"
        aria-label={props.label}
        disabled={!enabled}
        min={props.min}
        max={props.max}
        step={props.step}
        value={props.value ?? props.max}
        onChange={(event) => props.onChange(Number(event.target.value))}
        className="mt-1 w-full disabled:opacity-30"
      />
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="border-b border-line p-3">
      <h2 className="mb-2.5 text-[10px] font-semibold uppercase tracking-[0.18em] text-dim">{title}</h2>
      <div className="space-y-3">{children}</div>
    </section>
  );
}

const PANELS = [
  { name: "Discord", url: "https://discord.com/app" },
  { name: "Twitch", url: "https://www.twitch.tv/" },
  { name: "SteamDB", url: "https://steamdb.info/" },
  { name: "YouTube", url: "https://www.youtube.com/" },
];

export default function GamerSidebar() {
  const governor = useBrowser((s) => s.governor);
  const shield = useBrowser((s) => s.shield);
  const game = useBrowser((s) => s.game);
  const [limits, setLimits] = useState<ResourceLimits | null>(null);
  const [notes, setNotes] = useState("");
  const [boost, setBoost] = useState<string | null>(null);

  useEffect(() => {
    void api.governor().then((status) => setLimits(status.limits));
    void api.setting<string>("scratchpad").then((saved) => setNotes(saved ?? ""));
  }, []);

  function update(patch: Partial<ResourceLimits>) {
    if (!limits) return;
    const next = { ...limits, ...patch };
    setLimits(next);
    void api.setLimits(next).then(actions.refreshGovernor);
  }

  const telemetry = governor?.telemetry;
  const systemGb = telemetry ? telemetry.system_ram_total_mb / 1024 : 16;
  const ramMax = Math.max(1024, Math.floor(systemGb) * 1024);

  return (
    <aside className="flex w-72 flex-none flex-col overflow-y-auto border-l border-line bg-panel" aria-label="Gamer sidebar">
      <Section title="Resource limiter">
        {telemetry && (
          <>
            <Gauge label="RAM" value={telemetry.ram_mb} max={telemetry.ram_limit_mb ?? telemetry.system_ram_total_mb} unit="MB" limit={telemetry.ram_limit_mb} />
            <Gauge label="CPU" value={telemetry.cpu_percent} max={limits?.cpu_limit_percent ?? 100} unit="%" limit={limits?.cpu_limit_percent} />
            <div className="flex justify-between text-[11px] text-dim">
              <span>{governor.live_tabs} live tabs · {telemetry.process_count} processes</span>
              <span className={game ? "text-good" : ""}>{game ? `Game: ${game}` : "No game detected"}</span>
            </div>
            {!telemetry.enforced && (
              <p className="rounded-md bg-raised p-2 text-[11px] text-dim">
                Hard limits are enforced on Windows only. On this system the gauges are live but the caps are not applied.
              </p>
            )}
          </>
        )}
        {limits && (
          <>
            <LimitSlider label="RAM limit" value={limits.ram_limit_mb} min={512} max={ramMax} step={256} format={(v) => `${(v / 1024).toFixed(1)} GB`} onChange={(v) => update({ ram_limit_mb: v })} />
            <LimitSlider label="CPU limit" value={limits.cpu_limit_percent} min={5} max={100} step={5} format={(v) => `${v}%`} onChange={(v) => update({ cpu_limit_percent: v })} />
            <LimitSlider label="Network limit" value={limits.network_limit_kbps} min={500} max={100000} step={500} format={(v) => `${(v / 1000).toFixed(1)} Mbit/s`} onChange={(v) => update({ network_limit_kbps: v })} />
            <label className="flex items-center justify-between text-xs">
              <span className="flex items-center gap-2"><Icon name="gamepad" size={14} className="text-good" /> Auto game mode</span>
              <input type="checkbox" checked={limits.game_mode} onChange={(event) => update({ game_mode: event.target.checked })} className="accent-(--color-neon)" />
            </label>
          </>
        )}
        <button
          className="flex h-8 w-full items-center justify-center gap-2 rounded-md bg-neon text-xs font-semibold text-white hover:brightness-110"
          onClick={() => void api.freeMemory().then((count) => {
            setBoost(count ? `Hibernated ${count} tab${count === 1 ? "" : "s"}` : "Nothing to free");
            setTimeout(() => setBoost(null), 3000);
          })}
        >
          <Icon name="bolt" size={14} /> {boost ?? "Free memory now"}
        </button>
      </Section>

      <Section title="Shield">
        <div className="flex items-baseline justify-between">
          <span className="text-2xl font-semibold tabular-nums text-neon">{(shield?.blocked_total ?? 0).toLocaleString()}</span>
          <span className="text-[11px] text-dim">ads &amp; trackers blocked</span>
        </div>
        <p className="text-[11px] text-dim">
          {shield?.enabled ? (shield.using_full_lists ? `${shield.filter_lists} filter lists active` : "Built-in list active; full lists download in the background") : "Shield is turned off"}
        </p>
      </Section>

      <Section title="Panels">
        <div className="grid grid-cols-2 gap-1.5">
          {PANELS.map((panel) => (
            <button key={panel.name} className="h-8 rounded-md bg-raised text-xs text-dim hover:text-ink" title={`Open ${panel.name} beside the current tab`}
              onClick={() => void api.createTab(panel.url, true).then((id) => api.setSplit(id))}>
              {panel.name}
            </button>
          ))}
        </div>
        <button className="h-8 w-full rounded-md border border-line text-xs text-dim hover:text-ink" onClick={() => void api.toggleOverlay()}>
          Toggle in-game overlay (Alt+G)
        </button>
      </Section>

      <Section title="Scratchpad">
        <textarea
          value={notes}
          aria-label="Scratchpad"
          placeholder="Builds, codes, callouts…"
          onChange={(event) => setNotes(event.target.value)}
          onBlur={() => void api.setSetting("scratchpad", notes)}
          className="h-32 w-full resize-none rounded-md border border-line bg-void p-2 text-xs outline-none focus:border-cyan"
        />
      </Section>
    </aside>
  );
}
