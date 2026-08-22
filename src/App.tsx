import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Check,
  ChevronDown,
  Clock3,
  Copy,
  ExternalLink,
  Gauge,
  Plus,
  RefreshCw,
  Settings2,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";
import { providerIcons } from "./components/icons";
import { providerById, PROVIDERS } from "./providers";
import type { AppSettings, ProviderId, SnapshotStatus, UsageSnapshot, UsageWindow } from "./types";

type View = "overview" | "settings";

interface AccountSetup {
  supported: boolean;
  pending: boolean;
  identity?: string | null;
  suggestedName: string;
  explanation?: string | null;
}

interface AddAccountResult {
  profileName: string;
  profiles: string[];
  alreadySaved: boolean;
  message: string;
}

const fallbackSnapshots: UsageSnapshot[] = [
  { provider: "claude", profile_name: "Personal", plan_name: "Claude Pro", windows: [{ label: "5h", used_pct: 16, resets_at: "2026-08-22T00:20:00.902233Z" }, { label: "Weekly", used_pct: 4, resets_at: "2026-08-28T14:00:00.902252Z" }], fetched_at: new Date().toISOString(), status: "fresh" },
  { provider: "codex", profile_name: "Personal", plan_name: "prolite", windows: [{ label: "5h", used_pct: 11, resets_at: "2026-08-28T14:16:17Z" }], fetched_at: new Date().toISOString(), status: "fresh" },
  { provider: "grok", profile_name: "Personal", plan_name: "SuperGrok", windows: [{ label: "Weekly", used_pct: 20, resets_at: "2026-08-26T10:22:33.387788Z" }], fetched_at: new Date().toISOString(), status: "fresh" },
];

const defaultSettings: AppSettings = {
  enabled: { claude: true, codex: true, grok: true, cursor: true, opencode: true },
  refreshSeconds: 60,
  launchAtLogin: false,
  startHiddenInTray: false,
  limitWarnings: true,
  warningThresholds: [50, 80],
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
  if (value >= 90) return "critical";
  if (value >= 70) return "warning";
  return "normal";
}

