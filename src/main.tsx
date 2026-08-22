import { createRoot } from "react-dom/client";
import "./styles.css";
import App from "./App";
import { t3 } from "./theme/t3";

const root = document.documentElement;
const cssTokens: Record<string, string | number> = {
  canvas: t3.color.canvas,
  chrome: t3.color.chrome,
  surface: t3.color.surface,
  "surface-raised": t3.color.surfaceRaised,
  "surface-overlay": t3.color.surfaceOverlay,
  text: t3.color.text,
  muted: t3.color.textMuted,
  border: t3.color.border,
  input: t3.color.input,
  focus: t3.color.focus,
  accent: t3.color.accent,
  "accent-fg": t3.color.accentForeground,
  secondary: t3.color.secondary,
  success: t3.color.success,
  "success-fg": t3.color.successForeground,
  warning: t3.color.warning,
  "warning-fg": t3.color.warningForeground,
  "warning-surface": t3.color.warningSurface,
  danger: t3.color.error,
  "danger-fg": t3.color.errorForeground,
  "danger-surface": t3.color.errorSurface,
  glass: t3.color.glass,
  scrollbar: t3.color.scrollbar,
  "scrollbar-hover": t3.color.scrollbarHover,
  "inset-highlight": t3.color.insetHighlight,
  "provider-claude": t3.provider.claude,
  "provider-codex": t3.provider.codex,
  "provider-cursor": t3.provider.cursor,
  "provider-grok": t3.provider.grok,
  "provider-opencode": t3.provider.opencode,
  "font-sans": t3.font.sans,
  "font-mono": t3.font.mono,
};
Object.entries(cssTokens).forEach(([name, value]) => root.style.setProperty(`--t3-${name}`, String(value)));

createRoot(document.getElementById("root")!).render(<App />);
