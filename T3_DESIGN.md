# T3 Code design harvest

Source snapshot: `references/t3code` at shallow commit `592c5983`.

## Dark color roles

The values below are copied from `apps/web/src/themePalette.ts`.

| Role | Value |
| --- | --- |
| canvas / chrome / toolbar | `#0A0A0A` |
| surface / raised / overlay | `#111111` / `#141414` / `#191919` |
| toolbar border / control / control hover | `#191919` / `#191919` / `#141414` |
| text / muted text | `#F5F5F5` / `#818181` |
| border / input | `#191919` / `#1E1E1E` |
| focus / accent / accent foreground | `#346BF1` / `#346BF1` / `#FFFFFF` |
| secondary / secondary foreground | `#141414` / `#F5F5F5` |
| error / error foreground / error surface | `#FB414A` / `#FF6467` / `#301214` |
| warning / warning foreground / warning surface | `#FE9A00` / `#FFB900` / `#312108` |
| success / success foreground | `#10B981` / `#34D399` |
| update / update foreground / update surface | `#346BF1` / `#51A2FF` / `#121B34` |
| sidebar / foreground / muted | `#000000` / `#F1F3F7` / `#A3A3A3` |
| sidebar control / hover / active / selected / border | `#0A0A0A` / `#131313` / `#1A1B1B` / `#111111` / `#141414` |
| terminal background / foreground / cursor / selection | `#0A0A0A` / `#F5F5F5` / `#B4CBFF` / `#343A47` |
| terminal scrollbar / hover | `#222222` / `#363636` |

## Provider presentation

T3 Code's Usage view currently supports Claude and Codex only. Claude is explicitly `#D97757`; Codex is `var(--foreground)`. Burnrate cannot use a white or near-white provider accent, so Codex uses the brief's `#10A37F` fallback. T3 defines no Usage-view colors for Cursor, Grok, or OpenCode; their declared fallbacks are `#6E56CF`, `#60A5FA`, and `#F59E0B`. Grok remains blue even where monochrome brand art is white.

Provider artwork is copied from `apps/marketing/public/harnesses/`. T3's Usage view renders an 8px provider dot beside a 16px mark; Burnrate preserves that presentation language at its compact-window scale.

## Typography and geometry

- Sans: `-apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif`.
- Mono: `ui-monospace, "SF Mono", "SFMono-Regular", Menlo, Consolas, "Liberation Mono", monospace`.
- Usage type: 11px metadata, 12px controls and secondary values, 14px labels and headings, 16px body, 18px card titles, 36px summary values. Weights are 400, 500, and 600. Numeric values use tabular numerals.
- Radius base: 10px. Derived radii: 6, 8, 10, 14, 18, 22, and 26px.
- Spacing grid: 2, 4, 6, 8, 10, 12, 16, 20, and 24px.
- Cards: 18px radius, 1px border, `#111111` surface, `shadow-xs/5`, and a subtle 6%-white inset highlight at the top.
- Compact controls are 28px; segmented controls are 24px high with 8px radius, 10px horizontal padding, and 2px outer padding/gap.
- Segmented hover is 32% of the input surface; selected is 72%, with a tiny shadow. Transitions affect color, not layout.
- Scrollbars are 6px with a 3px-radius `rgba(255,255,255,.08)` thumb and `.12` hover thumb.
- Glass overlays use 80% opacity, 16px blur, and 1.08 saturation.

## Usage chart and progress language

T3 Code uses a bespoke SVG chart rather than a chart dependency: monotone lines at 2px, provider-colored area fills at 12%, 1px border-colored grid lines, a 1px muted hover guide, and a 14px-radius glass tooltip with 10px by 8px padding.

There is no T3 progress-bar component in this snapshot. Burnrate's existing quota bars therefore inherit the exact T3 surfaces, provider colors, borders, radii, and color transitions without inventing a component attributed to T3.