function formatPace(minutes: number) {
  const hours = Math.floor(minutes / 60);
  const remainder = minutes % 60;
  return hours > 0 ? `${hours}h ${remainder}m` : `${remainder}m`;
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

function ProviderMark({ providerId, muted = false }: { providerId: ProviderId; muted?: boolean }) {
  const Icon = providerIcons[providerId];
  return <span className={`provider-glyph ${muted ? "is-muted" : ""}`}><Icon size={22} /></span>;
}

function UsageBar({ window: usageWindow, accent }: { window: UsageWindow; accent: string }) {
  const [canAnimate, setCanAnimate] = useState(false);
  useEffect(() => {
    const frame = globalThis.requestAnimationFrame(() => setCanAnimate(true));
    return () => globalThis.cancelAnimationFrame(frame);
  }, []);
  const tone = toneFor(usageWindow.used_pct);
  const fill = tone === "critical" ? "#EF4444" : tone === "warning" ? "#F59E0B" : accent;
  return <div className="usage-row">
    <div className="usage-label"><span>{usageWindow.label}</span><span className={`usage-number ${tone}`}>{Math.round(usageWindow.used_pct)}%</span></div>
    <div className="usage-track"><div className={`usage-fill ${canAnimate ? "can-animate" : ""}`} style={{ width: `${Math.min(100, Math.max(0, usageWindow.used_pct))}%`, background: fill }} /></div>
    <div className="usage-meta"><span><Clock3 size={12} /> resets in {formatCountdown(usageWindow.resets_at)}</span><span>{formatTime(usageWindow.resets_at)}</span></div>
    {usageWindow.pace_limit_minutes != null && <div className="pace"><span className="pace-dot" /> at this pace: limit in <strong>{formatPace(usageWindow.pace_limit_minutes)}</strong></div>}
  </div>;
}

function ProfilePicker({ profile, profiles, onChange, onAddAccount }: { profile: string; profiles: string[]; onChange: (value: string) => void; onAddAccount: () => void }) {
  const [open, setOpen] = useState(false);
  const options = ["All", ...profiles, profile].filter((item, index, list) => list.indexOf(item) === index);
  return <div className="profile-picker">
    <button className="profile-button" onClick={() => setOpen(!open)} aria-expanded={open}><span>{profile}</span><ChevronDown size={12} /></button>
    {open && <>
      <button className="menu-scrim" aria-label="Close profile menu" onClick={() => setOpen(false)} />
      <div className="profile-menu">{options.map((option) => <button key={option} className={option === profile ? "selected" : ""} onClick={() => { onChange(option); setOpen(false); }}><span>{option}</span>{option === profile && <Check size={14} />}</button>)}<div className="menu-divider" /><button className="menu-action" onClick={() => { setOpen(false); onAddAccount(); }}><Plus size={14} /> Add account</button></div>
    </>}
  </div>;
}

function ProviderCard({ providerId, snapshots, profile, profiles, onProfileChange, onAddAccount }: { providerId: ProviderId; snapshots: UsageSnapshot[]; profile: string; profiles: string[]; onProfileChange: (value: string) => void; onAddAccount: () => void }) {
  const provider = providerById(providerId);
  const snapshot = snapshots[0];
  const profileControl = profiles.length > 1
    ? <ProfilePicker profile={profile} profiles={profiles} onChange={onProfileChange} onAddAccount={onAddAccount} />
    : <button className="add-account-button" onClick={onAddAccount} aria-label={`Add ${provider.name} account`}><Plus size={14} /></button>;
  if (!snapshot) return <section className="provider-card empty-card" style={{ "--accent": provider.accent, "--soft-accent": provider.softAccent } as React.CSSProperties}>
    <div className="card-top"><div className="provider-heading"><ProviderMark providerId={providerId} muted /><div className="provider-copy"><h2>{provider.name}</h2><p>Not connected</p></div></div>{profileControl}</div>
    <div className="empty-body"><p>Sign in to see your live limits.</p><code>{provider.command}</code><button className="login-link">How detection works <ExternalLink size={12} /></button></div>
  </section>;
  const status: SnapshotStatus = snapshot.status;
  return <section className={`provider-card ${status !== "fresh" ? "is-stale" : ""}`} style={{ "--accent": provider.accent, "--soft-accent": provider.softAccent } as React.CSSProperties}>
    <div className="card-top"><div className="provider-heading"><ProviderMark providerId={providerId} /><div className="provider-copy"><div className="title-line"><h2>{provider.name}</h2>{status === "fresh" ? <span className="fresh-pill">LIVE</span> : <span className="stale-pill">STALE SINCE {formatTime(snapshot.fetched_at)}</span>}</div><p>{profile === "All" ? `${snapshots.length} accounts` : snapshot.plan_name || "Usage"}</p></div></div>{profileControl}</div>
    {profile === "All" ? <div className="all-profiles">{snapshots.map((item) => <div className="all-profile" key={item.profile_name}><div className="all-profile-name">{item.profile_name}</div><div className="usage-list">{item.windows.map((window) => <UsageBar key={`${item.profile_name}-${window.label}`} window={window} accent={provider.accent} />)}</div>{item.error_message && <div className="rate-limit-line"><Clock3 size={11} />{item.error_message}</div>}</div>)}</div> : <><div className="usage-list">{snapshot.windows.map((window) => <UsageBar key={window.label} window={window} accent={provider.accent} />)}</div>{snapshot.error_message && <div className="rate-limit-line"><Clock3 size={11} />{snapshot.error_message}</div>}</>}
  </section>;
}

function AddAccountDialog({ providerId, onClose, onComplete }: { providerId: ProviderId; onClose: () => void; onComplete: (result: AddAccountResult) => void }) {
  const provider = providerById(providerId);
  const [setup, setSetup] = useState<AccountSetup | null>(null);
  const [name, setName] = useState("");
  const [step, setStep] = useState<1 | 2 | 3>(1);
  const [result, setResult] = useState<AddAccountResult | null>(null);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    void invoke<AccountSetup>("get_account_setup", { provider: providerId }).then((value) => {
      setSetup(value);
      setName(value.suggestedName || "Account");
      if (value.pending) setStep(2);
    }).catch((reason) => setError(String(reason)));
  }, [providerId]);
  const begin = async () => {
    setWorking(true);
    setError("");
    try {
      const value = await invoke<AccountSetup>("begin_add_account", { provider: providerId, name: name.trim() });
      setSetup(value);
      setStep(2);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setWorking(false);
    }
  };
  const detect = async () => {
    setWorking(true);
    setError("");
    try {
      const value = await invoke<AddAccountResult>("detect_new_account", { provider: providerId });
      setResult(value);
      onComplete(value);
      setStep(3);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setWorking(false);
    }
  };
  const cancelPending = async () => {
    setWorking(true);
    setError("");
    try {
      await invoke("cancel_add_account", { provider: providerId });
      onClose();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setWorking(false);
    }
  };
  return <div className="dialog-layer" role="presentation"><section className="account-dialog" role="dialog" aria-modal="true" aria-labelledby="account-dialog-title">
    <div className="dialog-heading"><div className="dialog-mark"><ProviderMark providerId={providerId} /></div><div><span className="eyebrow">ADD ACCOUNT</span><h2 id="account-dialog-title">{provider.name}</h2></div>{step !== 2 && <button className="icon-button" onClick={onClose} aria-label="Close"><X size={15} /></button>}</div>
    {!setup && !error && <div className="dialog-loading"><RefreshCw size={16} /> Reading current CLI login…</div>}
    {setup && !setup.supported && <div className="dialog-step"><p>{setup.explanation}</p><button className="dialog-primary" onClick={onClose}>Got it</button></div>}
    {setup?.supported && step === 1 && <div className="dialog-step"><span className="step-label">STEP 1 OF 2</span><p>Your current {provider.name} login will be saved as a profile first.</p><label className="dialog-field"><span>Profile name</span><input autoFocus value={name} maxLength={48} onChange={(event) => setName(event.target.value)} /></label>{setup.identity && <span className="detected-identity">Detected: {setup.identity}</span>}<button className="dialog-primary" disabled={!name.trim() || working} onClick={() => void begin()}>{working ? "Saving…" : "Save current login"}</button></div>}
    {setup?.supported && step === 2 && <div className="dialog-step"><span className="step-label">STEP 2 OF 2</span><p>Now run <code>{provider.command}</code> in a terminal and sign into the other account.</p><div className="command-copy"><code>{provider.command}</code><button onClick={() => { void navigator.clipboard.writeText(provider.command); setCopied(true); }} aria-label="Copy login command"><Copy size={14} />{copied ? "Copied" : "Copy"}</button></div><p className="dialog-safety">Monitoring is paused for {provider.name} until detection finishes, so neither login can be refreshed from the wrong credential source.</p><button className="dialog-primary" disabled={working} onClick={() => void detect()}>{working ? "Detecting…" : "Detect new login"}</button><button className="dialog-secondary" disabled={working} onClick={() => void cancelPending()}>Cancel safely</button></div>}
    {step === 3 && result && <div className="dialog-step dialog-success"><span className="success-mark"><Check size={18} /></span><h3>{result.message}</h3><p>{result.profileName} is ready. Every distinct account polls independently; duplicate references still share one request.</p><button className="dialog-primary" onClick={onClose}>Done</button></div>}
    {error && <div className="dialog-error">{error}</div>}
  </section></div>;
}

