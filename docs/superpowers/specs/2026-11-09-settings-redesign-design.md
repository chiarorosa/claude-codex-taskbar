# Settings window redesign — design

Date: 2026-11-09 · Status: draft for approval · Related: #58 (Level 1 polish), lands before v0.4.0

## Intent

The Settings window looks like a Windows dialog from the Delphi era: grey `COLOR_BTNFACE`
background, headers in the same weight as body text, one long column of checkboxes, OK/Cancel.
The owner wants it to look and feel like the **Windows 10/11 Settings app**: a large title,
rounded cards, on/off switches, clear status lines, and light/dark that **follows the Windows
app theme automatically**, while CCT stays tiny, native and accessible.

Approved direction: mockup "B · Estilo app Configurações", light and dark. The HTML mockups
lived in a temporary session folder; the layout sketch and token table below are the reference.

## Goals

1. Settings app look: page title, section captions, cards with 1 px borders and ~6 DIP radius,
   toggle switches with an "On"/"Off" text, item chips, accent color for "on".
2. Light and dark themes that follow the **Windows app theme** and switch live while the window
   is open, including a dark title bar.
3. Same behavior on Windows 10 and 11 (Windows 11 only adds its own rounded window corners).
4. Keyboard and screen readers keep working: Tab / Shift+Tab, Space toggles, Esc closes,
   visible focus, and every toggle and chip is announced as a checkbox with its state and name.
5. Changes apply immediately (no OK/Cancel), like the Windows Settings app.

## Non-goals

