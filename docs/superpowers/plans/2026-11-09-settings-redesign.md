# Settings Window Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the Settings window in the style of the Windows 10/11 Settings app (title, cards, switches, chips), light/dark following the Windows app theme, applying changes instantly.

**Architecture:** `src/ui/settings.rs` stays the window (controls, messages) and gains a `settings/` folder of pure, tested modules: `palette` (theme → colors), `model` (strings, status lines, controls → `Config`), `layout` (blocks → pixel rects) and `paint` (Direct2D drawing). Switches and chips are real `BS_AUTOCHECKBOX` buttons painted by us; cards, text and dividers are painted by the window in `WM_PAINT`.

**Tech Stack:** Rust stable, `windows` 0.62 (Win32 + Direct2D + DirectWrite + DWM), no new crates.

**Spec:** `docs/superpowers/specs/2026-11-09-settings-redesign-design.md`

## Global Constraints

- Documented APIs only. No `uxtheme` ordinals, no `SetWindowTheme(…, "DarkMode_*")`.
- New `windows` features allowed (`Win32_Graphics_Dwm`); no new crates.
- Release binary < 1,000,000 bytes; baseline 920,064. If a build would cross ~985,000 bytes, stop and report to the owner.
- Every user-facing string through `i18n::t`, English and pt-BR (`every_key_has_both_languages` must pass).
- Never show `0%` for an error.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` pass after every task.
- Failing test first for every pure function (AGENTS.md).
- Commits: Conventional Commits, no AI attribution trailers (owner is sole author).
- Branch: `feat/settings-redesign` (on top of `feat/switcher-ui`). Nothing is pushed without the owner asking.

## Review Focus

1. **Rapid toggling with instant apply** — clicking chips quickly must not trigger a limits fetch per click; only changes to provider flags or the limits interval wake the worker. Test in Task 2 (`only_provider_or_interval_changes_wake_the_worker`).
2. **Half-typed or empty number fields** — `""`, `"0"`, `"99999"` commit as the previous value, the minimum and the maximum, never panic or save garbage. Test in Task 2 (`number_fields_clamp_and_fall_back`).
3. **Many disks and long pt-BR labels** — chips wrap inside the card; nothing crosses the card's right edge. Test in Task 3 (`chips_wrap_inside_the_card`).
4. **High Contrast on** — accent and fixed colors are ignored; system colors win. Test in Task 1 (`high_contrast_uses_system_colors_and_ignores_accent`).
5. **No taskbar info yet (Windows 10 hidden strip, first frame)** — window centers on the primary work area instead of (0,0). Test in Task 3 (`centers_on_strip_monitor_or_primary`).

---

### Task 1: Palette

**Files:**
- Create: `src/ui/settings/palette.rs` (declare `mod palette;` in `src/ui/settings.rs`)

**Interfaces:**
- Produces:
  - `pub struct Palette { pub bg: u32, pub card: u32, pub border: u32, pub text: u32, pub sub: u32, pub success: u32, pub warning: u32, pub off_border: u32, pub off_knob: u32, pub field: u32, pub accent: u32, pub on_accent: u32 }` (0xRRGGBB)
  - `pub struct SysColors { pub window: u32, pub window_text: u32, pub highlight: u32, pub highlight_text: u32, pub gray_text: u32 }`
  - `pub struct ThemeInputs { pub light: bool, pub high_contrast: bool, pub accent: Option<u32>, pub sys: SysColors }`
  - `pub fn palette(t: &ThemeInputs) -> Palette`
  - `pub fn read_theme() -> ThemeInputs` (impure: `AppsUseLightTheme` via `RegGetValueW`, `SPI_GETHIGHCONTRAST`, accent from `DwmGetColorizationColor`, `GetSysColor`)

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn light_and_dark_match_the_spec_table() {
    let light = palette(&inputs(true, false, None));
    assert_eq!((light.bg, light.card, light.border, light.text), (0xF3F3F3, 0xFBFBFB, 0xE5E5E5, 0x1B1B1B));
    assert_eq!((light.success, light.warning, light.accent, light.on_accent), (0x0F7B0F, 0x9D5D00, 0x005FB8, 0xFFFFFF));
    let dark = palette(&inputs(false, false, None));
    assert_eq!((dark.bg, dark.card, dark.border, dark.text), (0x202020, 0x2B2B2B, 0x1D1D1D, 0xFFFFFF));
    assert_eq!((dark.success, dark.warning, dark.accent, dark.on_accent), (0x6CCB5F, 0xFCE100, 0x4CC2FF, 0x000000));
}

#[test]
fn user_accent_is_used_and_lightened_in_dark() {
    assert_eq!(palette(&inputs(true, false, Some(0x0078D4))).accent, 0x0078D4);
    let dark = palette(&inputs(false, false, Some(0x0078D4))).accent;
    assert!(luma(dark) > luma(0x0078D4), "dark mode uses a lighter accent");
}

#[test]
fn high_contrast_uses_system_colors_and_ignores_accent() {
    let p = palette(&inputs(false, true, Some(0xFF0000)));
    let s = sys();
    assert_eq!((p.bg, p.card, p.text, p.accent, p.on_accent, p.sub), (s.window, s.window, s.window_text, s.highlight, s.highlight_text, s.gray_text));
}
```

