import { readFileSync } from "node:fs";

const theme = readFileSync(new URL("../src/theme/t3.ts", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
const providerBlock = theme.match(/provider:\s*\{([\s\S]*?)\n\s*\}/)?.[1] || "";
const accents = [...providerBlock.matchAll(/(claude|codex|cursor|grok|opencode):\s*"(#[0-9A-Fa-f]{6})"/g)];

if (accents.length !== 5) throw new Error("Expected five provider accent tokens");

const channel = (value) => {
  const normalized = value / 255;
  return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
};

for (const [, name, hex] of accents) {
  const [r, g, b] = [1, 3, 5].map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16));
  const luminance = 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
  const max = Math.max(r, g, b) / 255;
  const min = Math.min(r, g, b) / 255;
  const saturation = max === 0 ? 0 : (max - min) / max;
  if (luminance > 0.7) throw new Error(`${name} accent ${hex} is too close to white`);
  if (saturation < 0.08) throw new Error(`${name} accent ${hex} has near-zero saturation`);
}

if (!/grok:\s*"#60A5FA"/.test(providerBlock)) throw new Error("Grok must remain #60A5FA");
if (!/\.window-fill\s*\{[^}]*background:\s*var\(--provider-accent\)/s.test(styles)) {
  throw new Error("Window bars must use only the provider accent token");
}
if (/\.window-fill[^\{]*(warning|critical)[^{]*\{|\.window-fill\s*\{[^}]*(warning|danger)/s.test(styles)) {
  throw new Error("Threshold logic must never recolor a window bar");
}
if (/window-fill[^\n]*(--t3-warning|--t3-danger)|className=\{?[^\n]*window-fill[^\n]*(warning|critical)/.test(app)) {
  throw new Error("Threshold state leaked into the window bar component");
}

console.log("ui guards: provider luminance/saturation and invariant bar colors passed");
