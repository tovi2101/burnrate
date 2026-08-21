import type { ProviderConfig, ProviderId } from "./types";

export const PROVIDERS: ProviderConfig[] = [
  { id: "claude", name: "Claude", accent: "#D97757", softAccent: "rgba(217,119,87,.14)", command: "claude auth login" },
  { id: "codex", name: "Codex", accent: "#8FCB9B", softAccent: "rgba(143,203,155,.14)", command: "codex login" },
  { id: "grok", name: "Grok", accent: "#D2D8E4", softAccent: "rgba(210,216,228,.12)", command: "grok login" },
  { id: "cursor", name: "Cursor", accent: "#8B9BFF", softAccent: "rgba(139,155,255,.14)", command: "cursor-agent login" },
  { id: "opencode", name: "OpenCode", accent: "#D7A5FF", softAccent: "rgba(215,165,255,.14)", command: "opencode auth login" },
];

export const providerById = (id: ProviderId) => PROVIDERS.find((provider) => provider.id === id)!;