(`inputs(light, hc, accent)` and `sys()` are test helpers with distinct fake colors; `luma` is `0.2126 r + 0.7152 g + 0.0722 b`.)

- [ ] **Step 2: Run** `cargo test --lib settings::palette` — expected: FAIL (module/functions missing).
- [ ] **Step 3: Implement `palette`** with the spec table; dark accent = user accent mixed 40% toward white; `read_theme` reads the registry value `AppsUseLightTheme` under `Software\Microsoft\Windows\CurrentVersion\Themes\Personalize` (missing → light), `SystemParametersInfoW(SPI_GETHIGHCONTRAST)` flag `HCF_HIGHCONTRASTON`, `DwmGetColorizationColor` (failure → `None`). Add `Win32_Graphics_Dwm` to `Cargo.toml` features.
- [ ] **Step 4: Run** `cargo test --lib settings::palette` — expected: 3 passed.
- [ ] **Step 5: Commit** `feat(settings): theme palette with light, dark and high contrast`

### Task 2: Model — strings, status lines, controls → Config, worker wake

**Files:**
- Create: `src/ui/settings/model.rs`
- Modify: `src/i18n.rs` (new keys; remove `Ok`, `Cancel`; edit two labels), `src/config.rs` (wake rule), `src/ui/app.rs:167-172` (`apply_config`)

**Interfaces:**
- Consumes: `Config`, `ItemId`, `Key`, `Lang`.
- Produces:
  - `pub enum Toggle { Claude, Codex, Item(ItemId), Autostart, KeepAliveClaude, KeepAliveCodex }` (moved from `settings.rs`)
  - `pub struct Applied { pub cfg: Config, pub autostart: Option<bool> }`
  - `pub fn config_from(base: &Config, toggles: &[(Toggle, bool)], hw_text: &str, limits_text: &str) -> Applied`
  - `pub enum Tone { Success, Warning }`; `pub fn status_line(found: bool, missing: Key) -> (Tone, Key)` → `(Success, LoginFoundStatus)` or `(Warning, missing)`; the painter prefixes `✓` / `⚠`.
  - `impl Config { pub fn needs_limits_wake(&self, new: &Config) -> bool }`
  - New keys (en / pt-BR, copy exactly): `SettingsPageTitle` "Settings" / "Configurações"; `SettingsGeneral` "General" / "Geral"; `SettingsHardware` "Hardware" / "Hardware"; `SettingsLimits` "Limits" / "Limites"; `SwitchOn` "On" / "Ativado"; `SwitchOff` "Off" / "Desativado"; `LoginFoundStatus` "Login found" / "Login encontrado"; `KeepAliveRow` "Keep login active" / "Manter login ativo"; `KeepAliveSubClaude` "Experimental · runs `claude auth status` when the login expires" / "Experimental · roda `claude auth status` quando o login expira"; `KeepAliveSubCodex` "Experimental · runs `codex login status` when the login expires" / "Experimental · roda `codex login status` quando o login expira"; `HwIntervalRange` "1 to 5 seconds" / "1 a 5 segundos"; `LimitsIntervalRange` "60 to 3600 seconds" / "60 a 3600 segundos". Change `SettingsHwInterval` to "Hardware refresh" / "Atualização do hardware" and `SettingsLimitsInterval` to "Limits refresh" / "Atualização dos limites". Remove `Ok`, `Cancel` (only Settings used them) and `KeepAliveSetting` if no other user remains (`grep` first).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn toggles_map_to_config_like_the_old_apply() {
    let base = Config::default();
    let a = config_from(&base, &[(Toggle::Claude, false), (Toggle::KeepAliveCodex, true), (Toggle::Item(ItemId::Gpu), false), (Toggle::Autostart, true)], "2", "300");
    assert!(!a.cfg.claude_enabled);
    assert!(a.cfg.keep_codex_login && a.cfg.codex_keepalive_asked);
    assert!(!a.cfg.is_visible(&ItemId::Gpu));
    assert_eq!((a.cfg.hw_interval_secs, a.cfg.limits_interval_secs, a.autostart), (2, 300, Some(true)));
}

