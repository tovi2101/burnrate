import type { ProviderConfig, ProviderId } from "./types";
import { t3 } from "./theme/t3";

export const PROVIDERS: ProviderConfig[] = [
  { id: "claude", name: "Claude", accent: t3.provider.claude, softAccent: t3.provider.claude, command: "claude auth login" },
  { id: "codex", name: "Codex", accent: t3.provider.codex, softAccent: t3.provider.codex, command: "codex login" },
  { id: "grok", name: "Grok", accent: t3.provider.grok, softAccent: t3.provider.grok, command: "grok login" },
  { id: "cursor", name: "Cursor", accent: t3.provider.cursor, softAccent: t3.provider.cursor, command: "cursor-agent login" },
  { id: "opencode", name: "OpenCode", accent: t3.provider.opencode, softAccent: t3.provider.opencode, command: "opencode auth login" },
];

export const providerById = (id: ProviderId) => PROVIDERS.find((provider) => provider.id === id)!;
