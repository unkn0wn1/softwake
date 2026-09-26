# ADR 0020 — Settings UI text scale

- **Status:** Accepted
- **Date:** 2026-09-26

## Decision

The Softwake Settings window exposes a **UI text size** control on **General**:
`xx-small`, `x-small` (default), `small`, `medium`, `large`.

- The value is persisted as `$XDG_CONFIG_HOME/softwake/ui-prefs.json` (or `~/.config/softwake/…`).
- A single `data-text-size` attribute on the document root drives CSS variables so **General, Profiles, Providers, Tools, Email, and Status** share one density.
- Label / body text scales with the setting. **Button chrome stays compact** (shared padding tokens) so nav and pane actions do not grow into large touch targets when text is larger.
- Profiles pack actions (**Save pack**, **Reload from disk**, **Reload soul**) sit in a **pinned footer** outside the scrollable editor region.
- Default Settings window size is slightly larger so denser content still fits.

## Context

Profiles chrome was already denser than Email and other panes. Operators asked for a smaller default text size, compact buttons (including the left nav), a Profiles footer that stays visible without scrolling past pack text, and the same density on Email.

## Alternatives

- Put `text_size` on `softwake.json`. Rejected for this slice: UI-only prefs stay beside `hud-position.json`.
- Per-pane font overrides. Rejected: one root scale keeps panes consistent.

## Consequences

- Missing or corrupt `ui-prefs.json` falls back to `x-small`.
- HUD capsule text scale is unchanged (separate HTML/CSS). The same file now also stores HUD idle collapse; see the amendment below.

## Amendment — HUD idle collapse (2026-09-26)

`ui-prefs.json` stores `hud_idle_collapse_ms` (`u32`, default 3000, clamped to 1000–30000). Settings → General shows it as seconds (1–30, default 3), with a slider and a number. Saving one pref does not reset the other. The HUD reads the file on load and on its status poll, so a save applies without restarting softwaked and without a daemon command.