#[test]
fn number_fields_clamp_and_fall_back() {
    let base = Config { hw_interval_secs: 3, limits_interval_secs: 120, ..Config::default() };
    let f = |hw: &str, lim: &str| { let c = config_from(&base, &[], hw, lim).cfg; (c.hw_interval_secs, c.limits_interval_secs) };
    assert_eq!(f("", ""), (3, 120));
    assert_eq!(f("0", "0"), (1, 60));
    assert_eq!(f("99999", "99999"), (5, 3600));
    assert_eq!(f(" 4 ", "abc"), (4, 120));
}

#[test]
fn status_line_tones() {
    assert_eq!(status_line(true, Key::ClaudeNoLogin), (Tone::Success, Key::LoginFoundStatus));
    assert_eq!(status_line(false, Key::CodexNoLogin), (Tone::Warning, Key::CodexNoLogin));
}

// in src/config.rs tests
#[test]
fn only_provider_or_interval_changes_wake_the_worker() {
    let old = Config::default();
    let mut items = old.clone(); items.set_visible(&ItemId::Cpu, false);
    assert!(!old.needs_limits_wake(&items));
    let mut hw = old.clone(); hw.hw_interval_secs = 4;
    assert!(!old.needs_limits_wake(&hw));
    let changes: [fn(&mut Config); 5] = [|c| c.claude_enabled = !c.claude_enabled, |c| c.codex_enabled = !c.codex_enabled, |c| c.limits_interval_secs = 600, |c| c.keep_claude_login = !c.keep_claude_login, |c| c.keep_codex_login = !c.keep_codex_login];
    for change in changes {
        let mut new = old.clone(); change(&mut new);
        assert!(old.needs_limits_wake(&new));
    }
}
```

- [ ] **Step 2: Run** `cargo test` — expected: FAIL to compile (missing items).
- [ ] **Step 3: Implement** `config_from` (port of today's `apply()` body; number parse = `trim().parse()`, unparsable → base value, then `clamped()`), `status_line`, `needs_limits_wake` (compares `claude_enabled`, `codex_enabled`, `limits_interval_secs`, `keep_claude_login`, `keep_codex_login`), and change `App::apply_config` to call `self.control.wake()` only when `old.needs_limits_wake(&new)`. Add/remove i18n keys including `Key::ALL`.
- [ ] **Step 4: Run** `cargo test` — expected: all pass, including `every_key_has_both_languages`.
- [ ] **Step 5: Commit** `feat(settings): settings model and wake the worker only for limit changes`

### Task 3: Layout

**Files:**
- Create: `src/ui/settings/layout.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks (strings arrive as `String`).
- Produces:
  - Tokens (DIP, `pub const`): `WIDTH = 460.0`, `PAD = 16.0`, `TITLE_GAP = 12.0`, `CAPTION_TOP = 14.0`, `CAPTION_BOTTOM = 6.0`, `CARD_PAD_X = 14.0`, `CARD_PAD_Y = 10.0`, `CARD_GAP = 3.0`, `CARD_RADIUS = 6.0`, `CHIP_GAP = 6.0`, `CHIP_PAD_X = 11.0`, `CHIP_PAD_Y = 4.0`, `SWITCH_W = 40.0`, `SWITCH_H = 20.0`, `TILE = 24.0`, `FIELD_W = 64.0`, `INDENT = 34.0`.
  - `pub enum TextRole { Title, Caption, Body, Secondary }`
  - `pub trait Measure { fn width(&self, text: &str, role: TextRole) -> f32; fn height(&self, text: &str, role: TextRole, max_width: f32) -> f32; }` (DIP)
  - `pub enum Row { Switch { id: usize, title: String, sub: Option<String>, tile: Option<(char, u32)>, indent: bool, state_text: String }, Chips { label: String, chips: Vec<(usize, String)> }, Number { id: usize, title: String, sub: String } }`
  - `pub enum Block { Hint(String), Caption(String), Card(Vec<Row>) }`
  - `pub struct Px { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }`
  - `pub struct Text { pub rect: Px, pub role: TextRole, pub text: String }`
  - `pub struct Page { pub width: i32, pub height: i32, pub cards: Vec<Px>, pub hints: Vec<Px>, pub dividers: Vec<Px>, pub tiles: Vec<(Px, char, u32)>, pub texts: Vec<Text>, pub controls: Vec<(usize, Px)> }`
  - `pub fn layout(title: &str, blocks: &[Block], scale: f32, m: &dyn Measure) -> Page`
  - `pub fn center_on(monitor: Option<Px>, primary: Px, size: (i32, i32)) -> (i32, i32)`

