# DESIGN.md

Visual rules for CCT's own windows, so people and agents change the UI the same way. The strip
is drawn by `src/ui/render.rs`; the Settings window by `src/ui/settings/` and follows the
Windows 11 Settings app.

## Principles

- **Real controls, our paint.** Anything you can click or tab to is a real Win32 control
  (`BUTTON` checkbox, `EDIT`), so keyboard, Narrator and UI Automation keep working. We only
  paint its look (`NM_CUSTOMDRAW`, `WM_CTLCOLOREDIT`). Backgrounds, cards and text are painted
  by the window.
- **Follow Windows.** Light or dark from the *app* theme (`AppsUseLightTheme`), the user's
  accent color, High Contrast colors when it is on, per-monitor DPI.
- **Documented APIs only.** No `uxtheme` ordinals, no `DarkMode_*` theme names.
- **Small.** Direct2D/DirectWrite are already in the binary; reuse them. No new crates.

## Spacing (DIP, 4/8 grid)

| Token | Value |
|---|---|
| Window width / padding | 460 / 16 |
| Title → first block | 12 |
| Caption top / bottom | 14 / 6 |
| Card padding (x × y) / gap between cards / radius | 14 × 10 / 3 / 6 |
| Chip padding (x × y) / gap / radius | 11 × 4 / 6 / 4 |
| Switch | 40 × 20, knob inset 4, radius 6 on / 5 off |
| Number field | 64 × 28 |
| Provider tile / text indent | 24 / 34 |

## Type

DirectWrite, `Segoe UI Variable Text` (Windows falls back when missing).

| Role | Size (DIP) | Weight |
|---|---|---|
| Page title | 20 | Semibold |
| Section caption | 12 | Semibold |
| Body (titles, switches, chips) | 13 | Regular |
| Secondary (status, hints under a title) | 11.5 | Regular |

## Color

| Token | Light | Dark |
|---|---|---|
| Window background | `#F3F3F3` | `#202020` |
| Card | `#FBFBFB` | `#2B2B2B` |
| Card border, dividers | `#E5E5E5` | `#1D1D1D` |
| Text | `#1B1B1B` | `#FFFFFF` |
| Secondary text | `#5F5F5F` | `#C5C5C5` |
| Success (`✓`) | `#0F7B0F` | `#6CCB5F` |
| Warning (`⚠`) | `#9D5D00` | `#FCE100` |
| Switch off border / knob | `#858585` / `#5D5D5D` | `#CFCFCF` / `#CFCFCF` |
| Field background | `#FFFFFF` | `#1F1F1F` |
| Accent (on switch, on chip) | user accent, else `#005FB8` | user accent mixed 40% to white, else `#4CC2FF` |
| Text on accent | `#FFFFFF` | `#000000` |

**High Contrast:** ignore this table and use `COLOR_WINDOW`, `COLOR_WINDOWTEXT`,
`COLOR_HIGHLIGHT`, `COLOR_HIGHLIGHTTEXT` and `COLOR_GRAYTEXT`, with solid borders.

The strip keeps its own palette in `render.rs` (bar colors from `model::bar_color`).

## Patterns

- **Card rows:** title (Body) with an optional secondary line, control on the right, 1 px
  divider between rows. A switch shows "On"/"Off" text to its left.
- **Status lines:** `✓` + success color, or `⚠` + warning color with how to fix it.
- **Focus:** a 2 px rounded ring in the text color around the focused switch or chip, only
  while keyboard cues are on; fields use a 2 px accent underline.
- **Changes apply immediately.** No OK/Cancel; Esc closes.

## Don'ts

- No hard-coded text: every string goes through `i18n::t`, English and pt-BR.
- Never show `0%` for an error; errors render `—`.
- No custom-drawn control that is not a real control underneath.
- No GDI rounded shapes (jagged); use Direct2D.
- No new crates for visuals.