function Settings({ settings, onSettingsChange, profiles, onDelete, onSave, onSaveManual }: { settings: AppSettings; onSettingsChange: (value: AppSettings) => void; profiles: Record<ProviderId, string[]>; onDelete: (provider: ProviderId, profile: string) => void; onSave: (provider: ProviderId, profile: string) => void; onSaveManual: (provider: ProviderId, value: string) => void }) {
  const [profileProvider, setProfileProvider] = useState<ProviderId>("claude");
  const [adding, setAdding] = useState(false);
  const [profileName, setProfileName] = useState("");
  const [manualProvider, setManualProvider] = useState<ProviderId>("cursor");
  const [manualValue, setManualValue] = useState("");
  const [manualSaved, setManualSaved] = useState(false);
  const hiddenLocation = /Macintosh|Mac OS X/.test(navigator.userAgent) ? "menu bar" : "tray";
  return <div className="settings-view"><div className="settings-heading"><div><span className="eyebrow">PREFERENCES</span><h1>Settings</h1><p>Keep Burnrate quiet, local, and useful.</p></div></div>
    <div className="settings-section"><div className="section-title"><div><h3>Providers</h3><p>Choose what appears in your popover.</p></div><Gauge size={17} /></div><div className="settings-list">{PROVIDERS.map((provider) => <div className="setting-row" key={provider.id}><ProviderMark providerId={provider.id} muted={!settings.enabled[provider.id]} /><div className="setting-copy"><strong>{provider.name}</strong><span>{provider.id === "cursor" || provider.id === "opencode" ? "Not detected" : "Auto detected"}</span></div><button className={`toggle ${settings.enabled[provider.id] ? "on" : ""}`} onClick={() => onSettingsChange({ ...settings, enabled: { ...settings.enabled, [provider.id]: !settings.enabled[provider.id] } })} aria-label={`Toggle ${provider.name}`}><span /></button></div>)}</div></div>
    <div className="settings-section"><div className="section-title"><div><h3>Profiles</h3><p>Profiles reference a credential source; OAuth tokens are never copied.</p></div><ShieldCheck size={17} /></div><div className="profile-tabs">{PROVIDERS.map((provider) => <button key={provider.id} className={provider.id === profileProvider ? "active" : ""} onClick={() => setProfileProvider(provider.id)}>{provider.name}</button>)}</div>{(["claude", "codex", "grok"] as ProviderId[]).includes(profileProvider) && <p className="profile-source-note">This CLI supports one active login per config directory. Labels on the same source share that login and one poll.</p>}<div className="saved-profiles">{(profiles[profileProvider] || []).map((profile) => <div className="saved-profile" key={profile}><span className="profile-avatar">{profile.slice(0, 1)}</span><span>{profile}</span>{profile !== "Personal" && <button aria-label={`Delete ${profile}`} onClick={() => onDelete(profileProvider, profile)}><Trash2 size={14} /></button>} {profile === "Personal" && <span className="current-label">CURRENT</span>}</div>)}{adding ? <div className="profile-add-row"><input autoFocus value={profileName} onChange={(event) => setProfileName(event.target.value)} placeholder="e.g. Work" maxLength={48} /><button className="save-small" disabled={!profileName.trim()} onClick={() => { onSave(profileProvider, profileName.trim()); setProfileName(""); setAdding(false); }}><Check size={13} /></button><button className="cancel-small" onClick={() => setAdding(false)}><X size={13} /></button></div> : <button className="outline-button" onClick={() => setAdding(true)}><Plus size={15} /> save credential source label</button>}</div></div>
    <div className="settings-section"><div className="section-title"><div><h3>Manual web fallback</h3><p>Stored in the OS keyring; never extracted from a browser.</p></div><ShieldCheck size={17} /></div><div className="profile-tabs">{(["cursor", "opencode"] as ProviderId[]).map((provider) => <button key={provider} className={provider === manualProvider ? "active" : ""} onClick={() => { setManualProvider(provider); setManualSaved(false); }}>{providerById(provider).name}</button>)}</div><div className="manual-entry"><input type="password" value={manualValue} onChange={(event) => { setManualValue(event.target.value); setManualSaved(false); }} placeholder={manualProvider === "cursor" ? "Paste Cookie header" : "Paste API key or Cookie header"} autoComplete="off" /><button className="save-small" disabled={!manualValue.trim()} onClick={() => { onSaveManual(manualProvider, manualValue.trim()); setManualValue(""); setManualSaved(true); }}><Check size={13} /></button></div>{manualSaved && <span className="manual-saved">Saved locally</span>}</div>
    <div className="settings-section compact-section"><div className="setting-row"><div className="setting-copy"><strong>Limit warnings</strong><span>Notify once when a window crosses a threshold</span></div><button className={`toggle ${settings.limitWarnings ? "on" : ""}`} onClick={() => onSettingsChange({ ...settings, limitWarnings: !settings.limitWarnings })} aria-label="Toggle limit warnings"><span /></button></div><div className="setting-row"><div className="setting-copy"><strong>Warning thresholds</strong><span>Percentage used</span></div><div className="threshold-inputs"><label><input type="number" min={1} max={settings.warningThresholds[1] - 1} value={settings.warningThresholds[0]} onChange={(event) => onSettingsChange({ ...settings, warningThresholds: [Math.max(1, Math.min(settings.warningThresholds[1] - 1, Number(event.target.value) || 1)), settings.warningThresholds[1]] })} /><span>%</span></label><label><input type="number" min={settings.warningThresholds[0] + 1} max={99} value={settings.warningThresholds[1]} onChange={(event) => onSettingsChange({ ...settings, warningThresholds: [settings.warningThresholds[0], Math.min(99, Math.max(settings.warningThresholds[0] + 1, Number(event.target.value) || 99))] })} /><span>%</span></label></div></div><div className="setting-row"><div className="setting-copy"><strong>Refresh interval</strong><span>Poll providers in the background</span></div><div className="segmented">{[30, 60, 300].map((seconds) => <button key={seconds} className={settings.refreshSeconds === seconds ? "active" : ""} onClick={() => onSettingsChange({ ...settings, refreshSeconds: seconds })}>{seconds < 60 ? `${seconds}s` : `${seconds / 60}m`}</button>)}</div></div><div className="setting-row"><div className="setting-copy"><strong>Launch at login</strong><span>Open Burnrate when you sign in</span></div><button className={`toggle ${settings.launchAtLogin ? "on" : ""}`} onClick={() => onSettingsChange({ ...settings, launchAtLogin: !settings.launchAtLogin })}><span /></button></div><div className="setting-row"><div className="setting-copy"><strong>Start hidden in {hiddenLocation}</strong><span>Hide the window on future launches</span></div><button className={`toggle ${settings.startHiddenInTray ? "on" : ""}`} onClick={() => onSettingsChange({ ...settings, startHiddenInTray: !settings.startHiddenInTray })} aria-label={`Start hidden in ${hiddenLocation}`}><span /></button></div><div className="setting-row"><div className="setting-copy"><strong>Theme</strong><span>Dark is easy on the eyes</span></div><div className="segmented">{["dark", "light", "system"].map((theme) => <button key={theme} className={settings.theme === theme ? "active" : ""} onClick={() => onSettingsChange({ ...settings, theme: theme as AppSettings["theme"] })}>{theme}</button>)}</div></div></div>
    <div className="privacy-note"><ShieldCheck size={15} /><span><strong>Everything stays local.</strong> Burnrate talks only to provider endpoints. No telemetry, no analytics, no browser cookie extraction.</span></div>
  </div>;
}

