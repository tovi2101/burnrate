import type { ProviderConfig, ProviderId } from "./types";

export const PROVIDERS: ProviderConfig[] = [
  { id: "claude", name: "Claude", accent: "#D97757", softAccent: "rgba(217,119,87,.14)", command: "claude auth login" },
  { id: "codex", name: "Codex", accent: "#10A37F", softAccent: "rgba(16,163,127,.14)", command: "codex login" },
  { id: "grok", name: "Grok", accent: "#E5E7EB", softAccent: "rgba(229,231,235,.1)", command: "grok login" },
  { id: "cursor", name: "Cursor", accent: "#6E56CF", softAccent: "rgba(110,86,207,.14)", command: "cursor-agent login" },
  { id: "opencode", name: "OpenCode", accent: "#F59E0B", softAccent: "rgba(245,158,11,.14)", command: "opencode auth login" },
];

export const providerById = (id: ProviderId) => PROVIDERS.find((provider) => provider.id === id)!;
