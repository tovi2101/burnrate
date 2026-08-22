export type ProviderId = "claude" | "codex" | "grok" | "cursor" | "opencode";
export type SnapshotStatus = "fresh" | "stale" | "error";
export type WindowLabel = "5h" | "Weekly" | "Monthly" | "Credits";

export interface UsageWindow {
  label: WindowLabel;
  used_pct: number;
  resets_at?: string | null;
  pace_limit_minutes?: number | null;
}

export interface UsageSnapshot {
  provider: ProviderId;
  profile_name: string;
  plan_name?: string | null;
  windows: UsageWindow[];
  fetched_at: string;
  status: SnapshotStatus;
  error_message?: string | null;
}

export interface ProviderConfig {
  id: ProviderId;
  name: string;
  accent: string;
  softAccent: string;
  command: string;
}

export interface AppSettings {
  enabled: Record<ProviderId, boolean>;
  refreshSeconds: number;
  launchAtLogin: boolean;
  startHiddenInTray: boolean;
  limitWarnings: boolean;
  warningThresholds: [number, number];
  theme: "dark" | "light" | "system";
}