export default function App() {
  const { snapshots, setSnapshots, refreshing, refresh } = useMockSnapshots();
  const [view, setView] = useState<View>(window.location.hash === "#settings" ? "settings" : "overview");
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const [profiles, setProfiles] = useState<Record<ProviderId, string[]>>({ claude: ["Personal"], codex: ["Personal"], grok: ["Personal"], cursor: [], opencode: [] });
  const [activeProfiles, setActiveProfiles] = useState<Record<ProviderId, string>>({ claude: "Personal", codex: "Personal", grok: "Personal", cursor: "Personal", opencode: "Personal" });
  const [addingAccount, setAddingAccount] = useState<ProviderId | null>(null);
  const visibleProviders = useMemo(() => PROVIDERS.filter((provider) => settings.enabled[provider.id]), [settings.enabled]);
  useEffect(() => { document.documentElement.dataset.theme = settings.theme; }, [settings.theme]);
  useEffect(() => {
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void invoke<AppSettings>("get_settings").then((loaded) => {
        console.info("settings: frontend after mount", loaded);
        if (!cancelled) setSettings(loaded);
      }).catch((error) => console.warn("settings: load failed", error));
    }, 0);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, []);
  useEffect(() => {
    let cancelled = false;
    Promise.all(PROVIDERS.map(async (provider) => {
      const result = await invoke<string[]>("list_profiles", { provider: provider.id }).catch(() => profiles[provider.id] || ["Personal"]);
      return [provider.id, result.length ? result : ["Personal"]] as const;
    })).then((entries) => {
      if (!cancelled) {
        const loaded = Object.fromEntries(entries) as Record<ProviderId, string[]>;
        setProfiles(loaded);
        setActiveProfiles((current) => Object.fromEntries(PROVIDERS.map((provider) => {
          const available = loaded[provider.id]?.length ? loaded[provider.id] : ["Personal"];
          return [provider.id, current[provider.id] === "All" || available.includes(current[provider.id]) ? current[provider.id] : available[0]];
        })) as Record<ProviderId, string>);
      }
    });
    return () => { cancelled = true; };
  }, []);
  useEffect(() => { void refresh(); }, [refresh]);
  useEffect(() => { const timer = window.setInterval(() => void refresh(), settings.refreshSeconds * 1000); return () => window.clearInterval(timer); }, [refresh, settings.refreshSeconds]);
  const snapshotsFor = (provider: ProviderId) => {
    const matches = snapshots.filter((snapshot) => snapshot.provider === provider && (activeProfiles[provider] === "All" || snapshot.profile_name === activeProfiles[provider]));
    return matches;
  };
  const changeSettings = (next: AppSettings) => {
    setSettings(next);
    void invoke("save_settings", { settings: next }).then(() => console.info("settings: save ok")).catch((error) => console.warn("settings: save failed", error));
  };
  const deleteProfile = (provider: ProviderId, profile: string) => {
    const apply = () => {
      setProfiles((current) => ({ ...current, [provider]: current[provider].filter((item) => item !== profile) }));
      if (activeProfiles[provider] === profile) setActiveProfiles((current) => ({ ...current, [provider]: "Personal" }));
    };
    void invoke("delete_profile", { provider, name: profile }).then(apply).catch(() => {
      if (!("__TAURI_INTERNALS__" in window)) apply();
    });
  };
  const saveProfile = (provider: ProviderId, profile: string) => {
    const apply = () => {
      setProfiles((current) => ({ ...current, [provider]: [...new Set([...current[provider], profile])] }));
      const source = snapshots.find((snapshot) => snapshot.provider === provider);
      if (source) setSnapshots((current) => [...current.filter((snapshot) => !(snapshot.provider === provider && snapshot.profile_name === profile)), { ...source, profile_name: profile }]);
    };
    void invoke("save_profile", { provider, name: profile }).then(apply).catch(() => {
      if (!("__TAURI_INTERNALS__" in window)) apply();
    });
  };
  const saveManual = (provider: ProviderId, value: string) => {
    void invoke("save_manual_credential", { provider, value }).catch(() => undefined);
  };
  const completeAddAccount = (provider: ProviderId, result: AddAccountResult) => {
    setProfiles((current) => ({ ...current, [provider]: result.profiles }));
    setActiveProfiles((current) => ({ ...current, [provider]: result.profileName }));
    void refresh();
  };
  return <main className="app-shell"><header className="app-header" data-tauri-drag-region><div className="brand" data-tauri-drag-region><span data-tauri-drag-region>Burnrate</span></div><div className="header-actions"><button className="icon-button" onClick={() => void refresh()} aria-label="Refresh" data-refreshing={refreshing}><RefreshCw size={15} /></button><button className={`icon-button ${view === "settings" ? "is-active" : ""}`} onClick={() => setView(view === "settings" ? "overview" : "settings")} aria-label="Settings"><Settings2 size={15} /></button><button className="icon-button" onClick={() => void getCurrentWindow().close()} aria-label="Close"><X size={15} /></button></div></header>
    {view === "overview" ? <><div className="overview-meta"><span>LIVE USAGE</span><span>Updated {formatTime(new Date().toISOString())}</span></div><div className="cards">{visibleProviders.map((provider) => <ProviderCard key={provider.id} providerId={provider.id} snapshots={snapshotsFor(provider.id)} profile={activeProfiles[provider.id]} profiles={profiles[provider.id] || ["Personal"]} onProfileChange={(profile) => setActiveProfiles((current) => ({ ...current, [provider.id]: profile }))} onAddAccount={() => setAddingAccount(provider.id)} />)}</div></> : <Settings settings={settings} onSettingsChange={changeSettings} profiles={profiles} onDelete={deleteProfile} onSave={saveProfile} onSaveManual={saveManual} />}
    {addingAccount && <AddAccountDialog providerId={addingAccount} onClose={() => setAddingAccount(null)} onComplete={(result) => completeAddAccount(addingAccount, result)} />}
    </main>;
}
