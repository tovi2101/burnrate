import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Check,
  ChevronDown,
  Copy,
  Gauge,
  Plus,
  RefreshCw,
  Settings2,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";
import { providerById, PROVIDERS } from "./providers";
import claudeLogo from "./assets/brands/claude.svg";
import codexLogo from "./assets/brands/codex.svg";
import cursorLogo from "./assets/brands/cursor.svg";
import grokLogo from "./assets/brands/grok.svg";
import opencodeLogo from "./assets/brands/opencode.svg";
import type { AppSettings, HistoryPayload, HistorySeries, ProviderId, UsageSnapshot, UsageWindow } from "./types";

type View = "usage" | "settings";
type UsageTab = "overview" | ProviderId;
type UsageRange = "now" | "24h" | "7d" | "30d";

const providerLogos: Record<ProviderId, string> = {
  claude: claudeLogo,
  codex: codexLogo,
  cursor: cursorLogo,
  grok: grokLogo,
  opencode: opencodeLogo,
};

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

function urgencyFor(remaining: number) {
  if (remaining <= 10) return "critical";
  if (remaining <= 30) return "warning";
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
  return <span className={`provider-glyph ${muted ? "is-muted" : ""}`} style={{ "--provider": providerById(providerId).accent } as React.CSSProperties}><img src={providerLogos[providerId]} alt="" /></span>;
}