- [ ] **Step 1: Write the failing tests** (fake measurer: `width = chars × 7.0`, `height = 18.0 × ceil(width / max_width)`)

```rust
#[test]
fn chips_wrap_inside_the_card() {
    let chips: Vec<_> = (0..12).map(|i| (i, format!("Disco {i} (D:)"))).collect();
    let p = layout("Configurações", &[Block::Card(vec![Row::Chips { label: "Hardware".into(), chips }])], 1.0, &Fake);
    let card = &p.cards[0];
    assert!(p.controls.iter().all(|(_, r)| r.x + r.w <= card.x + card.w - 14));
    assert!(p.controls.iter().map(|(_, r)| r.y).collect::<std::collections::BTreeSet<_>>().len() > 1, "wrapped");
}

#[test]
fn scale_multiplies_every_rect() {
    let blocks = sample_blocks();
    let a = layout("T", &blocks, 1.0, &Fake);
    let b = layout("T", &blocks, 1.5, &Fake);
    assert_eq!(b.width, (460.0f32 * 1.5).round() as i32);
    for ((_, ra), (_, rb)) in a.controls.iter().zip(&b.controls) {
        assert!((rb.x - (ra.x as f32 * 1.5) as i32).abs() <= 1 && (rb.w - (ra.w as f32 * 1.5) as i32).abs() <= 1);
    }
}

#[test]
fn rows_in_a_card_get_dividers_and_switches_sit_right() {
    let p = layout("T", &sample_blocks(), 1.0, &Fake); // one card with two Switch rows
    assert_eq!(p.dividers.len(), 1);
    let (_, sw) = p.controls[0];
    assert_eq!((sw.w, sw.h), (40, 20));
    assert_eq!(sw.x + sw.w, p.cards[0].x + p.cards[0].w - 14);
}

#[test]
fn hint_block_is_optional_and_grows_with_text() {
    let without = layout("T", &sample_blocks(), 1.0, &Fake).height;
    let mut with = vec![Block::Hint("x".repeat(200))];
    with.extend(sample_blocks());
    let p = layout("T", &with, 1.0, &Fake);
    assert_eq!(p.hints.len(), 1);
    assert!(p.height > without + 36);
}

#[test]
fn centers_on_strip_monitor_or_primary() {
    let primary = Px { x: 0, y: 0, w: 1920, h: 1040 };
    assert_eq!(center_on(None, primary, (460, 600)), (730, 220));
    assert_eq!(center_on(Some(Px { x: 1920, y: 0, w: 2560, h: 1400 }), primary, (460, 600)), (1920 + 1050, 400));
}
```

- [ ] **Step 2: Run** `cargo test --lib settings::layout` — expected: FAIL.
- [ ] **Step 3: Implement `layout`**: walk blocks top-down from `PAD` after the title; a `Card` stacks rows, each row height = max(text block, control) + 2×`CARD_PAD_Y`, a 1 px divider between rows; `Switch` puts the switch at the card's right padding and the `state_text` to its left; `Chips` flows left-to-right and wraps at the card's inner width; `Number` puts a `FIELD_W` field at the right; indent shifts text by `INDENT`; tiles are `TILE` squares at the row's left (letter and color carried through). All DIP math in f32, converted with `(v * scale).round() as i32` once per rect.
- [ ] **Step 4: Run** `cargo test --lib settings::layout` — expected: 5 passed.
- [ ] **Step 5: Commit** `feat(settings): pure layout for the Settings app style page`