- The right-click menu (stays the native menu; #58's menu part is separate work).
- Mica / acrylic backdrops, animations (no sliding switch animation), a resizable or scrolling
  window, a Settings → Accounts page (accounts stay in the right-click menu).
- Changing what any setting does.

## Anti-goals (failure even if it "works")

- Losing keyboard navigation or screen reader semantics (custom-painted controls must still be
  real checkbox/edit windows).
- Undocumented APIs: no `uxtheme` ordinals (`SetPreferredAppMode`, `AllowDarkModeForWindow`),
  no `SetWindowTheme(…, "DarkMode_Explorer"/"DarkMode_CFD")`.
- A new crate, or a release binary of 1,000,000 bytes or more.
- A white flash when opening in dark mode, or showing `0%` for an error anywhere.
- Hard-coded English or Portuguese text outside `i18n::t`.

## Constraints

- Documented Win32 / Direct2D / DirectWrite / DWM APIs only. New `windows` crate *features*
  are fine (e.g. `Win32_Graphics_Dwm`); new crates are not.
- Binary budget: the v0.4.0 build with the switcher is **920,064 bytes**; the limit is
  1,000,000. Estimated cost of this work is 15–40 KB. Measure after each step; if the build
  would cross ~985,000 bytes, stop and bring it back to the owner instead of cutting features
  silently.
- Every string through `i18n::t`, English and pt-BR.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` pass.
- Pure logic in testable functions, failing test first (AGENTS.md).

## Decided by the owner

- Direction B (Settings app style) over the native-polish option.
- Light and dark, following the Windows theme automatically.
- "Whatever saves size" for apply behavior → instant apply was chosen on that basis (below).

## The window

Fixed width ~460 DIP, height from content, not resizable, opens centered on the monitor that
hosts the strip, sized for that monitor's DPI.

```
Configurações                                  (page title, 20 DIP semibold)

[⚠ setup hint card — only when setup_hints() returns keys]

Assinaturas                                    (caption, 12 DIP semibold)
┌──────────────────────────────────────────────┐
│ [C] Claude Code                 Ativado  (●) │
│     ✓ Login encontrado                       │
├──────────────────────────────────────────────┤
│     Manter login ativo        Desativado (○) │
│     Experimental · roda `claude auth status` │
└──────────────────────────────────────────────┘
┌ same card for Codex (status ⚠ when missing / expired) ┐

Itens na faixa
┌──────────────────────────────────────────────┐
│ Hardware                                     │
│ [CPU] [RAM] [GPU] [C:] [D:]                  │   chips wrap; one per disk
├──────────────────────────────────────────────┤
│ Limites                                      │
│ [Claude 5h] [Claude semanal] [Codex 5h] …    │
└──────────────────────────────────────────────┘

Geral
┌──────────────────────────────────────────────┐
│ Iniciar com o Windows           Ativado  (●) │
├──────────────────────────────────────────────┤
│ Atualização do hardware               [  1 ] │
│ 1 a 5 segundos                               │
├──────────────────────────────────────────────┤
│ Atualização dos limites             [ 120 ]  │
│ 60 a 3600 segundos                           │
└──────────────────────────────────────────────┘
```

- Provider tiles are a colored rounded square with a monogram letter, not a brand logo.
- Login status line: `✓` + success color when found; `⚠` + warning color with the existing
  how-to-log-in / expired text otherwise (`ClaudeNoLogin`, `CodexNoLogin`, `CodexExpired`).
- Keep-alive rows keep today's semantics: independent of the provider switch, and toggling one
  sets the matching `*_asked` flag as `apply()` does now.
- Setup hints (`HintWindows10`, `HintCenterTaskbar`, `HintWidgetsButton`) move into one
  warning-tinted card at the top, text wrapped.

## Visual tokens

Spacing on a 4/8 grid (DIP): window padding 16, caption top 14 / bottom 6, card padding 10×14,
card gap 3, row divider 1, chip gap 6, chip padding 4×11, switch 40×20.

Type: DirectWrite, `Segoe UI Variable Text` with fallback (as the strip). Title 20 semibold,
caption 12 semibold, body 13, secondary 11.5.

Colors (light / dark), close to Windows 11 Settings:

| Token | Light | Dark |
|---|---|---|
| window background | `#F3F3F3` | `#202020` |
| card | `#FBFBFB` | `#2B2B2B` |
| card border / dividers | `#E5E5E5` | `#1D1D1D` |
| text | `#1B1B1B` | `#FFFFFF` |
| secondary text | `#5F5F5F` | `#C5C5C5` |
| success | `#0F7B0F` | `#6CCB5F` |
| warning | `#9D5D00` | `#FCE100` |
| switch off border / knob | `#858585` / `#5D5D5D` | `#CFCFCF` / `#CFCFCF` |
| field background | `#FFFFFF` | `#1F1F1F` |
| accent (on switch, on chip) | user accent, fallback `#005FB8` | user accent (lighter variant), fallback `#4CC2FF` |
| text on accent | `#FFFFFF` | `#000000` |

**High contrast:** when `SPI_GETHIGHCONTRAST` reports `HCF_HIGHCONTRASTON`, ignore the table
and use system colors (`COLOR_WINDOW`, `COLOR_WINDOWTEXT`, `COLOR_HIGHLIGHT`,
`COLOR_HIGHLIGHTTEXT`, `COLOR_GRAYTEXT`, `COLOR_BTNTEXT`) with no accent and solid borders.

Record these tokens in a short `DESIGN.md` at the repo root, linked from AGENTS.md (#58 asks
for it).

## How it is built (decided)

- **Real controls, our paint.** Each switch and chip is a real `BUTTON` with
  `BS_AUTOCHECKBOX | WS_TABSTOP`, its window text set to the visible label so UI Automation
  announces name + checkbox role + state. Its look is painted by us (custom draw / subclass
  paint); the control keeps its own keyboard and click handling.
- **Background, cards, titles, captions, status lines and dividers** are painted by the window
  itself in `WM_PAINT` (plus `WM_ERASEBKGND` returning handled). They are not controls.
- **Direct2D** for painting, reusing what the strip already links (`ID2D1Factory`,
  `IDWriteFactory`, Segoe UI Variable formats), so rounded corners and text are antialiased
  at no library cost. GDI rounded shapes are not acceptable (jagged).
- **Number fields** are real `EDIT` controls with `ES_NUMBER`, no system border (we paint the
  rounded field and its bottom accent line), colors via `WM_CTLCOLOREDIT`.
- **Theme:** read `AppsUseLightTheme` (the *app* theme; the strip uses `SystemUsesLightTheme`,
  the taskbar theme — they can differ). Re-read on `WM_SETTINGCHANGE` with
  `"ImmersiveColorSet"` and repaint. Title bar via
  `DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE)`; on builds that reject it, the title
  bar simply stays light.
- **No white flash:** create hidden, set the title bar attribute, lay out, paint, then show.
- **Per-monitor DPI:** size from `GetDpiForWindow`; handle `WM_DPICHANGED` (rebuild text
  formats and edit fonts, re-run layout, use the suggested rect). Center on the strip's monitor.
- **Instant apply:** chosen because it removes the OK/Cancel buttons and the discard path
  (smaller). Every `BN_CLICKED` on a switch/chip rebuilds the config from the controls and calls
  `config_applied` (today's `apply()` minus closing). Autostart switches immediately. Number
  fields apply on `EN_KILLFOCUS`, on Enter, and when the window closes; out-of-range values are
  clamped and written back to the field. Esc and the title bar ✕ close the window.
  `IsDialogMessageW` stays: with no default button, Enter arrives as `IDOK` (treat it as
  "commit the focused field", never close) and Esc as `IDCANCEL` (apply pending field edits,
  then close).

## Pure, unit-tested pieces

- **Layout:** `(model, width_dip, scale, text measurer) → rects in px` for every card, row,
  chip, switch, field and text run. Tests: chip wrapping (few/many disks, long pt-BR labels),
  scaling at 100/125/150/200%, hint card present/absent, row dividers, total height.
- **Palette selection:** `(app_light, high_contrast, accent) → palette`. Tests for all
  combinations and the accent fallbacks.
- **Config from controls:** `(base config, toggle states, number texts) → Config` including
  `*_asked` flags and clamping. Tests mirror today's `apply()` behavior.
- **Status line:** `(provider, login state, lang) → (icon, text, tone)`.

## Strings to add (en / pt-BR)

On / Ativado · Off / Desativado · General / Geral · Hardware / Hardware · Limits / Limites ·
"Login found" (capitalized status) / "Login encontrado" · keep-alive subtitle per provider
("Experimental · runs `claude auth status` when the login expires" /
"Experimental · roda `claude auth status` quando o login expira", and the Codex equivalent) ·
range subtitles ("1 to 5 seconds" / "1 a 5 segundos", "60 to 3600 seconds" /
"60 a 3600 segundos"). Existing interval labels lose their "(1–5 s)" suffix.
Removed: OK / Cancel from Settings (keep the keys if used elsewhere).

## Verification

- Unit tests above; full `fmt` / `clippy -D warnings` / `test`.
- Release binary size measured and reported (must stay < 1,000,000 bytes).
- Manual, by the owner: Windows 11 light and dark (switch theme while open), Windows 10 if
  available, 100% and 150% scale and moving between monitors, keyboard only (Tab through every
  control, Space, Esc), Narrator reads names and states, High Contrast on, a machine with
  several disks, pt-BR and English.

## Left to the builder

- Exact custom-draw mechanism for the checkboxes (`NM_CUSTOMDRAW` returning
  `CDRF_SKIPDEFAULT` vs. a `SetWindowSubclass` paint), as long as the control stays a real
  checkbox for UIA and keyboard.
- How the accent is read (`DwmGetColorizationColor` or the documented accent registry value)
  and how its lighter dark-mode variant is derived, with the fallbacks above.
- Focus indicator drawing (2 px rounded outline in text color around the focused control).
- Module split (e.g. `src/ui/settings/{layout,paint,palette}.rs`) and test fixtures.
- Changelog wording under 0.4.0 and the README screenshot refresh.