function UsageBar({ window: usageWindow, accent }: { window: UsageWindow; accent: string }) {
  const [canAnimate, setCanAnimate] = useState(false);
  useEffect(() => {
    const frame = globalThis.requestAnimationFrame(() => setCanAnimate(true));
    return () => globalThis.cancelAnimationFrame(frame);
  }, []);
  const remaining = Math.max(0, Math.min(100, 100 - usageWindow.used_pct));
  const urgency = urgencyFor(remaining);
  return <section className="window-block" style={{ "--provider-accent": accent } as React.CSSProperties}>
    <div className="window-headline"><div><i className={`urgency-dot ${urgency}`} /><strong>{usageWindow.label} · <span className={urgency}>{Math.round(remaining)}% left</span></strong></div><span>Resets in {formatCountdown(usageWindow.resets_at)}</span></div>
    <div className="window-track"><div className={`window-fill ${canAnimate ? "can-animate" : ""}`} style={{ width: `${remaining}%` }} /></div>
    {usageWindow.pace_limit_minutes != null && <p className="window-pace">At this pace: runs out in {formatPace(usageWindow.pace_limit_minutes)}</p>}
  </section>;
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

function OverviewRow({ providerId, snapshots, onOpen }: { providerId: ProviderId; snapshots: UsageSnapshot[]; onOpen: () => void }) {
  const provider = providerById(providerId);
  const snapshot = snapshots[0];
  if (!snapshot) return <button className="overview-row is-empty" onClick={onOpen} style={{ "--provider-accent": provider.accent } as React.CSSProperties}>
    <ProviderMark providerId={providerId} muted /><span className="overview-provider"><strong>{provider.name}</strong><code>{provider.command}</code></span><span className="overview-not-connected">Not connected</span>
  </button>;
  const constrained = snapshot.windows.reduce<UsageWindow | null>((current, item) => !current || item.used_pct > current.used_pct ? item : current, null);
  const left = Math.max(0, 100 - (constrained?.used_pct ?? 0));
  const urgency = urgencyFor(left);
  return <button className={`overview-row ${snapshot.status !== "fresh" ? "is-stale" : ""}`} onClick={onOpen} style={{ "--provider-accent": provider.accent } as React.CSSProperties}>
    <ProviderMark providerId={providerId} /><span className="overview-provider"><strong>{provider.name}</strong><span className={urgency}>{Math.round(left)}% left</span></span><span className="overview-meter"><i style={{ width: `${left}%` }} /></span><span className="overview-reset">{constrained?.resets_at ? formatCountdown(constrained.resets_at) : "—"}</span>
  </button>;
}

const estimatePerMillion: Partial<Record<ProviderId, number>> = { claude: 6, codex: 3, grok: 3 };

function ProviderStats({ providerId }: { providerId: ProviderId }) {
  const [history, setHistory] = useState<HistoryPayload | null>(null);
  useEffect(() => { void invoke<HistoryPayload>("get_history", { range: "30d" }).then(setHistory).catch(() => undefined); }, [providerId]);
  const tokenSeries = history?.series.filter((series) => series.provider === providerId && series.unit === "tokens") || [];
  const total = history?.summaries.find((summary) => summary.provider === providerId)?.totalTokens || 0;
  const startOfToday = new Date();
  startOfToday.setHours(0, 0, 0, 0);
  const today = tokenSeries.flatMap((series) => series.points).filter((point) => point.timestamp >= startOfToday.getTime()).reduce((sum, point) => sum + point.value, 0);
  if (total <= 0) return null;
  const rate = estimatePerMillion[providerId];
  return <section className="provider-stats"><div><span>Today tokens</span><strong>{formatTokens(today)}</strong></div><div><span>30d tokens</span><strong>{formatTokens(total)}</strong></div>{rate != null && <div><span>Estimated 30d cost</span><strong>~${((total / 1_000_000) * rate).toFixed(2)}</strong></div>}<p>Estimated from token usage, not a subscription bill.</p></section>;
}

function ProviderView({ providerId, snapshots, profile, profiles, range, onRangeChange, onProfileChange, onAddAccount }: { providerId: ProviderId; snapshots: UsageSnapshot[]; profile: string; profiles: string[]; range: UsageRange; onRangeChange: (value: UsageRange) => void; onProfileChange: (value: string) => void; onAddAccount: () => void }) {
  const provider = providerById(providerId);
  const snapshot = snapshots[0];
  const [identity, setIdentity] = useState<string | null>(null);
  useEffect(() => { void invoke<AccountSetup>("get_account_setup", { provider: providerId }).then((value) => setIdentity(value.identity || null)).catch(() => setIdentity(null)); }, [providerId]);
  return <div className="provider-view" style={{ "--provider-accent": provider.accent } as React.CSSProperties}>
    <header className="provider-header"><div className="provider-title"><ProviderMark providerId={providerId} muted={!snapshot} /><h1>{provider.name}</h1></div><div className="provider-account"><span title={identity || undefined}>{identity || snapshot?.profile_name || "Not connected"}</span>{profiles.length > 1 ? <ProfilePicker profile={profile} profiles={profiles} onChange={onProfileChange} onAddAccount={onAddAccount} /> : <button className="add-account-button" onClick={onAddAccount} aria-label={`Add ${provider.name} account`}><Plus size={14} /></button>}</div><div className="provider-subhead"><span>{snapshot ? `Updated ${formatUpdated(snapshot.fetched_at)}` : "Waiting for a CLI login"}</span><span>{snapshot?.plan_name || "—"}</span></div></header>
    <nav className="range-tabs" aria-label={`${provider.name} usage range`}>{(["now", "24h", "7d", "30d"] as UsageRange[]).map((option) => <button key={option} className={range === option ? "active" : ""} onClick={() => onRangeChange(option)}>{option === "now" ? "Now" : option}</button>)}</nav>
    {range === "now" ? <div className="provider-now">{!snapshot ? <div className="provider-empty"><strong>Not connected</strong><span>Sign in to see your live limits.</span><code>{provider.command}</code></div> : <>{snapshots.map((item) => <section className="profile-usage" key={item.profile_name}>{profile === "All" && <h2>{item.profile_name}</h2>}{item.status !== "fresh" && <div className="stale-line">Stale since {formatTime(item.fetched_at)}</div>}{item.windows.map((usageWindow, index) => <UsageBar key={`${item.profile_name}-${usageWindow.label}-${index}`} window={usageWindow} accent={provider.accent} />)}{item.error_message && <p className="provider-message">{item.error_message}</p>}</section>)}<ProviderStats providerId={providerId} /></>}</div> : <HistoryView range={range} providerId={providerId} />}
  </div>;
}

function formatUpdated(value: string) {
  const minutes = Math.max(0, Math.floor((Date.now() - new Date(value).getTime()) / 60_000));
  return minutes < 1 ? "just now" : `${minutes}m ago`;
}

function formatTokens(value: number) {
  if (value >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(value >= 10_000_000_000 ? 0 : 1)}B`;
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(value >= 10_000 ? 0 : 1)}K`;
  return Math.round(value).toLocaleString();
}

function formatDay(value: string) {
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" }).format(new Date(`${value}T00:00:00Z`));
}

function chartSegments(series: HistorySeries, bucketMs: number) {
  return series.points.reduce<Array<typeof series.points>>((segments, point) => {
    const current = segments.at(-1);
    if (!current || (current.at(-1) && point.timestamp - current.at(-1)!.timestamp > bucketMs * 1.5)) {
      segments.push([point]);
    } else {
      current.push(point);
    }
    return segments;
  }, []);
}

function HistoryChart({ title, unit, series, range }: { title: string; unit: "tokens" | "percent"; series: HistorySeries[]; range: Exclude<UsageRange, "now"> }) {
  const width = 328;
  const height = 174;
  const inset = { left: 42, right: 7, top: 13, bottom: 24 };
  const duration = range === "24h" ? 86_400_000 : range === "7d" ? 7 * 86_400_000 : 30 * 86_400_000;
  const bucketMs = range === "24h" ? 3_600_000 : 86_400_000;
  const end = Date.now();
  const start = end - duration;
  const maxValue = unit === "percent" ? 100 : Math.max(1, ...series.flatMap((item) => item.points.map((point) => point.value)));
  const x = (timestamp: number) => inset.left + ((timestamp - start) / duration) * (width - inset.left - inset.right);
  const y = (value: number) => inset.top + (1 - Math.min(1, value / maxValue)) * (height - inset.top - inset.bottom);
  const label = unit === "percent" ? "100%" : formatTokens(maxValue);
  return <section className="history-chart-card">
    <div className="history-chart-heading"><div><span className="chart-kicker">{unit === "tokens" ? (range === "24h" ? "TOKENS / HOUR" : "TOKENS / DAY") : "LIMIT USED"}</span><h2>{title}</h2></div><span className="chart-unit">{unit === "tokens" ? "tokens" : "%"}</span></div>
    <div className="history-legend">{series.map((item) => <span key={`${item.provider}-${item.kind}`}><i style={{ background: providerById(item.provider).accent }} />{providerById(item.provider).name}</span>)}</div>
    <svg className="history-chart" viewBox={`0 0 ${width} ${height}`} role="img" aria-label={`${title}, ${range}`}>
      {[0, .5, 1].map((ratio) => <line key={ratio} x1={inset.left} x2={width - inset.right} y1={y(maxValue * ratio)} y2={y(maxValue * ratio)} className="chart-grid" />)}
      <text x={inset.left - 7} y={y(maxValue) + 3} textAnchor="end">{label}</text>
      <text x={inset.left - 7} y={y(0) + 3} textAnchor="end">0</text>
      {series.flatMap((item) => chartSegments(item, bucketMs).map((segment, index) => {
        const color = providerById(item.provider).accent;
        if (segment.length === 1) return <circle key={`${item.provider}-${index}`} cx={x(segment[0].timestamp)} cy={y(segment[0].value)} r="2.5" fill={color} />;
        return <polyline key={`${item.provider}-${index}`} points={segment.map((point) => `${x(point.timestamp)},${y(point.value)}`).join(" ")} fill="none" stroke={color} className="chart-series" />;
      }))}
      <text x={inset.left} y={height - 5}>{range === "24h" ? "24h ago" : `${range} ago`}</text>
      <text x={width - inset.right} y={height - 5} textAnchor="end">Now</text>
    </svg>
  </section>;
}

function HistoryView({ range, providerId }: { range: Exclude<UsageRange, "now">; providerId: ProviderId }) {
  const [history, setHistory] = useState<HistoryPayload | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    let cancelled = false;
    let timer = 0;
    const load = async () => {
      try {
        const value = await invoke<HistoryPayload>("get_history", { range });
        if (cancelled) return;
        setHistory(value);
        setError("");
        if (value.importing) timer = window.setTimeout(() => void load(), 700);
      } catch (reason) {
        if (!cancelled) setError(String(reason));
      }
    };
    void load();
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [range]);
  if (!history && !error) return <div className="history-loading"><RefreshCw size={14} /> Reading local usage history…</div>;
  if (error) return <div className="history-empty"><strong>History is unavailable</strong><span>{error}</span></div>;
  const tokenSeries = history?.series.filter((series) => series.provider === providerId && series.unit === "tokens") || [];
  const percentPriority = ["percent:5h", "percent:Credits", "percent:Weekly", "percent:Monthly"];
  const percentSeries = (history?.series.filter((series) => series.provider === providerId && series.unit === "percent") || [])
    .sort((left, right) => percentPriority.indexOf(left.kind) - percentPriority.indexOf(right.kind));
  const hasHistory = tokenSeries.some((series) => series.points.length) || percentSeries.some((series) => series.points.length);
  return <div className="history-view">
    {history?.importing && <div className="importing-history"><span /> Importing history</div>}
    {!hasHistory && <div className="history-empty"><strong>No usage in this range</strong><span>{history?.message || "Burnrate will keep honest local samples as they appear."}</span></div>}
    {tokenSeries.length > 0 && <HistoryChart title="Local CLI activity" unit="tokens" series={tokenSeries} range={range} />}
    {percentSeries.length > 0 && <HistoryChart title="Live limit snapshots" unit="percent" series={percentSeries} range={range} />}
    <section className="history-summary-card"><div className="summary-heading"><span>SUMMARY</span><span>{range.toUpperCase()}</span></div>{history?.summaries.filter((summary) => summary.provider === providerId).map((summary) => <div className="history-summary-row" key={summary.provider}><ProviderMark providerId={summary.provider} /><div className="summary-provider"><strong>{providerById(summary.provider).name}</strong><span>{summary.since ? `History since ${formatDay(summary.since)}` : "No history in range"}</span></div><div className="summary-values"><strong>{summary.totalTokens > 0 ? formatTokens(summary.totalTokens) : summary.peakPercent != null ? `${Math.round(summary.peakPercent)}% peak` : "—"}</strong><span>{summary.totalTokens > 0 ? `Most active ${summary.mostActiveDay ? formatDay(summary.mostActiveDay) : "—"}` : `${summary.limitHits} limit hits`}</span></div></div>)}</section>
    {history?.series.some((series) => series.provider === providerId && series.unit === "context_tokens") && <p className="history-footnote">Grok session context snapshots are stored separately from tokens/day because they are cumulative, not per-message usage.</p>}
  </div>;
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
  const [view, setView] = useState<View>(window.location.hash === "#settings" ? "settings" : "usage");
  const [activeTab, setActiveTab] = useState<UsageTab>("overview");
  const [range, setRange] = useState<UsageRange>("now");
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const [profiles, setProfiles] = useState<Record<ProviderId, string[]>>({ claude: ["Personal"], codex: ["Personal"], grok: ["Personal"], cursor: [], opencode: [] });
  const [activeProfiles, setActiveProfiles] = useState<Record<ProviderId, string>>({ claude: "Personal", codex: "Personal", grok: "Personal", cursor: "Personal", opencode: "Personal" });
  const [addingAccount, setAddingAccount] = useState<ProviderId | null>(null);
  const visibleProviders = useMemo(() => PROVIDERS.filter((provider) => settings.enabled[provider.id]), [settings.enabled]);
  const shortcut = /Macintosh|Mac OS X/.test(navigator.userAgent) ? "Cmd" : "Ctrl";
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
  useEffect(() => {
    if (activeTab !== "overview" && !settings.enabled[activeTab]) setActiveTab("overview");
  }, [activeTab, settings.enabled]);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey)) return;
      if (event.key.toLowerCase() === "r") { event.preventDefault(); void refresh(); }
      if (event.key === ",") { event.preventDefault(); setView("settings"); }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [refresh]);
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
  return <main className="app-shell"><header className="window-header" data-tauri-drag-region><div className="brand" data-tauri-drag-region><span className="brand-mark" data-tauri-drag-region><i /><i /><i /></span><span data-tauri-drag-region>Burnrate</span></div><button className="icon-button" onClick={() => void getCurrentWindow().close()} aria-label="Close"><X size={15} /></button></header>
    <nav className="provider-tabs" aria-label="Provider tabs"><button className={view === "usage" && activeTab === "overview" ? "active overview" : ""} onClick={() => { setView("usage"); setActiveTab("overview"); }}>Overview</button>{visibleProviders.map((provider) => <button key={provider.id} className={view === "usage" && activeTab === provider.id ? "active" : ""} style={{ "--provider-accent": provider.accent } as React.CSSProperties} onClick={() => { setView("usage"); setActiveTab(provider.id); setRange("now"); }}><ProviderMark providerId={provider.id} /><span>{provider.name}</span></button>)}</nav>
    <div className="content-scroll">{view === "settings" ? <Settings settings={settings} onSettingsChange={changeSettings} profiles={profiles} onDelete={deleteProfile} onSave={saveProfile} onSaveManual={saveManual} /> : activeTab === "overview" ? <div className="overview-view"><header><h1>Overview</h1><p>Most constrained limit for each provider</p></header><div className="overview-list">{visibleProviders.map((provider) => <OverviewRow key={provider.id} providerId={provider.id} snapshots={snapshotsFor(provider.id)} onOpen={() => { setActiveTab(provider.id); setRange("now"); }} />)}</div></div> : <ProviderView providerId={activeTab} snapshots={snapshotsFor(activeTab)} profile={activeProfiles[activeTab]} profiles={profiles[activeTab] || ["Personal"]} range={range} onRangeChange={setRange} onProfileChange={(profile) => setActiveProfiles((current) => ({ ...current, [activeTab]: profile }))} onAddAccount={() => setAddingAccount(activeTab)} />}</div>
    <footer className="bottom-bar"><button onClick={() => void refresh()} data-refreshing={refreshing}><RefreshCw size={15} /><span>Refresh</span><kbd>{shortcut}+R</kbd></button><button className={view === "settings" ? "active" : ""} onClick={() => setView(view === "settings" ? "usage" : "settings")}><Settings2 size={15} /><span>Settings</span><kbd>{shortcut}+,</kbd></button></footer>
    {addingAccount && <AddAccountDialog providerId={addingAccount} onClose={() => setAddingAccount(null)} onComplete={(result) => completeAddAccount(addingAccount, result)} />}
    </main>;
}
