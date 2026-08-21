import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowLeft,
  Check,
  ChevronDown,
  CircleHelp,
  Clock3,
  ExternalLink,
  Gauge,
  MoreHorizontal,
  Plus,
  RefreshCw,
  Settings2,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { providerById, PROVIDERS } from "./providers";
import type { AppSettings, ProviderId, SnapshotStatus, UsageSnapshot, UsageWindow } from "./types";

type View = "overview" | "settings";

const fallbackSnapshots: UsageSnapshot[] = [
  { provider: "claude", profile_name: "Personal", plan_name: "Claude Pro", windows: [{ label: "5h", used_pct: 16, resets_at: "2026-08-22T00:20:00.902233Z" }, { label: "Weekly", used_pct: 4, resets_at: "2026-08-28T14:00:00.902252Z" }], fetched_at: new Date().toISOString(), status: "fresh" },
  { provider: "codex", profile_name: "Personal", plan_name: "prolite", windows: [{ label: "5h", used_pct: 11, resets_at: "2026-08-28T14:16:17Z" }], fetched_at: new Date().toISOString(), status: "fresh" },
  { provider: "grok", profile_name: "Personal", plan_name: "SuperGrok", windows: [{ label: "Weekly", used_pct: 20, resets_at: "2026-08-26T10:22:33.387788Z" }], fetched_at: new Date().toISOString(), status: "fresh" },
];

const defaultSettings: AppSettings = {
  enabled: { claude: true, codex: true, grok: true, cursor: true, opencode: true },
  refreshSeconds: 60,
  launchAtLogin: false,
  theme: "dark",
};