### Task 4: Painter

**Files:**
- Create: `src/ui/settings/paint.rs`

**Interfaces:**
- Consumes: `Palette` (Task 1), `Page`, `Px`, `TextRole` (Task 3).
- Produces:
  - `pub struct Painter` — owns `ID2D1Factory`, `IDWriteFactory`, one `IDWriteTextFormat` per `TextRole` (Segoe UI Variable Text; sizes Title 20 semibold, Caption 12 semibold, Body 13, Secondary 11.5), and an `ID2D1DCRenderTarget`.
  - `impl Painter { pub fn new(dpi: u32) -> windows::core::Result<Painter>; pub fn set_dpi(&mut self, dpi: u32) -> windows::core::Result<()>; pub fn page(&self, hdc: HDC, client: Px, page: &Page, pal: &Palette) ; pub fn switch(&self, hdc: HDC, r: Px, on: bool, focused: bool, pal: &Palette); pub fn chip(&self, hdc: HDC, r: Px, text: &str, on: bool, focused: bool, pal: &Palette); pub fn field(&self, hdc: HDC, r: Px, focused: bool, pal: &Palette) }`
  - `impl Measure for Painter` (DirectWrite text layout metrics, DIP)
  - `pub fn knob(r: Px, on: bool) -> (f32, f32, f32)` — knob center x, y and radius: on = right side radius 6, off = left side radius 5, inset 4 (DIP scaled by `r.h / 20`).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn knob_moves_and_grows_when_on() {
    let r = Px { x: 0, y: 0, w: 40, h: 20 };
    assert_eq!(knob(r, true), (30.0, 10.0, 6.0));
    assert_eq!(knob(r, false), (9.0, 10.0, 5.0));
    assert_eq!(knob(Px { x: 0, y: 0, w: 60, h: 30 }, true), (45.0, 15.0, 9.0));
}
```

- [ ] **Step 2: Run** `cargo test --lib settings::paint` — expected: FAIL.
- [ ] **Step 3: Implement** `knob`, then `Painter`: `page` fills `bg`, draws hint cards (warning tint = `warning` at 12% alpha over `card`, `⚠` prefix), cards (rounded `CARD_RADIUS`, 1 px `border`), dividers, tiles (rounded 6, monogram text in white), and texts with role colors (`text` / `sub`; status texts arrive already prefixed by the window). `switch`: on = filled `accent` pill + `on_accent` knob; off = 1 px `off_border` pill + `off_knob` knob; focus = 2 px `text` rounded outline 2 DIP outside. `chip`: on = `accent` fill + `on_accent` text; off = `field` fill + `border` + `text`. `field`: `field` fill, `border`, bottom 1 px line in `accent` when focused else `off_border`. Bind the DC target per call (`BindDC`), antialiased.
- [ ] **Step 4: Run** `cargo test --lib settings::paint` — expected: 1 passed; `cargo clippy --all-targets -- -D warnings` clean.
- [ ] **Step 5: Commit** `feat(settings): Direct2D painter for cards, switches, chips and fields`

### Task 5: Window — controls, theme, DPI, instant apply

**Files:**
- Modify: `src/ui/settings.rs` (rewrite `create`, `wndproc`, `State`; remove `Builder`, `ID_OK/ID_CANCEL` buttons, `COLOR_BTNFACE`, `GetDpiForSystem`)
- Modify: `src/ui/app.rs` (add `pub fn strip_monitor(&self) -> Option<taskbar::Rect>` returning `self.taskbar.map(|t| t.monitor)`)
- Modify: `Cargo.toml` only if a further feature is needed (`Win32_UI_Shell` already present for `SetWindowSubclass`)

**Interfaces:**
- Consumes: `read_theme`, `palette` (Task 1); `Toggle`, `config_from`, `status_line`, keys (Task 2); `layout`, `Block`, `Row`, `center_on` (Task 3); `Painter` (Task 4).
- Produces: unchanged public API: `pub fn open(parent: HWND)`, `pub fn is_dialog_message(msg: &MSG) -> bool`.

Behavior to implement (from the spec; no new pure logic, so no unit test in this task):
- Create the window hidden with `WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU`, `WS_EX_CONTROLPARENT`, class background `None`; `DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE, !light)` before the first show; size from `GetDpiForWindow`; position with `center_on(strip monitor, primary work area, size)`.
- Blocks: hints (from `setup_hints`) → `Block::Hint`; Caption "Subscriptions" (`SettingsProviders`); two cards (provider `Switch` with tile `('C', 0xD97757)` for Claude / `('X', 0x2F2F2F)` light, `0x4A4A4A` dark for Codex + status `sub` "✓ …"/"⚠ …"; indented keep-alive `Switch` with `KeepAliveRow` / `KeepAliveSub*`); Caption `SettingsItems`; one card with `Chips` Hardware (CPU, RAM, GPU, disks) and `Chips` Limits (Claude 5h, Claude weekly, Codex 5h, Codex weekly); Caption `SettingsGeneral`; card with autostart `Switch`, `Number` hw (`SettingsHwInterval` / `HwIntervalRange`) and `Number` limits.
- Controls: one `BUTTON` `BS_AUTOCHECKBOX | WS_TABSTOP` per switch/chip at its layout rect, window text = visible title/label, painted by us (subclass `WM_PAINT` via `SetWindowSubclass`, or `NM_CUSTOMDRAW` → `CDRF_SKIPDEFAULT`); two `EDIT` `ES_NUMBER | WS_TABSTOP` without border, inset inside the painted field, colors via `WM_CTLCOLOREDIT` (`field` / `text`), font from DirectWrite-equivalent `CreateFontW` "Segoe UI" at Body size for the DPI.
- `WM_PAINT`: `Painter::page`; `WM_ERASEBKGND` → 1.
- Instant apply: `BN_CLICKED` → `config_from(...)` → autostart enable/disable if changed → `config_applied(parent, cfg)`; also update the switch's `state_text` (On/Off) and invalidate. `EN_KILLFOCUS` on an edit → same apply, then write the clamped value back. `IDOK` (Enter) → apply and keep open; `IDCANCEL` (Esc) and `WM_CLOSE` → apply pending edits, `DestroyWindow`.
- `WM_SETTINGCHANGE` with `lParam` string `"ImmersiveColorSet"` (or any `SPI_SETHIGHCONTRAST`) → `read_theme`, recompute palette, reset the DWM attribute, invalidate all.
- `WM_DPICHANGED` → `Painter::set_dpi`, recreate edit font, re-run `layout`, move every control, `SetWindowPos` to the suggested rect.
- `WM_DESTROY` → drop `State` (fonts, painter).

- [ ] **Step 1: Implement** the window as above.
- [ ] **Step 2: Run** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` — expected: all pass.
- [ ] **Step 3: Build and measure** `cargo build --release` then `stat -c %s target/release/claude-codex-taskbar.exe` — expected: < 985,000 (else stop and report).
- [ ] **Step 4: Manual smoke (owner)** — open Settings in light and in dark, flip the Windows app theme while it is open, Tab through every control, Space toggles, Esc closes, a toggle takes effect on the strip immediately.
- [ ] **Step 5: Commit** `feat(settings): Settings app style window with live light/dark and instant apply`

### Task 6: Docs and release notes

**Files:**
- Create: `DESIGN.md` (repo root)
- Modify: `AGENTS.md` (Layout line for `src/ui/settings/`; link to `DESIGN.md` under Rules), `CHANGELOG.md` (on `release/v0.4.0` later; here add the entry under 0.4.0 "Changed"), `README.md` (Usage/Settings wording: changes apply immediately; light/dark)

**Interfaces:** none.

- [ ] **Step 1: Write `DESIGN.md`**: spacing grid (4/8 DIP and the Task 3 tokens), type ramp (Task 4 sizes), the color table and High Contrast rule from the spec, "real controls, our paint", don'ts (undocumented dark-mode APIs, hard-coded strings, `0%` for errors, new crates).
- [ ] **Step 2: Update** AGENTS.md, README and the CHANGELOG 0.4.0 entry ("Settings window redesigned in the style of the Windows Settings app: cards, switches, light and dark following Windows, changes apply immediately").
- [ ] **Step 3: Run** `cargo test` (doc changes only; sanity) — expected: pass.
- [ ] **Step 4: Commit** `docs: DESIGN.md and notes for the Settings redesign`