function formatCountdown(reset?: string | null) {
  if (!reset) return "No reset scheduled";
  const seconds = Math.max(0, Math.round((new Date(reset).getTime() - Date.now()) / 1000));
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${Math.max(1, minutes)}m`;
}

function formatTime(value?: string | null) {
  if (!value) return "—";
  return new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(new Date(value));
}

function toneFor(value: number) {
  if (value > 90) return "critical";
  if (value > 70) return "warning";
  return "normal";
}

function useMockSnapshots() {
  const [snapshots, setSnapshots] = useState<UsageSnapshot[]>(fallbackSnapshots);
  const [refreshing, setRefreshing] = useState(false);
  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      const result = await invoke<UsageSnapshot[]>("refresh_snapshots");
      if (result?.length) setSnapshots(result);
    } catch {
      setSnapshots((current) => current.map((snapshot) => ({ ...snapshot, fetched_at: snapshot.fetched_at || new Date().toISOString() })));
    } finally {
      window.setTimeout(() => setRefreshing(false), 350);
    }
  }, []);
  return { snapshots, setSnapshots, refreshing, refresh };
}

function MiniBars({ snapshot }: { snapshot?: UsageSnapshot }) {
  const windows = snapshot?.windows ?? [];
  const value = Math.max(...windows.map((window) => window.used_pct), 0);
  const tone = toneFor(value);
  return <div className="mini-bars" aria-label={`${value}% used`}><i className={tone} style={{ height: `${Math.max(4, Math.min(16, value / 6))}px` }} /><i className={tone} style={{ height: `${Math.max(4, Math.min(16, value / 5))}px` }} /><i className={tone} style={{ height: `${Math.max(4, Math.min(16, value / 4))}px` }} /></div>;
}

function UsageBar({ window, accent }: { window: UsageWindow; accent: string }) {
  const tone = toneFor(window.used_pct);
  const fill = tone === "critical" ? "#F26D78" : tone === "warning" ? "#E6AA54" : accent;
  return <div className="usage-row">
    <div className="usage-label"><span>{window.label}</span><span className={`usage-number ${tone}`}>{Math.round(window.used_pct)}%</span></div>
    <div className="usage-track"><div className="usage-fill" style={{ width: `${Math.min(100, Math.max(0, window.used_pct))}%`, background: fill }} /></div>
    <div className="usage-meta"><span><Clock3 size={12} /> resets in {formatCountdown(window.resets_at)}</span><span>{formatTime(window.resets_at)}</span></div>
  </div>;
}

function BurnRate({ snapshot }: { snapshot: UsageSnapshot }) {
  const highest = Math.max(...snapshot.windows.map((window) => window.used_pct), 0);
  const paceMinutes = Math.max(20, Math.round((100 - highest) * 2.1));
  const h = Math.floor(paceMinutes / 60);
  const m = paceMinutes % 60;
  return <div className="pace"><span className="pace-dot" /> at this pace: limit in <strong>{h ? `${h}h ${m}m` : `${m}m`}</strong><span className="pace-muted"> · last hour</span></div>;
}

function ProfilePicker({ provider, profile, onChange }: { provider: ProviderId; profile: string; onChange: (value: string) => void }) {
  const [open, setOpen] = useState(false);
  const options = profile === "All" ? ["All", "Personal", "Work"] : [profile, "All", "Personal", "Work"].filter((item, index, list) => list.indexOf(item) === index);
  return <div className="profile-picker">
    <button className="profile-button" onClick={() => setOpen(!open)} aria-expanded={open}><span className="profile-avatar">{profile.slice(0, 1)}</span><span>{profile}</span><ChevronDown size={13} /></button>
    {open && <>
      <button className="menu-scrim" aria-label="Close profile menu" onClick={() => setOpen(false)} />
      <div className="profile-menu">{options.map((option) => <button key={option} className={option === profile ? "selected" : ""} onClick={() => { onChange(option); setOpen(false); }}><span>{option}</span>{option === profile && <Check size={14} />}</button>)}<div className="menu-divider" /><button className="menu-action" onClick={() => setOpen(false)}><Plus size={14} /> save current login</button></div>
    </>}
  </div>;
}

function ProviderCard({ providerId, snapshot, profile, onProfileChange }: { providerId: ProviderId; snapshot?: UsageSnapshot; profile: string; onProfileChange: (value: string) => void }) {
  const provider = providerById(providerId);
  if (!snapshot) return <section className="provider-card empty-card" style={{ "--accent": provider.accent, "--soft-accent": provider.softAccent } as React.CSSProperties}>
    <div className="card-top"><div className="provider-heading"><span className="provider-mark">{provider.name.slice(0, 1)}</span><div><h2>{provider.name}</h2><p>Not connected</p></div></div><MoreHorizontal size={17} className="muted-icon" /></div>
    <div className="empty-body"><div className="empty-icon"><SlidersHorizontal size={18} /></div><p>Sign in to see your live limits.</p><code>{provider.command}</code><button className="login-link">How detection works <ExternalLink size={12} /></button></div>
  </section>;
  const status: SnapshotStatus = snapshot.status;
  return <section className={`provider-card ${status !== "fresh" ? "is-stale" : ""}`} style={{ "--accent": provider.accent, "--soft-accent": provider.softAccent } as React.CSSProperties}>
    <div className="card-top"><div className="provider-heading"><span className="provider-mark">{provider.name.slice(0, 1)}</span><div><div className="title-line"><h2>{provider.name}</h2>{status === "fresh" ? <span className="fresh-pill">LIVE</span> : <span className="stale-pill">STALE · {formatTime(snapshot.fetched_at)}</span>}</div><p>{snapshot.plan_name || "Usage"}</p></div></div><MoreHorizontal size={17} className="muted-icon" /></div>
    <div className="card-controls"><ProfilePicker provider={providerId} profile={profile} onChange={onProfileChange} /><span className="card-updated">updated {formatTime(snapshot.fetched_at)}</span></div>
    <div className="usage-list">{snapshot.windows.map((window) => <UsageBar key={window.label} window={window} accent={provider.accent} />)}</div>
    <BurnRate snapshot={snapshot} />
  </section>;
}

function Settings({ settings, onSettingsChange, profiles, onDelete }: { settings: AppSettings; onSettingsChange: (value: AppSettings) => void; profiles: Record<ProviderId, string[]>; onDelete: (provider: ProviderId, profile: string) => void }) {
  const [profileProvider, setProfileProvider] = useState<ProviderId>("claude");
  return <div className="settings-view"><div className="settings-heading"><div><span className="eyebrow">PREFERENCES</span><h1>Settings</h1><p>Keep Burnrate quiet, local, and useful.</p></div></div>
    <div className="settings-section"><div className="section-title"><div><h3>Providers</h3><p>Choose what appears in your popover.</p></div><Gauge size={17} /></div><div className="settings-list">{PROVIDERS.map((provider) => <div className="setting-row" key={provider.id}><span className="setting-provider-dot" style={{ background: provider.accent }} /><div className="setting-copy"><strong>{provider.name}</strong><span>{provider.id === "cursor" || provider.id === "opencode" ? "Not detected" : "Auto detected"}</span></div><button className={`toggle ${settings.enabled[provider.id] ? "on" : ""}`} onClick={() => onSettingsChange({ ...settings, enabled: { ...settings.enabled, [provider.id]: !settings.enabled[provider.id] } })} aria-label={`Toggle ${provider.name}`}><span /></button></div>)}</div></div>
    <div className="settings-section"><div className="section-title"><div><h3>Profiles</h3><p>Credentials stay in your OS keyring.</p></div><ShieldCheck size={17} /></div><div className="profile-tabs">{PROVIDERS.map((provider) => <button key={provider.id} className={provider.id === profileProvider ? "active" : ""} onClick={() => setProfileProvider(provider.id)}>{provider.name}</button>)}</div><div className="saved-profiles">{(profiles[profileProvider] || []).map((profile) => <div className="saved-profile" key={profile}><span className="profile-avatar">{profile.slice(0, 1)}</span><span>{profile}</span>{profile !== "Personal" && <button aria-label={`Delete ${profile}`} onClick={() => onDelete(profileProvider, profile)}><Trash2 size={14} /></button>} {profile === "Personal" && <span className="current-label">CURRENT</span>}</div>)}<button className="outline-button"><Plus size={15} /> save current login as profile</button></div></div>
    <div className="settings-section compact-section"><div className="setting-row"><div className="setting-copy"><strong>Refresh interval</strong><span>Poll providers in the background</span></div><div className="segmented">{[30, 60, 300].map((seconds) => <button key={seconds} className={settings.refreshSeconds === seconds ? "active" : ""} onClick={() => onSettingsChange({ ...settings, refreshSeconds: seconds })}>{seconds < 60 ? `${seconds}s` : `${seconds / 60}m`}</button>)}</div></div><div className="setting-row"><div className="setting-copy"><strong>Launch at login</strong><span>Start minimized to the system tray</span></div><button className={`toggle ${settings.launchAtLogin ? "on" : ""}`} onClick={() => onSettingsChange({ ...settings, launchAtLogin: !settings.launchAtLogin })}><span /></button></div><div className="setting-row"><div className="setting-copy"><strong>Theme</strong><span>Dark is easy on the eyes</span></div><div className="segmented">{["dark", "light", "system"].map((theme) => <button key={theme} className={settings.theme === theme ? "active" : ""} onClick={() => onSettingsChange({ ...settings, theme: theme as AppSettings["theme"] })}>{theme}</button>)}</div></div></div>
    <div className="privacy-note"><ShieldCheck size={15} /><span><strong>Everything stays local.</strong> Burnrate talks only to provider endpoints. No telemetry, no analytics, no browser cookie extraction.</span></div>
  </div>;
}

export default function App() {
  const { snapshots, setSnapshots, refreshing, refresh } = useMockSnapshots();
  const [view, setView] = useState<View>(window.location.hash === "#settings" ? "settings" : "overview");
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const [profiles, setProfiles] = useState<Record<ProviderId, string[]>>({ claude: ["Personal"], codex: ["Personal"], grok: ["Personal"], cursor: [], opencode: [] });
  const [activeProfiles, setActiveProfiles] = useState<Record<ProviderId, string>>({ claude: "Personal", codex: "Personal", grok: "Personal", cursor: "Personal", opencode: "Personal" });
  const visibleProviders = useMemo(() => PROVIDERS.filter((provider) => settings.enabled[provider.id]), [settings.enabled]);
  useEffect(() => { document.documentElement.dataset.theme = settings.theme; }, [settings.theme]);
  useEffect(() => { void refresh(); }, [refresh]);
  useEffect(() => { const timer = window.setInterval(() => void refresh(), settings.refreshSeconds * 1000); return () => window.clearInterval(timer); }, [refresh, settings.refreshSeconds]);
  const snapshotFor = (provider: ProviderId) => snapshots.find((snapshot) => snapshot.provider === provider && (activeProfiles[provider] === "All" || snapshot.profile_name === activeProfiles[provider]));
  const changeSettings = (next: AppSettings) => setSettings(next);
  const deleteProfile = (provider: ProviderId, profile: string) => { setProfiles((current) => ({ ...current, [provider]: current[provider].filter((item) => item !== profile) })); if (activeProfiles[provider] === profile) setActiveProfiles((current) => ({ ...current, [provider]: "Personal" })); };
  return <main className="app-shell"><header className="app-header"><div className="brand"><span className="brand-glyph"><Sparkles size={14} /></span><span>Burnrate</span></div><div className="header-actions"><div className="tray-status"><span className="status-dot" />{snapshots.length ? "All systems normal" : "Waiting for logins"}</div><button className="icon-button" onClick={() => void refresh()} aria-label="Refresh" data-refreshing={refreshing}><RefreshCw size={16} /></button><button className="icon-button" onClick={() => setView(view === "settings" ? "overview" : "settings")} aria-label="Settings">{view === "settings" ? <X size={17} /> : <Settings2 size={17} />}</button></div></header>
    {view === "overview" ? <><div className="hero"><div><span className="eyebrow">TODAY · {new Intl.DateTimeFormat(undefined, { weekday: "short", month: "short", day: "numeric" }).format(new Date())}</span><h1>Usage at a glance<span className="cursor-blink">_</span></h1><p>Five providers. One calm little window.</p></div><div className="hero-meter"><div className="meter-ring"><span>{Math.round(snapshots.length ? snapshots.reduce((sum, snapshot) => sum + Math.max(...snapshot.windows.map((window) => window.used_pct), 0), 0) / snapshots.length : 0)}%</span></div><span>avg. used</span></div></div><div className="provider-rail">{visibleProviders.map((provider) => <div className="rail-item" key={provider.id}><span className="rail-label"><span className="rail-dot" style={{ background: provider.accent }} />{provider.name}</span><MiniBars snapshot={snapshotFor(provider.id)} /></div>)}</div><div className="cards">{visibleProviders.map((provider) => <ProviderCard key={provider.id} providerId={provider.id} snapshot={snapshotFor(provider.id)} profile={activeProfiles[provider.id]} onProfileChange={(profile) => setActiveProfiles((current) => ({ ...current, [provider.id]: profile }))} />)}</div><footer className="app-footer"><span><span className="footer-pulse" /> Last checked {formatTime(new Date().toISOString())}</span><button onClick={() => setView("settings")}><SlidersHorizontal size={13} /> customize</button></footer></> : <Settings settings={settings} onSettingsChange={changeSettings} profiles={profiles} onDelete={deleteProfile} />}
  </main>;
}
